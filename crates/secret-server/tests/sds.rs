//! SDS over real Unix sockets, with a tonic client, in process (no Docker).

use std::path::{Path, PathBuf};
use std::time::Duration;

use envoy_types::pb::envoy::config::core::v3::data_source::Specifier;
use envoy_types::pb::envoy::extensions::transport_sockets::tls::v3::{Secret, secret};
use envoy_types::pb::envoy::service::discovery::v3::{DiscoveryRequest, DiscoveryResponse};
use envoy_types::pb::envoy::service::secret::v3::secret_discovery_service_client::SecretDiscoveryServiceClient;
use hyper_util::rt::TokioIo;
use olai_secret_server::sds::secret_type_url;
use olai_secret_server::{Config, serve};
use prost::Message;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};

struct Harness {
    dir: tempfile::TempDir,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let yaml = format!(
            "version: 1\nlisteners:\n  - socket: {a}\n    consumer: a\n    secrets:\n      - {{name: planner, env: PLANNER}}\n      - {{name: reviewer, env: REVIEWER}}\n  - socket: {b}\n    consumer: b\n    secrets:\n      - {{name: other, env: OTHER}}\n",
            a = dir.path().join("a/sds.sock").display(),
            b = dir.path().join("b/sds.sock").display(),
        );
        let config = Config::from_yaml(&yaml).unwrap();
        let policy = config
            .resolve(|name| match name {
                "PLANNER" => Some("planner-value".into()),
                "REVIEWER" => Some("reviewer-value".into()),
                "OTHER" => Some("other-value".into()),
                _ => None,
            })
            .unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            serve(&config, policy, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        });
        let harness = Self {
            dir,
            stop: Some(stop),
            task,
        };
        for consumer in ["a", "b"] {
            let path = harness.socket(consumer);
            for _ in 0..100 {
                if UnixStream::connect(&path).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        harness
    }

    fn socket(&self, consumer: &str) -> PathBuf {
        self.dir.path().join(consumer).join("sds.sock")
    }

    async fn client(&self, consumer: &str) -> SecretDiscoveryServiceClient<Channel> {
        SecretDiscoveryServiceClient::new(connect(&self.socket(consumer)).await)
    }

    async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.await.unwrap();
    }
}

async fn connect(path: &Path) -> Channel {
    let path = path.to_path_buf();
    Endpoint::try_from("http://sds.invalid")
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        }))
        .await
        .unwrap()
}

fn request(names: &[&str], version: &str, nonce: &str) -> DiscoveryRequest {
    DiscoveryRequest {
        version_info: version.into(),
        resource_names: names.iter().map(|n| n.to_string()).collect(),
        type_url: secret_type_url(),
        response_nonce: nonce.into(),
        ..Default::default()
    }
}

/// `(name, value)` of every resource in a response.
fn secrets(response: &DiscoveryResponse) -> Vec<(String, String)> {
    response
        .resources
        .iter()
        .map(|any| {
            assert_eq!(any.type_url, secret_type_url());
            let secret = Secret::decode(any.value.as_slice()).unwrap();
            let Some(secret::Type::GenericSecret(generic)) = secret.r#type else {
                panic!("not a generic secret");
            };
            let Some(Specifier::InlineBytes(bytes)) = generic.secret.unwrap().specifier else {
                panic!("not inline bytes");
            };
            (secret.name, String::from_utf8(bytes).unwrap())
        })
        .collect()
}

/// Opens a stream and returns its request sender and response stream.
async fn open(
    client: &mut SecretDiscoveryServiceClient<Channel>,
) -> (
    mpsc::Sender<DiscoveryRequest>,
    tonic::Streaming<DiscoveryResponse>,
) {
    let (tx, rx) = mpsc::channel(8);
    let responses = client
        .stream_secrets(ReceiverStream::new(rx))
        .await
        .unwrap()
        .into_inner();
    (tx, responses)
}

async fn next_within(
    responses: &mut tonic::Streaming<DiscoveryResponse>,
    wait: Duration,
) -> Option<DiscoveryResponse> {
    match tokio::time::timeout(wait, responses.message()).await {
        Ok(message) => message.unwrap(),
        Err(_) => None,
    }
}

const QUIET: Duration = Duration::from_millis(300);

