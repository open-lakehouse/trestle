//! A client of the secret discovery service (SDS): named secrets fetched from a secret server
//! over a Unix socket, and watched for rotation.
//!
//! The server is `olai-secret-server`, or anything else that serves Envoy's v3
//! `SecretDiscoveryService` with `generic_secret` resources. Each consumer reaches it on a
//! socket of its own, and the socket is its identity: the server only answers it the names it
//! may see.
//!
//! [`SdsCredentialProvider`] fetches its secret with `FetchSecrets` on first use, then keeps a
//! `StreamSecrets` watch open in the background. Each new version the server sends replaces
//! the cached value, and is acknowledged with its nonce. When the server goes away (it is
//! recreated to rotate a value, say), the provider keeps the last value and reconnects with
//! backoff, so callers never see the gap.
//!
//! ```no_run
//! # async fn example() -> olai_http::Result<()> {
//! use olai_http::CredentialProvider;
//! use olai_http::sds::SdsCredentialProvider;
//!
//! let kek = SdsCredentialProvider::new("/run/trestle-sds/sds.sock", "server-kek");
//! let secret = kek.get_credential().await?;
//! let bytes: &[u8] = secret.expose_bytes();
//! # let _ = bytes;
//! # Ok(())
//! # }
//! ```
//!
//! For a bearer token, use [`BearerAuth::sds`](crate::bearer::BearerAuth::sds).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use envoy_types::pb::envoy::config::core::v3::data_source::Specifier;
use envoy_types::pb::envoy::extensions::transport_sockets::tls::v3::{Secret, secret};
use envoy_types::pb::envoy::service::discovery::v3::{DiscoveryRequest, DiscoveryResponse};
use envoy_types::pb::envoy::service::secret::v3::secret_discovery_service_client::SecretDiscoveryServiceClient;
use envoy_types::pb::google::rpc::Status as RpcStatus;
use futures::StreamExt;
use hyper_util::rt::TokioIo;
use prost::{Message, Name};
use tokio::net::UnixStream;
use tokio::runtime::Handle;
use tokio::sync::{OnceCell, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Code;
use tonic::transport::{Channel, Endpoint};

use crate::backoff::{Backoff, BackoffConfig};
use crate::error::generic;
use crate::{CredentialProvider, Error, Result};

/// How long the first fetch keeps trying to reach the server before it gives up.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// gRPC's `INVALID_ARGUMENT`, sent in a NACK's `error_detail`.
const INVALID_ARGUMENT: i32 = 3;

/// One version of a secret fetched over SDS.
///
/// The value never leaves through formatting: `Debug` renders `<redacted>`, and there is no
/// `Display`. Read it with [`expose_bytes`](Self::expose_bytes) or
/// [`expose_str`](Self::expose_str).
#[derive(Clone, PartialEq, Eq)]
pub struct SdsSecret {
    name: String,
    version: String,
    value: Vec<u8>,
}

impl SdsSecret {
    /// The secret's name, as the consumer asks for it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The server's version of the value. A new value comes with a new version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The value's bytes. This is secret material.
    pub fn expose_bytes(&self) -> &[u8] {
        &self.value
    }

    /// The value as UTF-8. This is secret material.
    pub fn expose_str(&self) -> Result<&str> {
        std::str::from_utf8(&self.value)
            .map_err(|_| generic(format!("SDS secret `{}` is not UTF-8", self.name)))
    }
}

impl fmt::Debug for SdsSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SdsSecret")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// A [`CredentialProvider`] of one named secret from a secret server's Unix socket.
///
/// Construction does no I/O. The first [`get_credential`](CredentialProvider::get_credential)
/// fetches the secret, retrying for up to
/// [`with_connect_timeout`](Self::with_connect_timeout) while the server is unreachable, and
/// starts a background watch that swaps in each rotated value. Later calls return the cached
/// value without I/O, including while the server is down. The watch stops when the provider
/// is dropped.
///
/// A name the server doesn't offer this consumer fails with [`Error::NotFound`].
pub struct SdsCredentialProvider {
    config: Arc<WatchConfig>,
    runtime: Option<Handle>,
    current: OnceCell<watch::Receiver<Arc<SdsSecret>>>,
}

#[derive(Debug)]
struct WatchConfig {
    socket: PathBuf,
    name: String,
    backoff: BackoffConfig,
    connect_timeout: Duration,
}

impl SdsCredentialProvider {
    /// A provider of the secret `name` from the server listening on `socket`.
    pub fn new(socket: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            config: Arc::new(WatchConfig {
                socket: socket.into(),
                name: name.into(),
                backoff: BackoffConfig::default(),
                connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            }),
            runtime: None,
            current: OnceCell::new(),
        }
    }

    /// Set the backoff between reconnects (and between first-fetch attempts).
    pub fn with_backoff(mut self, backoff: BackoffConfig) -> Self {
        self.config_mut().backoff = backoff;
        self
    }

    /// Set how long the first fetch keeps retrying an unreachable server (30 s by default).
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.config_mut().connect_timeout = timeout;
        self
    }

    /// Run the background watch on `handle` instead of the runtime of the first
    /// [`get_credential`](CredentialProvider::get_credential).
    pub fn with_runtime(mut self, handle: Handle) -> Self {
        self.runtime = Some(handle);
        self
    }

    /// The socket the provider connects to.
    pub fn socket(&self) -> &Path {
        &self.config.socket
    }

    /// The secret's name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    fn config_mut(&mut self) -> &mut WatchConfig {
        Arc::get_mut(&mut self.config).expect("configured before first use")
    }

    async fn start(&self) -> Result<watch::Receiver<Arc<SdsSecret>>> {
        let first = self.config.fetch_with_retry().await?;
        let (tx, rx) = watch::channel(Arc::new(first));
        let config = Arc::clone(&self.config);
        let task = async move { config.watch(tx).await };
        match &self.runtime {
            Some(handle) => drop(handle.spawn(task)),
            None => drop(tokio::spawn(task)),
        }
        Ok(rx)
    }
}

