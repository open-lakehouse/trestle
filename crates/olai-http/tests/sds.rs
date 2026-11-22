//! `SdsCredentialProvider` and `BearerAuth::sds` against an in-process `olai-secret-server`
//! on real Unix sockets (no Docker).

#![cfg(feature = "sds")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use olai_http::bearer::BearerAuth;
use olai_http::sds::SdsCredentialProvider;
use olai_http::{BackoffConfig, CredentialProvider, Error};
use olai_secret_server::{Config, serve};
use tokio::net::UnixStream;

/// A secret server with consumers `a` (secret `planner`) and `b` (secret `other`), under
/// `dir`, on a runtime of its own: stopping it drops that runtime, which ends every open
/// connection and stream at once, as a recreated container does.
struct Server {
    runtime: Option<tokio::runtime::Runtime>,
}

impl Server {
    async fn start(dir: &Path, planner: &str) -> Self {
        let yaml = format!(
            "version: 1\nlisteners:\n  - socket: {a}\n    consumer: a\n    secrets:\n      - {{name: planner, env: PLANNER}}\n  - socket: {b}\n    consumer: b\n    secrets:\n      - {{name: other, env: OTHER}}\n",
            a = socket(dir, "a").display(),
            b = socket(dir, "b").display(),
        );
        let config = Config::from_yaml(&yaml).unwrap();
        let planner = planner.to_string();
        let policy = config
            .resolve(|name| match name {
                "PLANNER" => Some(planner.clone().into()),
                "OTHER" => Some("other-value".into()),
                _ => None,
            })
            .unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.spawn(async move {
            serve(&config, policy, std::future::pending())
                .await
                .unwrap();
        });
        // The server removes a stale socket and binds a new one; wait until it answers.
        for _ in 0..200 {
            if UnixStream::connect(socket(dir, "a")).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            runtime: Some(runtime),
        }
    }

    async fn stop(mut self) {
        let runtime = self.runtime.take().unwrap();
        tokio::task::spawn_blocking(move || runtime.shutdown_timeout(Duration::from_secs(1)))
            .await
            .unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

fn socket(dir: &Path, consumer: &str) -> PathBuf {
    dir.join(consumer).join("sds.sock")
}

fn fast() -> BackoffConfig {
    BackoffConfig {
        init_backoff: Duration::from_millis(20),
        max_backoff: Duration::from_millis(100),
        base: 2.,
    }
}

fn provider(dir: &Path, consumer: &str, name: &str) -> SdsCredentialProvider {
    SdsCredentialProvider::new(socket(dir, consumer), name)
        .with_backoff(fast())
        .with_connect_timeout(Duration::from_secs(5))
}

async fn value(provider: &SdsCredentialProvider) -> String {
    let secret = provider.get_credential().await.unwrap();
    secret.expose_str().unwrap().to_string()
}

/// Polls until `provider` yields `expected`.
async fn eventually(provider: &SdsCredentialProvider, expected: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if value(provider).await == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the provider never yielded the rotated value"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_first_fetch_returns_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), "planner-1").await;
    let kek = provider(dir.path(), "a", "planner");
    let secret = kek.get_credential().await.unwrap();
    assert_eq!(secret.name(), "planner");
    assert_eq!(secret.expose_bytes(), b"planner-1");
    assert!(!secret.version().is_empty());
    // Cached: the same value, without another fetch.
    assert!(Arc::ptr_eq(&secret, &kek.get_credential().await.unwrap()));

    let bearer = BearerAuth::new().sds(socket(dir.path(), "a"), "planner");
    assert_eq!(bearer.get_credential().await.unwrap().token, "planner-1");
    server.stop().await;
}

#[tokio::test]
async fn a_restarted_server_rotates_the_value_without_a_new_provider() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), "planner-1").await;
    let kek = provider(dir.path(), "a", "planner");
    let bearer = BearerAuth::new().sds(socket(dir.path(), "a"), "planner");
    assert_eq!(value(&kek).await, "planner-1");
    assert_eq!(bearer.get_credential().await.unwrap().token, "planner-1");
    let first_version = kek.get_credential().await.unwrap().version().to_string();

    // While the server is gone, the last value is kept.
    server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(value(&kek).await, "planner-1");
    assert_eq!(bearer.get_credential().await.unwrap().token, "planner-1");

    // A new server (a recreate with a rotated launch value) reaches the same provider.
    let server = Server::start(dir.path(), "planner-2").await;
    eventually(&kek, "planner-2").await;
    assert_ne!(kek.get_credential().await.unwrap().version(), first_version);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while bearer.get_credential().await.unwrap().token != "planner-2" {
        assert!(
            tokio::time::Instant::now() < deadline,
            "bearer never rotated"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.stop().await;
}

#[tokio::test]
async fn a_provider_built_before_the_server_starts_waits_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let kek = provider(dir.path(), "a", "planner");
    let path = dir.path().to_path_buf();
    let starter = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Server::start(&path, "late").await
    });
    assert_eq!(value(&kek).await, "late");
    starter.await.unwrap().stop().await;
}

#[tokio::test]
async fn a_name_the_consumer_may_not_see_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), "planner-1").await;
    // Another consumer's name, on this consumer's socket.
    let err = provider(dir.path(), "a", "other")
        .get_credential()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }), "{err}");
    assert!(err.to_string().contains("`other`"), "{err}");
    // A name nobody holds.
    let err = provider(dir.path(), "b", "planner")
        .get_credential()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }), "{err}");
    assert!(!err.to_string().contains("planner-1"), "{err}");
    server.stop().await;
}

#[tokio::test]
async fn no_server_fails_after_the_connect_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let err = provider(dir.path(), "a", "planner")
        .with_connect_timeout(Duration::from_millis(200))
        .get_credential()
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("cannot fetch SDS secret `planner`"),
        "{err}"
    );
}

#[tokio::test]
async fn debug_never_prints_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(dir.path(), "hunter2-planner").await;
    let kek = provider(dir.path(), "a", "planner");
    let secret = kek.get_credential().await.unwrap();
    let bearer = BearerAuth::new().sds(socket(dir.path(), "a"), "planner");
    let token = bearer.get_credential().await.unwrap();
    let rendered = format!("{secret:?} {kek:?} {bearer:?} {token:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
    server.stop().await;
}
