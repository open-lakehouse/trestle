//! The key service over real Unix sockets, with olai-key-service's `KeyServiceProvider`, in
//! process (no Docker).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hyper_util::rt::TokioIo;
use olai_key_service::KeyServiceProvider;
use olai_key_service::v1::UnwrapRequest;
use olai_key_service::v1::key_service_client::KeyServiceClient;
use olai_secret_server::{CallerId, Config, Keyring, Policy, serve};
use olai_store::{EnvelopeEncryptor, Error, KeyProvider, LocalKeyProvider};
use tokio::net::UnixStream;
use tokio::sync::oneshot;
use tonic::transport::{Channel, Endpoint};

const KEY: &str = "server-kek";

fn material(version: u8) -> Vec<u8> {
    vec![version; 32]
}

/// A keyring JSON with `versions`, `active`, an optional `min_decrypt`, and `kek-1` → 1 when
/// version 1 is present.
fn keyring(versions: &[u8], active: u8, min_decrypt: Option<u8>) -> String {
    let versions: Vec<String> = versions
        .iter()
        .map(|v| format!(r#""{v}": "{}""#, STANDARD.encode(material(*v))))
        .collect();
    let min = min_decrypt.map_or(String::new(), |m| format!(r#", "min_decrypt": {m}"#));
    format!(
        r#"{{"versions": {{{}}}, "active": {active}{min}, "aliases": {{"kek-1": 1}}}}"#,
        versions.join(", ")
    )
}

/// The server, serving consumer `a` the key `server-kek` and consumer `b` the key `b-kek`.
struct Server {
    dir: PathBuf,
    policy: Policy,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start(dir: &Path, a_keyring: &str) -> Self {
        let yaml = format!(
            "version: 1\nlisteners:\n  - socket: {a}\n    consumer: a\n    keys:\n      - {{name: {KEY}, env: A_KEK}}\n  - socket: {b}\n    consumer: b\n    keys:\n      - {{name: b-kek, env: B_KEK}}\n",
            a = dir.join("a/sds.sock").display(),
            b = dir.join("b/sds.sock").display(),
        );
        let config = Config::from_yaml(&yaml).unwrap();
        let b_keyring = keyring(&[9], 9, None).replace(r#""kek-1": 1"#, "");
        let policy = config
            .resolve(|name| match name {
                "A_KEK" => Some(a_keyring.into()),
                "B_KEK" => Some(b_keyring.clone().into()),
                _ => None,
            })
            .unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let served = policy.clone();
        let task = tokio::spawn(async move {
            serve(&config, served, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        });
        for consumer in ["a", "b"] {
            let path = dir.join(consumer).join("sds.sock");
            for _ in 0..100 {
                if UnixStream::connect(&path).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        Self {
            dir: dir.to_path_buf(),
            policy,
            stop: Some(stop),
            task,
        }
    }

    fn socket(&self, consumer: &str) -> PathBuf {
        self.dir.join(consumer).join("sds.sock")
    }

    async fn provider(&self, consumer: &str, key: &str) -> olai_store::Result<KeyServiceProvider> {
        builder(&self.socket(consumer), key).connect().await
    }

    async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.await.unwrap();
    }
}

fn builder(socket: &Path, key: &str) -> olai_key_service::KeyServiceProviderBuilder {
    KeyServiceProvider::builder(socket, key)
        .connect_timeout(Duration::from_secs(5))
        .call_timeout(Duration::from_secs(5))
        .backoff(Duration::from_millis(10), Duration::from_millis(100))
}

async fn raw_client(socket: &Path) -> KeyServiceClient<Channel> {
    let path = socket.to_path_buf();
    let channel = Endpoint::from_static("http://keys.invalid")
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        }))
        .await
        .unwrap();
    KeyServiceClient::new(channel)
}

/// Waits for the provider to report `active` as the active version.
async fn await_active(provider: &mut KeyServiceProvider, active: u32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while provider.key_info().active_version != active {
            provider.changed().await.unwrap();
        }
    })
    .await
    .expect("the provider never saw the new active version");
}

#[tokio::test]
async fn seals_and_opens_through_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), &keyring(&[1], 1, None)).await;
    let provider = server.provider("a", KEY).await.unwrap();
    assert_eq!(provider.active_kek_id(), "server-kek@v1");
    let rendered = format!("{provider:?}");
    assert!(
        !rendered.contains(&STANDARD.encode(material(1))),
        "{rendered}"
    );

    let encryptor = EnvelopeEncryptor::new(Arc::new(provider));
    let blob = encryptor.seal("row-1", b"a secret value").await.unwrap();
    assert_eq!(
        encryptor.open("row-1", &blob).await.unwrap(),
        b"a secret value"
    );
    assert!(encryptor.open("row-2", &blob).await.is_err());
    // Nothing to rewrap: the blob is under the active version.
    assert!(encryptor.rewrap(&blob).await.unwrap().is_none());

    // The wrap is LocalKeyProvider's format under the canonical id.
    let local =
        EnvelopeEncryptor::local(LocalKeyProvider::single("server-kek@v1", material(1)).unwrap());
    assert_eq!(local.open("row-1", &blob).await.unwrap(), b"a secret value");
    server.stop().await;
}