#[tokio::test]
async fn stream_sends_own_secrets_and_follows_ack_nack() {
    let harness = Harness::start().await;
    let mut client = harness.client("a").await;
    let (tx, mut responses) = open(&mut client).await;

    tx.send(request(&["planner"], "", "")).await.unwrap();
    let first = next_within(&mut responses, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        secrets(&first),
        [("planner".into(), "planner-value".into())]
    );
    assert!(!first.version_info.is_empty());

    // ACK: nothing more.
    tx.send(request(&["planner"], &first.version_info, &first.nonce))
        .await
        .unwrap();
    assert!(next_within(&mut responses, QUIET).await.is_none());

    // ACK that adds a name: a new version with both.
    tx.send(request(
        &["planner", "reviewer"],
        &first.version_info,
        &first.nonce,
    ))
    .await
    .unwrap();
    let second = next_within(&mut responses, Duration::from_secs(5))
        .await
        .unwrap();
    assert_ne!(second.version_info, first.version_info);
    assert_eq!(secrets(&second).len(), 2);

    // NACK of that version: not re-sent.
    let mut nack = request(&["planner", "reviewer"], &first.version_info, &second.nonce);
    nack.error_detail = Some(envoy_types::pb::google::rpc::Status {
        code: 3,
        message: "rejected".into(),
        ..Default::default()
    });
    tx.send(nack).await.unwrap();
    assert!(next_within(&mut responses, QUIET).await.is_none());

    // A stale nonce is ignored even though it names something new.
    tx.send(request(&["reviewer"], &first.version_info, &first.nonce))
        .await
        .unwrap();
    assert!(next_within(&mut responses, QUIET).await.is_none());

    drop(tx);
    harness.stop().await;
}

#[tokio::test]
async fn stream_never_sends_another_consumers_secret() {
    let harness = Harness::start().await;
    let mut client = harness.client("b").await;
    let (tx, mut responses) = open(&mut client).await;

    tx.send(request(&["planner"], "", "")).await.unwrap();
    assert!(next_within(&mut responses, QUIET).await.is_none());

    // Mixed: only b's own name comes back.
    tx.send(request(&["planner", "other"], "", ""))
        .await
        .unwrap();
    let response = next_within(&mut responses, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(secrets(&response), [("other".into(), "other-value".into())]);

    drop(tx);
    harness.stop().await;
}

#[tokio::test]
async fn fetch_returns_not_found_for_unknown_and_disallowed_names_alike() {
    let harness = Harness::start().await;

    let mut a = harness.client("a").await;
    let ok = a
        .fetch_secrets(request(&["planner"], "", ""))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(secrets(&ok), [("planner".into(), "planner-value".into())]);

    let mut b = harness.client("b").await;
    let disallowed = b
        .fetch_secrets(request(&["planner"], "", ""))
        .await
        .unwrap_err();
    assert_eq!(disallowed.code(), tonic::Code::NotFound);
    let unknown = b
        .fetch_secrets(request(&["nope"], "", ""))
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), tonic::Code::NotFound);
    assert_eq!(
        disallowed.message().replace("planner", "X"),
        unknown.message().replace("nope", "X"),
    );
    // One disallowed name fails the whole fetch.
    let mixed = b
        .fetch_secrets(request(&["other", "planner"], "", ""))
        .await
        .unwrap_err();
    assert_eq!(mixed.code(), tonic::Code::NotFound);

    harness.stop().await;
}

#[tokio::test]
async fn foreign_type_urls_are_rejected() {
    let harness = Harness::start().await;
    let mut a = harness.client("a").await;
    let mut req = request(&["planner"], "", "");
    req.type_url = "type.googleapis.com/envoy.config.cluster.v3.Cluster".into();
    let err = a.fetch_secrets(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    harness.stop().await;
}

#[tokio::test]
async fn binding_replaces_a_stale_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/sds.sock");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // A socket left behind by an earlier run: bound, then dropped without unlinking.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let listener = olai_secret_server::bind_socket(&path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o666);
    drop(listener);
}

/// A graceful stop ends the streams consumers hold open (with `UNAVAILABLE`, so they
/// reconnect) instead of waiting on them: a recreate of the server needn't wait for Docker to
/// kill it.
#[tokio::test]
async fn a_stop_ends_open_streams_instead_of_waiting_for_them() {
    let harness = Harness::start().await;
    let mut client = harness.client("a").await;
    let (tx, mut responses) = open(&mut client).await;
    tx.send(request(&["planner"], "", "")).await.unwrap();
    let first = next_within(&mut responses, Duration::from_secs(5))
        .await
        .unwrap();
    tx.send(request(&["planner"], &first.version_info, &first.nonce))
        .await
        .unwrap();

    // The client keeps its request stream open, as Envoy and the Rust client do.
    tokio::time::timeout(Duration::from_secs(5), harness.stop())
        .await
        .expect("the server stopped with a stream open");
    let status = responses.message().await.unwrap_err();
    assert_eq!(status.code(), tonic::Code::Unavailable, "{status}");
    drop(tx);
}
