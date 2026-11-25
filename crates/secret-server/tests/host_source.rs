//! The host source against a fake host endpoint, in process: a value changed at the host
//! reaches an open stream without a restart, a host that goes down leaves the last values in
//! place, and an unknown path or a missing variable fails startup.

mod common;

use std::time::Duration;

use common::host::*;
use olai_secret_server::{Config, ConfigError, resolve};
use tokio::net::TcpListener;

#[tokio::test]
async fn a_change_at_the_host_reaches_an_open_stream_without_a_restart() {
    let host = FakeHost::new(&[("PLANNER", "planner-v1"), ("OTHER", "other-v1")]);
    let (addr, _stop) = host.serve("127.0.0.1:0".parse().unwrap()).await;
    let harness = Harness::start(addr).await.unwrap();
    let mut client = harness.client("a").await;
    let (tx, mut responses) = subscribe(&mut client, "planner", "planner-v1").await;

    host.set("PLANNER", "planner-v2");
    let pushed = next_within(&mut responses, Duration::from_secs(5))
        .await
        .expect("the stream gets the rotated value");
    assert_eq!(values(&pushed), ["planner-v2"]);
    assert_eq!(fetch(&mut client, "planner").await, "planner-v2");
    tx.send(request(&["planner"], &pushed.version_info, &pushed.nonce))
        .await
        .unwrap();

    // A change to another consumer's value is not sent on this stream.
    host.set("OTHER", "other-v2");
    assert!(
        next_within(&mut responses, Duration::from_millis(500))
            .await
            .is_none()
    );
    assert_eq!(
        fetch(&mut harness.client("b").await, "other").await,
        "other-v2"
    );
    drop(tx);
    harness.stop().await;
}

#[tokio::test]
async fn a_host_that_goes_down_leaves_the_last_values() {
    let host = FakeHost::new(&[("PLANNER", "planner-v1"), ("OTHER", "other-v1")]);
    let (addr, stop) = host.serve("127.0.0.1:0".parse().unwrap()).await;
    let harness = Harness::start(addr).await.unwrap();
    let mut client = harness.client("a").await;
    let (_tx, mut responses) = subscribe(&mut client, "planner", "planner-v1").await;

    // Down: the server keeps answering with what it had.
    stop.stop().await;
    host.set("PLANNER", "planner-v2");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(fetch(&mut client, "planner").await, "planner-v1");
    assert!(
        next_within(&mut responses, Duration::from_millis(300))
            .await
            .is_none()
    );

    // Back, on the same address: the change arrives.
    let (_addr, _stop) = host.serve(addr).await;
    let pushed = next_within(&mut responses, Duration::from_secs(10))
        .await
        .expect("the change arrives once the host is back");
    assert_eq!(values(&pushed), ["planner-v2"]);
    harness.stop().await;
}

#[tokio::test]
async fn an_unknown_path_fails_startup_like_unset_variables() {
    let host = FakeHost::new(&[("PLANNER", "planner-v1"), ("OTHER", "other-v1")]);
    let (addr, _stop) = host.serve("127.0.0.1:0".parse().unwrap()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = Config::from_yaml(&yaml(dir.path(), addr)).unwrap();
    let wrong = |name: &str| (name == "HOST_TOKEN").then(|| "not-the-token".into());
    let error = resolve(&config, wrong, options()).await.unwrap_err();
    let ConfigError::MissingVariables(missing) = &error else {
        panic!("expected missing variables, got {error}");
    };
    let names: Vec<_> = missing.iter().map(|m| m.env.as_str()).collect();
    assert_eq!(names, ["PLANNER", "OTHER"]);
}

#[tokio::test]
async fn a_variable_the_host_does_not_serve_fails_startup() {
    let host = FakeHost::new(&[("PLANNER", "planner-v1")]);
    let (addr, _stop) = host.serve("127.0.0.1:0".parse().unwrap()).await;
    let error = Harness::start(addr).await.err().expect("startup fails");
    let message = error.to_string();
    assert!(message.contains("other (consumer b): $OTHER"), "{message}");
}

#[tokio::test]
async fn startup_fails_without_a_token_or_a_reachable_host() {
    let dir = tempfile::tempdir().unwrap();
    // Nothing listens on this port once the listener is dropped.
    let addr = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let config = Config::from_yaml(&yaml(dir.path(), addr)).unwrap();
    let error = resolve(&config, |_| None, options()).await.unwrap_err();
    assert!(
        error.to_string().contains("$HOST_TOKEN is not set"),
        "{error}"
    );

    let error = resolve(&config, token_only, options()).await.unwrap_err();
    let ConfigError::HostUnreachable { url, .. } = &error else {
        panic!("expected an unreachable host, got {error}");
    };
    assert_eq!(url, &format!("http://{addr}"));
    assert!(!error.to_string().contains(TOKEN), "{error}");
}