#[tokio::test]
async fn opens_and_rewraps_a_local_key_provider_blob_through_an_alias() {
    // Sealed before the key service existed, by a LocalKeyProvider with the KEK as `kek-1`.
    let legacy = EnvelopeEncryptor::local(LocalKeyProvider::single("kek-1", material(1)).unwrap());
    let blob = legacy.seal("row-1", b"sealed long ago").await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    // Imported as version 1, aliased `kek-1`; version 2 is active.
    let server = Server::start(dir.path(), &keyring(&[1, 2], 2, None)).await;
    let provider = Arc::new(server.provider("a", KEY).await.unwrap());
    assert_eq!(provider.key_info().aliases.get("kek-1"), Some(&1));
    let encryptor = EnvelopeEncryptor::new(provider.clone());
    assert_eq!(
        encryptor.open("row-1", &blob).await.unwrap(),
        b"sealed long ago"
    );

    let rewrapped = encryptor
        .rewrap(&blob)
        .await
        .unwrap()
        .expect("under an old KEK id");
    assert_eq!(
        encryptor.open("row-1", &rewrapped).await.unwrap(),
        b"sealed long ago"
    );
    assert!(encryptor.rewrap(&rewrapped).await.unwrap().is_none());
    // Now under version 2: a provider holding only version 2 opens it, not the original.
    let v2 =
        EnvelopeEncryptor::local(LocalKeyProvider::single("server-kek@v2", material(2)).unwrap());
    assert_eq!(
        v2.open("row-1", &rewrapped).await.unwrap(),
        b"sealed long ago"
    );
    assert!(v2.open("row-1", &blob).await.is_err());

    // An alias the keyring doesn't have is refused by the server.
    let wrapped = provider.wrap_dek(&[7; 32], "server-kek@v1").await.unwrap();
    assert!(provider.unwrap_dek(&wrapped, "kek-9").await.is_err());
    server.stop().await;
}

#[tokio::test]
async fn a_recreated_server_rotates_a_running_client() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), &keyring(&[1], 1, None)).await;
    let mut provider = server.provider("a", KEY).await.unwrap();
    let blob = {
        let encryptor = EnvelopeEncryptor::new(Arc::new(server.provider("a", KEY).await.unwrap()));
        encryptor.seal("row-1", b"v1 value").await.unwrap()
    };
    server.stop().await;

    // The rotation: the server comes back with version 2 active.
    let server = Server::start(dir.path(), &keyring(&[1, 2], 2, None)).await;
    await_active(&mut provider, 2).await;
    assert_eq!(provider.active_kek_id(), "server-kek@v2");

    let encryptor = EnvelopeEncryptor::new(Arc::new(provider));
    assert_eq!(encryptor.open("row-1", &blob).await.unwrap(), b"v1 value");
    let fresh = encryptor.seal("row-2", b"v2 value").await.unwrap();
    let v2 =
        EnvelopeEncryptor::local(LocalKeyProvider::single("server-kek@v2", material(2)).unwrap());
    assert_eq!(v2.open("row-2", &fresh).await.unwrap(), b"v2 value");
    let rewrapped = encryptor
        .rewrap(&blob)
        .await
        .unwrap()
        .expect("v1 is retired");
    assert_eq!(v2.open("row-1", &rewrapped).await.unwrap(), b"v1 value");
    server.stop().await;
}

