//! A fake host endpoint and a secret server on the host source, shared by the host source
//! tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use envoy_types::pb::envoy::config::core::v3::data_source::Specifier;
use envoy_types::pb::envoy::extensions::transport_sockets::tls::v3::{Secret, secret};
use envoy_types::pb::envoy::service::discovery::v3::{DiscoveryRequest, DiscoveryResponse};
use envoy_types::pb::envoy::service::secret::v3::secret_discovery_service_client::SecretDiscoveryServiceClient;
use hyper_util::rt::TokioIo;
use olai_secret_server::sds::secret_type_url;
use olai_secret_server::{Config, ConfigError, HostOptions, WatchGuard, resolve, serve};
use prost::Message;
use tokio::net::{TcpListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};

pub const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// The fake endpoint: values by variable, each with a version, behind one path token.
#[derive(Clone)]
pub struct FakeHost {
    values: Arc<Mutex<HashMap<String, (String, u64)>>>,
    changed: Arc<watch::Sender<u64>>,
    down: Arc<AtomicBool>,
}

/// Takes a fake host down: held polls end, and the listener closes.
pub struct Stop {
    host: FakeHost,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Stop {
    pub async fn stop(self) {
        self.host.down.store(true, Ordering::SeqCst);
        self.host.changed.send_modify(|n| *n += 1);
        let _ = self.stop.send(());
        self.task.await.unwrap();
        self.host.down.store(false, Ordering::SeqCst);
    }
}

#[derive(serde::Deserialize)]
struct Wait {
    wait: Option<u64>,
}

impl FakeHost {
    pub fn new(values: &[(&str, &str)]) -> Self {
        Self {
            values: Arc::new(Mutex::new(
                values
                    .iter()
                    .map(|(k, v)| (k.to_string(), (v.to_string(), 1)))
                    .collect(),
            )),
            changed: Arc::new(watch::channel(0).0),
            down: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set(&self, variable: &str, value: &str) {
        let mut values = self.values.lock().unwrap();
        let entry = values
            .entry(variable.to_string())
            .or_insert((String::new(), 0));
        *entry = (value.to_string(), entry.1 + 1);
        self.changed.send_modify(|n| *n += 1);
    }

    fn current(&self, variable: &str) -> Option<(String, u64)> {
        self.values.lock().unwrap().get(variable).cloned()
    }

    /// Serves on `addr` (port 0 for any) until the returned sender is dropped or fired.
    pub async fn serve(&self, addr: SocketAddr) -> (SocketAddr, Stop) {
        let listener = TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/{token}/secrets/{variable}", get(handle))
            .with_state(self.clone());
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let stop = Stop {
            host: self.clone(),
            stop,
            task,
        };
        (addr, stop)
    }
}

async fn handle(
    State(host): State<FakeHost>,
    UrlPath((token, variable)): UrlPath<(String, String)>,
    Query(wait): Query<Wait>,
    headers: HeaderMap,
) -> Response {
    if token != TOKEN {
        return StatusCode::NOT_FOUND.into_response();
    }
    let seen = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut changed = host.changed.subscribe();
    let hold = tokio::time::sleep(Duration::from_secs(wait.wait.unwrap_or(0)));
    tokio::pin!(hold);
    loop {
        if host.down.load(Ordering::SeqCst) {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let Some((value, version)) = host.current(&variable) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let etag = format!("\"{version}\"");
        if seen.as_deref() != Some(etag.as_str()) {
            let mut response = value.into_response();
            response
                .headers_mut()
                .insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
            return response;
        }
        tokio::select! {
            () = &mut hold => return StatusCode::NOT_MODIFIED.into_response(),
            _ = changed.changed() => {}
        }
    }
}

pub fn options() -> HostOptions {
    HostOptions {
        wait: Duration::from_secs(2),
        startup_timeout: Duration::from_millis(800),
        max_backoff: Duration::from_millis(200),
    }
}

pub fn yaml(dir: &Path, addr: SocketAddr) -> String {
    format!(
        "version: 1\nsource:\n  kind: host\n  url: http://{addr}/\n  token_env: HOST_TOKEN\nlisteners:\n  - socket: {a}\n    consumer: a\n    secrets:\n      - {{name: planner, env: PLANNER}}\n  - socket: {b}\n    consumer: b\n    secrets:\n      - {{name: other, env: OTHER}}\n",
        a = dir.join("a/sds.sock").display(),
        b = dir.join("b/sds.sock").display(),
    )
}

pub fn token_only(name: &str) -> Option<std::ffi::OsString> {
    (name == "HOST_TOKEN").then(|| TOKEN.into())
}

/// A running server on the host source.
pub struct Harness {
    dir: tempfile::TempDir,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
    _watching: WatchGuard,
}

impl Harness {
    pub async fn start(addr: SocketAddr) -> Result<Self, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_yaml(&yaml(dir.path(), addr)).unwrap();
        let resolved = resolve(&config, token_only, options()).await?;
        let watching = resolved.watcher.expect("the host source watches").spawn();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            serve(&config, resolved.policy, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        });
        let harness = Self {
            dir,
            stop: Some(stop),
            task,
            _watching: watching,
        };
        for _ in 0..100 {
            if UnixStream::connect(harness.socket("a")).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(harness)
    }

    fn socket(&self, consumer: &str) -> PathBuf {
        self.dir.path().join(consumer).join("sds.sock")
    }

    pub async fn client(&self, consumer: &str) -> SecretDiscoveryServiceClient<Channel> {
        let path = self.socket(consumer);
        let channel =
            Endpoint::try_from("http://sds.invalid")
                .unwrap()
                .connect_with_connector(tower::service_fn(move |_| {
                    let path = path.clone();
                    async move {
                        Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?))
                    }
                }))
                .await
                .unwrap();
        SecretDiscoveryServiceClient::new(channel)
    }