impl fmt::Debug for SdsCredentialProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let version = self.current.get().map(|rx| rx.borrow().version.clone());
        f.debug_struct("SdsCredentialProvider")
            .field("socket", &self.config.socket)
            .field("name", &self.config.name)
            .field("version", &version)
            .finish()
    }
}

#[async_trait]
impl CredentialProvider for SdsCredentialProvider {
    type Credential = SdsSecret;

    async fn get_credential(&self) -> Result<Arc<SdsSecret>> {
        let rx = self.current.get_or_try_init(|| self.start()).await?;
        Ok(Arc::clone(&rx.borrow()))
    }
}

/// Why one attempt to talk to the server failed.
enum Failure {
    /// The server could not be reached, or the connection broke: worth retrying.
    Unreachable(String),
    /// The server answered, and the answer is final.
    Fatal(Error),
}

impl WatchConfig {
    /// `FetchSecrets`, retried with backoff while the server is unreachable.
    async fn fetch_with_retry(&self) -> Result<SdsSecret> {
        let deadline = tokio::time::Instant::now() + self.connect_timeout;
        let mut backoff = Backoff::new(&self.backoff);
        loop {
            match self.fetch().await {
                Ok(secret) => return Ok(secret),
                Err(Failure::Fatal(e)) => return Err(e),
                Err(Failure::Unreachable(reason)) => {
                    let wait = backoff.next();
                    if tokio::time::Instant::now() + wait > deadline {
                        return Err(generic(format!(
                            "cannot fetch SDS secret `{}` from {}: {reason}",
                            self.name,
                            self.socket.display()
                        )));
                    }
                    tracing::debug!(
                        socket = %self.socket.display(),
                        name = %self.name,
                        %reason,
                        "secret server unreachable; retrying",
                    );
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }

    async fn fetch(&self) -> Result<SdsSecret, Failure> {
        let mut client = self.client().await?;
        let response = client
            .fetch_secrets(fetch_request(&self.name))
            .await
            .map_err(|status| self.status_failure(status))?
            .into_inner();
        secret_from(&response, &self.name).map_err(|reason| {
            Failure::Fatal(generic(format!(
                "secret server at {} sent an unusable `{}`: {reason}",
                self.socket.display(),
                self.name
            )))
        })
    }

    async fn client(&self) -> Result<SecretDiscoveryServiceClient<Channel>, Failure> {
        connect(&self.socket)
            .await
            .map(SecretDiscoveryServiceClient::new)
            .map_err(|e| Failure::Unreachable(e.to_string()))
    }

    fn status_failure(&self, status: tonic::Status) -> Failure {
        let (socket, name) = (self.socket.display(), &self.name);
        match status.code() {
            Code::Unavailable | Code::Unknown | Code::Cancelled => {
                Failure::Unreachable(status.message().to_string())
            }
            Code::NotFound => Failure::Fatal(Error::NotFound {
                source: format!("SDS secret `{name}` is not offered on {socket}").into(),
            }),
            Code::Unauthenticated | Code::PermissionDenied => {
                Failure::Fatal(Error::PermissionDenied {
                    source: format!("{socket} refused SDS secret `{name}`: {}", status.message())
                        .into(),
                })
            }
            code => Failure::Fatal(generic(format!(
                "{socket} failed SDS secret `{name}`: {code:?}: {}",
                status.message()
            ))),
        }
    }

    /// Watches the secret until every receiver is gone: one `StreamSecrets` stream at a time,
    /// reconnecting with backoff whenever a stream ends. `tx` always holds the last good value.
    async fn watch(&self, tx: watch::Sender<Arc<SdsSecret>>) {
        let mut backoff = Backoff::new(&self.backoff);
        loop {
            let mut healthy = false;
            let ended = tokio::select! {
                () = tx.closed() => return,
                ended = self.stream(&tx, &mut healthy) => ended,
            };
            if healthy {
                backoff = Backoff::new(&self.backoff);
            }
            tracing::debug!(
                socket = %self.socket.display(),
                name = %self.name,
                reason = %ended,
                "secret stream ended; keeping the last value and reconnecting",
            );
            tokio::select! {
                () = tx.closed() => return,
                () = tokio::time::sleep(backoff.next()) => {}
            }
        }
    }

    /// One `StreamSecrets` stream, until it ends. Returns why it ended. Sets `healthy` once the
    /// server has sent a usable version.
    async fn stream(&self, tx: &watch::Sender<Arc<SdsSecret>>, healthy: &mut bool) -> String {
        let mut client = match self.client().await {
            Ok(client) => client,
            Err(Failure::Unreachable(reason)) => return reason,
            Err(Failure::Fatal(e)) => return e.to_string(),
        };
        let (requests, outbound) = mpsc::channel(4);
        let current = tx.borrow().version.clone();
        // The channel has room: the first send never waits.
        let _ = requests
            .send(stream_request(&self.name, &current, "", None))
            .await;
        let mut inbound = match client.stream_secrets(ReceiverStream::new(outbound)).await {
            Ok(response) => response.into_inner(),
            Err(status) => return status.to_string(),
        };
        while let Some(message) = inbound.next().await {
            let response = match message {
                Ok(response) => response,
                Err(status) => return status.to_string(),
            };
            let ack = match secret_from(&response, &self.name) {
                Ok(secret) => {
                    *healthy = true;
                    if secret != **tx.borrow() {
                        tracing::info!(
                            name = %self.name,
                            version = %secret.version,
                            "received a new version of the secret",
                        );
                        tx.send_replace(Arc::new(secret));
                    }
                    stream_request(&self.name, &response.version_info, &response.nonce, None)
                }
                Err(reason) => {
                    tracing::warn!(
                        name = %self.name,
                        version = %response.version_info,
                        %reason,
                        "rejecting an unusable secret version (NACK)",
                    );
                    let current = tx.borrow().version.clone();
                    stream_request(&self.name, &current, &response.nonce, Some(reason))
                }
            };
            if requests.send(ack).await.is_err() {
                return "the request stream closed".into();
            }
        }
        "the server closed the stream".into()
    }
}

/// Connects a tonic channel to the Unix socket at `path`.
async fn connect(path: &Path) -> Result<Channel, tonic::transport::Error> {
    let path = path.to_path_buf();
    // The URI is required but unused: the connector ignores it and dials the socket.
    Endpoint::from_static("http://sds.invalid")
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        }))
        .await
}