#[tokio::test]
async fn a_replaced_keyring_reaches_a_watching_client() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), &keyring(&[1], 1, None)).await;
    let mut provider = server.provider("a", KEY).await.unwrap();
    let handle = server.policy.key(&CallerId::new("a"), KEY).unwrap().clone();
    handle
        .replace(Keyring::from_json(KEY, keyring(&[1, 2, 3], 3, Some(2)).as_bytes()).unwrap())
        .unwrap();
    await_active(&mut provider, 3).await;
    assert_eq!(provider.key_info().min_decrypt_version, 2);
    assert_eq!(provider.active_kek_id(), "server-kek@v3");
    server.stop().await;
}

#[tokio::test]
async fn refuses_versions_below_min_decrypt() {
    let v1 =
        EnvelopeEncryptor::local(LocalKeyProvider::single("server-kek@v1", material(1)).unwrap());
    let old = v1.seal("row-1", b"old").await.unwrap();
    let legacy = EnvelopeEncryptor::local(LocalKeyProvider::single("kek-1", material(1)).unwrap());
    let aliased = legacy.seal("row-2", b"older").await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), &keyring(&[1, 2], 2, Some(2))).await;
    let provider = Arc::new(server.provider("a", KEY).await.unwrap());
    let encryptor = EnvelopeEncryptor::new(provider.clone());
    for (name, blob) in [("row-1", &old), ("row-2", &aliased)] {
        let err = encryptor.open(name, blob).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
        assert!(
            err.to_string().contains("below its min_decrypt version 2"),
            "{err}"
        );
        // And so it can't be rewrapped either.
        assert!(encryptor.rewrap(blob).await.is_err());
    }
    // Nor can a new DEK be wrapped under a retired version.
    assert!(provider.wrap_dek(&[7; 32], "server-kek@v1").await.is_err());
    // An unknown version is refused too.
    assert!(
        provider
            .unwrap_dek(&[0; 60], "server-kek@v7")
            .await
            .is_err()
    );
    server.stop().await;
}

#[tokio::test]
async fn another_consumers_socket_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), &keyring(&[1], 1, None)).await;
    let a = server.provider("a", KEY).await.unwrap();
    let wrapped = a.wrap_dek(&[7; 32], "server-kek@v1").await.unwrap();

    // b's socket doesn't serve a's key: connecting fails, the same as for a key nobody has.
    let err = server.provider("b", KEY).await.unwrap_err().to_string();
    assert!(err.contains("key server-kek not found"), "{err}");
    let err = server
        .provider("a", "nobody")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("key nobody not found"), "{err}");

    // A raw unwrap of a's DEK on b's socket: NOT_FOUND, nothing unwrapped.
    let mut client = raw_client(&server.socket("b")).await;
    let status = client
        .unwrap(UnwrapRequest {
            key: KEY.into(),
            version: 1,
            wrapped: wrapped.clone(),
            alias: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
    // b's own key doesn't open it either.
    let b = server.provider("b", "b-kek").await.unwrap();
    assert!(b.unwrap_dek(&wrapped, "b-kek@v9").await.is_err());
    assert_eq!(
        a.unwrap_dek(&wrapped, "server-kek@v1")
            .await
            .unwrap()
            .as_slice(),
        [7; 32]
    );
    server.stop().await;
}

#[tokio::test]
async fn an_opt_in_dek_cache_rides_out_a_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), &keyring(&[1], 1, None)).await;
    let cached = Arc::new(
        builder(&server.socket("a"), KEY)
            .call_timeout(Duration::from_millis(300))
            .dek_cache(Duration::from_secs(60), 16)
            .connect()
            .await
            .unwrap(),
    );
    let uncached = Arc::new(
        builder(&server.socket("a"), KEY)
            .call_timeout(Duration::from_millis(300))
            .connect()
            .await
            .unwrap(),
    );
    let with_cache = EnvelopeEncryptor::new(cached);
    let without = EnvelopeEncryptor::new(uncached);
    let blob = with_cache.seal("row-1", b"value").await.unwrap();
    assert_eq!(with_cache.open("row-1", &blob).await.unwrap(), b"value");
    assert_eq!(without.open("row-1", &blob).await.unwrap(), b"value");
    server.stop().await;

    // The server is gone: the cache still opens the value, the default provider can't.
    assert_eq!(with_cache.open("row-1", &blob).await.unwrap(), b"value");
    let err = without.open("row-1", &blob).await.unwrap_err().to_string();
    assert!(err.contains("unreachable"), "{err}");
    // A new value needs a wrap, which is never cached.
    assert!(with_cache.seal("row-2", b"value").await.is_err());
}
