//! The host source's logs never carry the path token or a value, through a rotation and a
//! host that goes down.
//!
//! On its own test binary: tracing caches whether a callsite is enabled process-wide, so a
//! test without a subscriber running in parallel could switch the captured callsites off.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::host::*;

/// Collects everything the server logs.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn logs_never_carry_the_token_or_a_value() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // The test runtime is single-threaded, so the spawned tasks log through this default.
    let _guard = tracing::subscriber::set_default(subscriber);

    let host = FakeHost::new(&[
        ("PLANNER", "planner-SENTINEL1"),
        ("OTHER", "other-SENTINEL1"),
    ]);
    let (addr, stop) = host.serve("127.0.0.1:0".parse().unwrap()).await;
    let harness = Harness::start(addr).await.unwrap();
    let mut client = harness.client("a").await;
    let (_tx, mut responses) = subscribe(&mut client, "planner", "planner-SENTINEL1").await;
    host.set("PLANNER", "planner-SENTINEL2");
    next_within(&mut responses, Duration::from_secs(5))
        .await
        .unwrap();
    // A failure is logged too.
    stop.stop().await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    harness.stop().await;

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("secret changed at the host source"), "{logs}");
    assert!(logs.contains("keeping the last value"), "{logs}");
    for secret in [TOKEN, "SENTINEL"] {
        assert!(!logs.contains(secret), "{secret} in the logs:\n{logs}");
    }
}