fn fetch_request(name: &str) -> DiscoveryRequest {
    DiscoveryRequest {
        resource_names: vec![name.to_string()],
        type_url: Secret::type_url(),
        ..Default::default()
    }
}

/// A `StreamSecrets` request for `name`: the initial request (empty nonce), an ACK of
/// `version` with its `nonce`, or (with an `error`) a NACK that keeps `version`, the last
/// version accepted.
fn stream_request(
    name: &str,
    version: &str,
    nonce: &str,
    error: Option<String>,
) -> DiscoveryRequest {
    DiscoveryRequest {
        version_info: version.to_string(),
        resource_names: vec![name.to_string()],
        type_url: Secret::type_url(),
        response_nonce: nonce.to_string(),
        error_detail: error.map(|message| RpcStatus {
            code: INVALID_ARGUMENT,
            message,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The `generic_secret` named `name` in `response`. The reason it is unusable names the
/// secret's shape, never its value.
fn secret_from(response: &DiscoveryResponse, name: &str) -> Result<SdsSecret, String> {
    let type_url = Secret::type_url();
    for any in &response.resources {
        if any.type_url != type_url {
            return Err(format!("unexpected resource type {}", any.type_url));
        }
        let secret = Secret::decode(any.value.as_slice())
            .map_err(|e| format!("the resource does not decode: {e}"))?;
        if secret.name != name {
            continue;
        }
        let Some(secret::Type::GenericSecret(generic)) = secret.r#type else {
            return Err("not a generic secret".into());
        };
        let value = match generic.secret.and_then(|source| source.specifier) {
            Some(Specifier::InlineBytes(bytes)) => bytes,
            Some(Specifier::InlineString(s)) => s.into_bytes(),
            Some(_) => return Err("not an inline value".into()),
            None => return Err("no value".into()),
        };
        if value.is_empty() {
            return Err("the value is empty".into());
        }
        return Ok(SdsSecret {
            name: secret.name,
            version: response.version_info.clone(),
            value,
        });
    }
    Err("the response does not hold it".into())
}

#[cfg(test)]
mod tests {
    use envoy_types::pb::envoy::config::core::v3::DataSource;
    use envoy_types::pb::envoy::extensions::transport_sockets::tls::v3::GenericSecret;
    use envoy_types::util::pack_any;

    use super::*;

    fn response(name: &str, specifier: Option<Specifier>) -> DiscoveryResponse {
        DiscoveryResponse {
            version_info: "boot-1".into(),
            resources: vec![pack_any(Secret {
                name: name.into(),
                r#type: Some(secret::Type::GenericSecret(GenericSecret {
                    secret: Some(DataSource {
                        specifier,
                        ..Default::default()
                    }),
                    ..Default::default()
                })),
            })],
            type_url: Secret::type_url(),
            nonce: "1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn requests_ack_and_nack() {
        let initial = stream_request("kek", "", "", None);
        assert_eq!(initial.resource_names, ["kek"]);
        assert_eq!(initial.type_url, Secret::type_url());
        assert!(initial.response_nonce.is_empty() && initial.error_detail.is_none());

        let ack = stream_request("kek", "boot-2", "2", None);
        assert_eq!(
            (ack.version_info.as_str(), ack.response_nonce.as_str()),
            ("boot-2", "2")
        );
        assert!(ack.error_detail.is_none());

        let nack = stream_request("kek", "boot-1", "2", Some("bad".into()));
        assert_eq!(nack.version_info, "boot-1");
        assert_eq!(nack.response_nonce, "2");
        assert_eq!(nack.error_detail.unwrap().code, INVALID_ARGUMENT);
    }

    #[test]
    fn reads_inline_bytes_and_strings() {
        let bytes = secret_from(
            &response("kek", Some(Specifier::InlineBytes(b"v".to_vec()))),
            "kek",
        )
        .unwrap();
        assert_eq!(
            (bytes.expose_bytes(), bytes.version()),
            (&b"v"[..], "boot-1")
        );
        let string = secret_from(
            &response("kek", Some(Specifier::InlineString("s".into()))),
            "kek",
        )
        .unwrap();
        assert_eq!(string.expose_str().unwrap(), "s");
    }

    #[test]
    fn rejects_unusable_resources() {
        let other = response("other", Some(Specifier::InlineBytes(b"v".to_vec())));
        assert!(secret_from(&other, "kek").is_err());
        assert!(secret_from(&response("kek", None), "kek").is_err());
        let file = response("kek", Some(Specifier::Filename("/etc/k".into())));
        assert!(secret_from(&file, "kek").is_err());
        let empty = response("kek", Some(Specifier::InlineBytes(vec![])));
        assert!(secret_from(&empty, "kek").is_err());
    }

    #[test]
    fn debug_redacts_the_value() {
        let secret = secret_from(
            &response(
                "kek",
                Some(Specifier::InlineBytes(b"hunter2-secret".to_vec())),
            ),
            "kek",
        )
        .unwrap();
        let rendered = format!("{secret:?} {:#?}", Some(&secret));
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }
}
