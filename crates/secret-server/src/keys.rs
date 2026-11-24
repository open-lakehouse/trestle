//! The key service: `olai.keys.v1.KeyService` (`Wrap`, `Unwrap`, `GetKey`, `WatchKey`).
//!
//! A consumer wraps and unwraps its data keys under the keys its listener lists, and never
//! sees a key encryption key. A key another consumer holds and a key nobody holds both answer
//! `NOT_FOUND`.
//!
//! Nothing here logs a request or a response: `WrapRequest` and `UnwrapResponse` carry
//! plaintext DEKs. Logs carry consumers, keys and versions only.

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use olai_key_service::v1::key_service_server::{KeyService as KeyServiceApi, KeyServiceServer};
use olai_key_service::v1::{
    GetKeyRequest, KeyInfo, UnwrapRequest, UnwrapResponse, WatchKeyRequest, WrapRequest,
    WrapResponse,
};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::caller::{CallerId, CallerResolver, SocketCaller};
use crate::keyring::{KeyError, KeyHandle};
use crate::policy::Policy;

type InfoStream = Pin<Box<dyn Stream<Item = Result<KeyInfo, Status>> + Send + 'static>>;

/// The key service over a [`Policy`]'s keys.
///
/// Each connection's caller comes from the resolver `R` ([`SocketCaller`] by default); the
/// policy decides which keys that caller may use.
#[derive(Debug)]
pub struct KeyService<R = SocketCaller> {
    policy: Arc<Policy>,
    resolver: Arc<R>,
    closing: Arc<watch::Sender<bool>>,
}

impl<R> Clone for KeyService<R> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            resolver: self.resolver.clone(),
            closing: self.closing.clone(),
        }
    }
}

impl KeyService<SocketCaller> {
    /// A service that identifies callers by the socket they connected on.
    pub fn new(policy: Arc<Policy>) -> Self {
        Self::with_resolver(policy, SocketCaller)
    }
}

impl<R: CallerResolver> KeyService<R> {
    /// A service that identifies callers with `resolver`.
    pub fn with_resolver(policy: Arc<Policy>, resolver: R) -> Self {
        Self {
            policy,
            resolver: Arc::new(resolver),
            closing: Arc::new(watch::channel(false).0),
        }
    }

    /// Ends every open `WatchKey` stream, and every one opened later, with `UNAVAILABLE`, so a
    /// graceful shutdown doesn't wait on them. Clients reconnect to the next server and keep
    /// the versions they last saw meanwhile.
    pub fn close_streams(&self) {
        self.closing.send_replace(true);
    }

    /// The tonic service to add to a server.
    pub fn into_server(self) -> KeyServiceServer<Self> {
        KeyServiceServer::new(self)
    }

    /// The caller of `request`, and the keyring of `key` if the caller may use it.
    fn key<T>(&self, request: &Request<T>, key: &str) -> Result<(CallerId, KeyHandle), Status> {
        let caller = self
            .resolver
            .resolve(request.extensions())
            .ok_or_else(|| Status::unauthenticated("connection has no caller identity"))?;
        match self.policy.key(&caller, key) {
            Some(handle) => Ok((caller, handle.clone())),
            None => {
                tracing::warn!(
                    consumer = caller.as_str(),
                    key,
                    "consumer asked for a key it may not use"
                );
                Err(Status::not_found(format!("key {key} not found")))
            }
        }
    }
}

fn status(caller: &CallerId, key: &str, op: &str, error: KeyError) -> Status {
    tracing::warn!(consumer = caller.as_str(), key, op, reason = %error, "refused");
    match error {
        KeyError::NotFound(message) => Status::not_found(message),
        KeyError::FailedPrecondition(message) => Status::failed_precondition(message),
        KeyError::InvalidArgument(message) => Status::invalid_argument(message),
    }
}

#[tonic::async_trait]
impl<R: CallerResolver> KeyServiceApi for KeyService<R> {
    type WatchKeyStream = InfoStream;

    async fn wrap(&self, request: Request<WrapRequest>) -> Result<Response<WrapResponse>, Status> {
        let (caller, handle) = self.key(&request, &request.get_ref().key)?;
        let request = request.get_ref();
        let (version, wrapped) = handle
            .current()
            .wrap(&request.dek, request.version, &request.alias)
            .await
            .map_err(|e| status(&caller, &request.key, "wrap", e))?;
        tracing::debug!(consumer = caller.as_str(), key = %request.key, version, "wrapped a DEK");
        Ok(Response::new(WrapResponse { version, wrapped }))
    }

    async fn unwrap(
        &self,
        request: Request<UnwrapRequest>,
    ) -> Result<Response<UnwrapResponse>, Status> {
        let (caller, handle) = self.key(&request, &request.get_ref().key)?;
        let request = request.get_ref();
        let mut dek = handle
            .current()
            .unwrap(&request.wrapped, request.version, &request.alias)
            .await
            .map_err(|e| status(&caller, &request.key, "unwrap", e))?;
        tracing::debug!(
            consumer = caller.as_str(),
            key = %request.key,
            version = request.version,
            alias = %request.alias,
            "unwrapped a DEK",
        );
        Ok(Response::new(UnwrapResponse {
            dek: std::mem::take(&mut *dek),
        }))
    }

    async fn get_key(&self, request: Request<GetKeyRequest>) -> Result<Response<KeyInfo>, Status> {
        let (_, handle) = self.key(&request, &request.get_ref().key)?;
        Ok(Response::new(handle.current().info()))
    }

    async fn watch_key(
        &self,
        request: Request<WatchKeyRequest>,
    ) -> Result<Response<Self::WatchKeyStream>, Status> {
        let (caller, handle) = self.key(&request, &request.get_ref().key)?;
        let key = request.into_inner().key;
        let mut keyring = handle.subscribe();
        let mut closing = self.closing.subscribe();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tracing::debug!(consumer = caller.as_str(), key = %key, "key watch opened");
            loop {
                let info = keyring.borrow_and_update().info();
                if tx.send(Ok(info)).await.is_err() {
                    break;
                }
                // The borrow `wait_for` returns is dropped inside, so it never crosses an await.
                let closed = async {
                    let _ = closing.wait_for(|closed| *closed).await;
                };
                tokio::select! {
                    () = closed => {
                        let _ = tx.send(Err(Status::unavailable("the secret server is shutting down"))).await;
                        break;
                    }
                    () = tx.closed() => break,
                    changed = keyring.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                }
            }
            tracing::debug!(consumer = caller.as_str(), key = %key, "key watch closed");
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}