    pub async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.await.unwrap();
    }
}

pub fn request(names: &[&str], version: &str, nonce: &str) -> DiscoveryRequest {
    DiscoveryRequest {
        version_info: version.into(),
        resource_names: names.iter().map(|n| n.to_string()).collect(),
        type_url: secret_type_url(),
        response_nonce: nonce.into(),
        ..Default::default()
    }
}

pub fn values(response: &DiscoveryResponse) -> Vec<String> {
    response
        .resources
        .iter()
        .map(|any| {
            let secret = Secret::decode(any.value.as_slice()).unwrap();
            let Some(secret::Type::GenericSecret(generic)) = secret.r#type else {
                panic!("not a generic secret");
            };
            let Some(Specifier::InlineBytes(bytes)) = generic.secret.unwrap().specifier else {
                panic!("not inline bytes");
            };
            String::from_utf8(bytes).unwrap()
        })
        .collect()
}

pub async fn next_within(
    responses: &mut tonic::Streaming<DiscoveryResponse>,
    wait: Duration,
) -> Option<DiscoveryResponse> {
    match tokio::time::timeout(wait, responses.message()).await {
        Ok(message) => message.unwrap(),
        Err(_) => None,
    }
}

pub async fn fetch(client: &mut SecretDiscoveryServiceClient<Channel>, name: &str) -> String {
    let response = client
        .fetch_secrets(request(&[name], "", ""))
        .await
        .unwrap()
        .into_inner();
    values(&response).remove(0)
}

/// Opens a stream for `name`, checks the first value, and ACKs it.
pub async fn subscribe(
    client: &mut SecretDiscoveryServiceClient<Channel>,
    name: &str,
    expected: &str,
) -> (
    mpsc::Sender<DiscoveryRequest>,
    tonic::Streaming<DiscoveryResponse>,
) {
    let (tx, rx) = mpsc::channel(8);
    let mut responses = client
        .stream_secrets(ReceiverStream::new(rx))
        .await
        .unwrap()
        .into_inner();
    tx.send(request(&[name], "", "")).await.unwrap();
    let first = next_within(&mut responses, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(values(&first), [expected]);
    tx.send(request(&[name], &first.version_info, &first.nonce))
        .await
        .unwrap();
    (tx, responses)
}
