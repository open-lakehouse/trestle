//! Reads a bearer token over SDS and reports each new value by its SHA-256, never the value.
//!
//! The consumer the Docker tests run: `SDS_NAME` is the secret, and the socket is
//! `SDS_SOCKET`, else `TRESTLE_SDS_SOCKET` (which the planner sets), else
//! `/run/trestle-sds/sds.sock`. Each change prints one line,
//! `sds-probe sha256=<hex>`, to stdout.
//!
//! `cargo run -p olai-http --features sds --example sds_probe`

use std::time::Duration;

use olai_http::bearer::BearerAuth;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let socket = std::env::var("SDS_SOCKET")
        .or_else(|_| std::env::var("TRESTLE_SDS_SOCKET"))
        .unwrap_or_else(|_| "/run/trestle-sds/sds.sock".into());
    let name = std::env::var("SDS_NAME").expect("SDS_NAME names the secret to read");
    let provider = BearerAuth::new().sds(&socket, &name);
    println!("sds-probe reading {name} from {socket}");
    let mut last = String::new();
    loop {
        match provider.get_credential().await {
            Ok(bearer) => {
                let digest = ring::digest::digest(&ring::digest::SHA256, bearer.token.as_bytes());
                let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
                if hex != last {
                    println!("sds-probe sha256={hex}");
                    last = hex;
                }
            }
            Err(e) => println!("sds-probe error: {e}"),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
