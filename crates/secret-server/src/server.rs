//! Binding the per-consumer Unix sockets and serving SDS and the key service on them.

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use futures::future::{BoxFuture, FutureExt, Shared};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;

use crate::caller::{CallerId, ConsumerConn};
use crate::config::Config;
use crate::keys::KeyService;
use crate::policy::Policy;
use crate::sds::SdsService;

/// Why the server could not start or stopped serving.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// A socket could not be prepared or bound.
    #[error("cannot bind {path}: {source}")]
    Bind {
        /// The socket path.
        path: PathBuf,
        /// The I/O error.
        source: std::io::Error,
    },
    /// A listener failed while serving.
    #[error("listener for consumer {consumer} failed: {source}")]
    Serve {
        /// The consumer of the listener.
        consumer: String,
        /// The transport error.
        source: tonic::transport::Error,
    },
}

/// Binds `path` for a consumer: creates the parent directory, removes a socket left by an
/// earlier run, binds, and opens the socket to every local user.
///
/// The permissions are wide on purpose: a consumer's identity is the volume only it mounts,
/// not the socket's owner, and consumers such as Envoy run as a different user.
pub fn bind_socket(path: &Path) -> Result<UnixListener, ServeError> {
    let bind_err = |source| ServeError::Bind {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(bind_err)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(bind_err(e)),
    }
    let listener = UnixListener::bind(path).map_err(bind_err)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).map_err(bind_err)?;
    Ok(listener)
}

/// Serves SDS and the key service for one consumer on an already bound listener until
/// `shutdown` resolves.
pub async fn serve_listener(
    listener: UnixListener,
    consumer: CallerId,
    sds: SdsService,
    keys: KeyService,
    shutdown: impl Future<Output = ()>,
) -> Result<(), tonic::transport::Error> {
    let incoming = UnixListenerStream::new(listener)
        .map(move |conn| conn.map(|io| ConsumerConn::new(io, consumer.clone())));
    tonic::transport::Server::builder()
        .add_service(sds.into_server())
        .add_service(keys.into_server())
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await
}

/// Binds every listener in `config` and serves them until `shutdown` resolves or one fails.
///
/// All sockets are bound before any is served, so a bad path fails startup as a whole.
pub async fn serve(
    config: &Config,
    policy: Policy,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let policy = Arc::new(policy);
    let sds = SdsService::new(policy.clone());
    let keys = KeyService::new(policy);
    let shutdown: Shared<BoxFuture<'static, ()>> = shutdown.boxed().shared();
    let mut bound = Vec::with_capacity(config.listeners.len());
    for listener in &config.listeners {
        bound.push((listener, bind_socket(&listener.socket)?));
    }
    let mut tasks = tokio::task::JoinSet::new();
    for (config, listener) in bound {
        let consumer = config.consumer.clone();
        tracing::info!(
            consumer = %consumer,
            socket = %config.socket.display(),
            secrets = config.secrets.len(),
            keys = config.keys.len(),
            "listening",
        );
        let caller = CallerId::new(&consumer);
        let (sds, keys) = (sds.clone(), keys.clone());
        // End the open streams first: a graceful shutdown waits for them, and a consumer's
        // stream stays open for as long as it runs.
        let shutdown = {
            let shutdown = shutdown.clone();
            let (sds, keys) = (sds.clone(), keys.clone());
            async move {
                shutdown.await;
                sds.close_streams();
                keys.close_streams();
            }
        };
        tasks.spawn(async move {
            serve_listener(listener, caller, sds, keys, shutdown)
                .await
                .map_err(|source| ServeError::Serve { consumer, source })
        });
    }
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(join) => std::panic::resume_unwind(join.into_panic()),
        }
    }
    Ok(())
}
