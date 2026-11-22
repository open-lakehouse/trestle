//! Envoy, at the pinned egress release, injects a bearer token it fetched over SDS from the
//! secret server, and picks up a rotated value when only the server is recreated.
//!
//! Needs Docker (`--features emulator-it`). The server image comes from
//! `SECRET_SERVER_IMAGE`; when that is unset, the test builds
//! `crates/secret-server/Dockerfile` itself (passing `$CARGO_HOME/config.toml` as the
//! `cargo-config` build secret when there is one, for registry mirrors).
//!
//! The layout mirrors the egress gateway, plus a second consumer:
//!
//! - one secret server with two listeners, consumer `a` (`planner`) and consumer `b`
//!   (`reviewer`), each on its own named volume;
//! - one Envoy that mounts both volumes. Listener 1 injects `planner` fetched on `a`'s
//!   socket, listener 2 asks for `planner` on `b`'s socket (it must never get it), and
//!   listener 3 injects `reviewer` on `b`'s socket (so `b`'s socket provably works);
//! - an echo upstream that returns the request headers it saw.

#![cfg(feature = "emulator-it")]

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use testcontainers::core::{ContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// The Envoy release the egress gateway pins. `credential_injector` and the generic
/// credential are alpha, so this test runs against exactly that release.
const ENVOY_IMAGE: (&str, &str) = ("envoyproxy/envoy", "v1.40.2");
/// An upstream that echoes the request (headers included) as JSON.
const ECHO_IMAGE: (&str, &str) = ("mendhak/http-https-echo", "39");
const ECHO_PORT: u16 = 8080;
const ADMIN_PORT: u16 = 9901;
const PLANNER_ON_A: u16 = 10001;
const PLANNER_ON_B: u16 = 10002;
const REVIEWER_ON_B: u16 = 10003;
/// Envoy's `initial_fetch_timeout` for every SDS secret.
const INITIAL_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

fn token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The server image: `SECRET_SERVER_IMAGE`, or one built from the Dockerfile.
fn server_image() -> (String, String) {
    let reference = std::env::var("SECRET_SERVER_IMAGE").unwrap_or_else(|_| {
        let reference = "olai-secret-server:it".to_string();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut build = Command::new("docker");
        build.arg("build").arg("-q").arg("-t").arg(&reference);
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
        if let Some(config) = cargo_home.map(|h| h.join("config.toml"))
            && config.exists()
        {
            build
                .arg("--secret")
                .arg(format!("id=cargo-config,src={}", config.display()));
        }
        build
            .arg("-f")
            .arg(root.join("crates/secret-server/Dockerfile"))
            .arg(&root);
        let status = build.status().expect("docker build runs");
        assert!(status.success(), "building the secret server image failed");
        reference
    });
    let (name, tag) = reference
        .rsplit_once(':')
        .expect("image reference has a tag");
    (name.to_string(), tag.to_string())
}

/// Named volumes that outlive the containers that mount them, removed on drop.
struct Volumes(Vec<String>);

impl Drop for Volumes {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f"])
            .args(&self.0)
            .output();
    }
}

struct Run {
    id: String,
    network: String,
    volume_a: String,
    volume_b: String,
}

impl Run {
    fn new() -> (Self, Volumes) {
        let id = token()[..12].to_string();
        let run = Self {
            network: format!("sds-it-{id}"),
            volume_a: format!("sds-it-a-{id}"),
            volume_b: format!("sds-it-b-{id}"),
            id,
        };
        let volumes = Volumes(vec![run.volume_a.clone(), run.volume_b.clone()]);
        (run, volumes)
    }

    fn echo_host(&self) -> String {
        format!("echo-{}", self.id)
    }
}

const SERVER_CONFIG: &str = r#"version: 1
listeners:
  - socket: /run/trestle-sds/a/sds.sock
    consumer: a
    secrets:
      - name: planner
        env: PLANNER_KEY
  - socket: /run/trestle-sds/b/sds.sock
    consumer: b
    secrets:
      - name: reviewer
        env: REVIEWER_KEY
"#;

async fn start_server(
    run: &Run,
    image: &(String, String),
    planner: &str,
    reviewer: &str,
) -> ContainerAsync<GenericImage> {
    GenericImage::new(&image.0, &image.1)
        .with_wait_for(WaitFor::message_on_stderr("listening"))
        .with_network(&run.network)
        .with_mount(Mount::volume_mount(&run.volume_a, "/run/trestle-sds/a"))
        .with_mount(Mount::volume_mount(&run.volume_b, "/run/trestle-sds/b"))
        .with_copy_to(
            "/etc/trestle/secret-server.yaml",
            SERVER_CONFIG.as_bytes().to_vec(),
        )
        .with_env_var("PLANNER_KEY", planner)
        .with_env_var("REVIEWER_KEY", reviewer)
        .with_env_var("RUST_LOG", "olai_secret_server=debug")
        .start()
        .await
        .expect("secret server starts")
}

/// One listener whose filter chain injects `secret`, fetched over SDS through `cluster`.
fn listener(name: &str, port: u16, secret: &str, cluster: &str) -> String {
    let timeout = INITIAL_FETCH_TIMEOUT.as_secs();
    format!(
        r#"
  - name: {name}
    address: {{ socket_address: {{ address: 0.0.0.0, port_value: {port} }} }}
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          stat_prefix: {name}
          route_config:
            virtual_hosts:
            - name: echo
              domains: ["*"]
              routes:
              - match: {{ prefix: "/" }}
                route: {{ cluster: echo }}
          http_filters:
          # The caller's own credentials go before injection. A route's
          # request_headers_to_remove would run in the router, after the injector, and
          # remove the injected header too.
          - name: envoy.filters.http.header_mutation
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.header_mutation.v3.HeaderMutation
              mutations:
                request_mutations:
                - remove: authorization
                - remove: x-api-key
          - name: envoy.filters.http.credential_injector
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.credential_injector.v3.CredentialInjector
              overwrite: true
              allow_request_without_credential: false
              credential:
                name: envoy.http.injected_credentials.generic
                typed_config:
                  "@type": type.googleapis.com/envoy.extensions.http.injected_credentials.generic.v3.Generic
                  header: Authorization
                  header_value_prefix: "Bearer "
                  credential:
                    name: {secret}
                    sds_config:
                      resource_api_version: V3
                      initial_fetch_timeout: {timeout}s
                      api_config_source:
                        api_type: GRPC
                        transport_api_version: V3
                        grpc_services:
                        - envoy_grpc: {{ cluster_name: {cluster} }}
          - name: envoy.filters.http.router
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router
"#
    )
}

fn sds_cluster(name: &str, socket: &str) -> String {
    format!(
        r#"
  - name: {name}
    type: STATIC
    typed_extension_protocol_options:
      envoy.extensions.upstreams.http.v3.HttpProtocolOptions:
        "@type": type.googleapis.com/envoy.extensions.upstreams.http.v3.HttpProtocolOptions
        explicit_http_config: {{ http2_protocol_options: {{}} }}
    load_assignment:
      cluster_name: {name}
      endpoints:
      - lb_endpoints:
        - endpoint: {{ address: {{ pipe: {{ path: {socket} }} }} }}
"#
    )
}

fn envoy_bootstrap(run: &Run) -> String {
    let echo = run.echo_host();
    format!(
        r#"# SDS over gRPC needs a node id and cluster.
node: {{ id: egress-gateway, cluster: egress-gateway }}
# Test-only: the egress gateway binds admin to 127.0.0.1; here the test reads it.
admin:
  address: {{ socket_address: {{ address: 0.0.0.0, port_value: {ADMIN_PORT} }} }}
static_resources:
  listeners:{l1}{l2}{l3}
  clusters:{c1}{c2}
  - name: echo
    type: STRICT_DNS
    load_assignment:
      cluster_name: echo
      endpoints:
      - lb_endpoints:
        - endpoint: {{ address: {{ socket_address: {{ address: {echo}, port_value: {ECHO_PORT} }} }} }}
"#,
        l1 = listener("planner_on_a", PLANNER_ON_A, "planner", "secret_a"),
        l2 = listener("planner_on_b", PLANNER_ON_B, "planner", "secret_b"),
        l3 = listener("reviewer_on_b", REVIEWER_ON_B, "reviewer", "secret_b"),
        c1 = sds_cluster("secret_a", "/run/trestle-sds/a/sds.sock"),
        c2 = sds_cluster("secret_b", "/run/trestle-sds/b/sds.sock"),
    )
}

struct Envoy {
    container: ContainerAsync<GenericImage>,
    http: reqwest::Client,
}

impl Envoy {
    async fn url(&self, port: u16) -> String {
        let host = self.container.get_host().await.unwrap();
        let port = self
            .container
            .get_host_port_ipv4(ContainerPort::Tcp(port))
            .await
            .unwrap();
        format!("http://{host}:{port}")
    }

    /// Status and the `authorization` and `x-api-key` headers the upstream saw (if it was
    /// reached). Status 0 is a connection error (the port forward may still be warming up).
    async fn call(&self, port: u16) -> (u16, Option<String>, Option<String>) {
        let sent = self
            .http
            .get(format!("{}/check", self.url(port).await))
            .header("authorization", "Bearer caller-supplied")
            .header("x-api-key", "caller-supplied")
            .send()
            .await;
        let Ok(response) = sent else {
            return (0, None, None);
        };
        let status = response.status().as_u16();
        if status != 200 {
            return (status, None, None);
        }
        let body: serde_json::Value = response.json().await.expect("echo returns JSON");
        let header = |name: &str| body["headers"][name].as_str().map(str::to_string);
        (status, header("authorization"), header("x-api-key"))
    }

    async fn admin(&self, path: &str) -> String {
        self.http
            .get(format!("{}{path}", self.url(ADMIN_PORT).await))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }

    /// Polls `port` until the upstream sees `Bearer <expected>`.
    async fn wait_for_token(&self, port: u16, expected: &str, within: Duration) {
        let want = format!("Bearer {expected}");
        let deadline = Instant::now() + within;
        let mut last = None;
        while Instant::now() < deadline {
            let (status, auth, api_key) = self.call(port).await;
            if status == 200 && auth.as_deref() == Some(want.as_str()) {
                assert_eq!(api_key, None, "the caller's x-api-key reached the upstream");
                return;
            }
            last = Some((status, auth.map(|a| a != want)));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!(
            "port {port} never injected the expected token (last: status, wrong token = {last:?})"
        );
    }
}

#[tokio::test]
async fn envoy_injects_rotates_and_isolates_consumers() {
    let image = server_image();
    let (run, _volumes) = Run::new();
    let planner_v1 = token();
    let planner_v2 = token();
    let reviewer = token();

    let _echo = GenericImage::new(ECHO_IMAGE.0, ECHO_IMAGE.1)
        .with_wait_for(WaitFor::message_on_stdout("Listening"))
        .with_network(&run.network)
        .with_container_name(run.echo_host())
        .start()
        .await
        .expect("echo upstream starts");

    let server = start_server(&run, &image, &planner_v1, &reviewer).await;

    let container = GenericImage::new(ENVOY_IMAGE.0, ENVOY_IMAGE.1)
        .with_exposed_port(ContainerPort::Tcp(ADMIN_PORT))
        .with_exposed_port(ContainerPort::Tcp(PLANNER_ON_A))
        .with_exposed_port(ContainerPort::Tcp(PLANNER_ON_B))
        .with_exposed_port(ContainerPort::Tcp(REVIEWER_ON_B))
        .with_wait_for(WaitFor::message_on_stderr("starting main dispatch loop"))
        .with_network(&run.network)
        .with_mount(Mount::volume_mount(&run.volume_a, "/run/trestle-sds/a"))
        .with_mount(Mount::volume_mount(&run.volume_b, "/run/trestle-sds/b"))
        .with_copy_to("/etc/envoy/envoy.yaml", envoy_bootstrap(&run).into_bytes())
        .with_startup_timeout(Duration::from_secs(60))
        .start()
        .await
        .expect("envoy starts");
    let envoy_id = container.id().to_string();
    let envoy = Envoy {
        container,
        http: reqwest::Client::new(),
    };

    // 1. Consumer a's secret is injected, in place of the caller's own credentials.
    envoy
        .wait_for_token(PLANNER_ON_A, &planner_v1, Duration::from_secs(30))
        .await;
    // 2. Consumer b's socket works for b's own secret.
    envoy
        .wait_for_token(REVIEWER_ON_B, &reviewer, Duration::from_secs(30))
        .await;
    // 3. b's socket never yields a's secret, well past the initial fetch timeout.
    tokio::time::sleep(INITIAL_FETCH_TIMEOUT + Duration::from_secs(2)).await;
    let (status, auth, _) = envoy.call(PLANNER_ON_B).await;
    assert_eq!(status, 401, "a's secret over b's socket: {auth:?}");

    // 4. The admin interface never shows a value.
    let dumps = [
        envoy.admin("/config_dump?include_eds").await,
        envoy
            .admin("/config_dump?resource=dynamic_active_secrets")
            .await,
    ];
    assert!(
        dumps[1].contains("planner") && dumps[1].contains("reviewer"),
        "{}",
        dumps[1]
    );
    for dump in &dumps {
        for value in [&planner_v1, &reviewer] {
            assert!(
                !dump.contains(value.as_str()),
                "a secret value is in /config_dump"
            );
        }
    }

    // 5. Rotation: recreate only the server, with a new value on the same volumes.
    server.rm().await.expect("server removed");
    let server = start_server(&run, &image, &planner_v2, &reviewer).await;
    envoy
        .wait_for_token(PLANNER_ON_A, &planner_v2, Duration::from_secs(60))
        .await;
    assert_eq!(envoy.container.id(), envoy_id);
    assert!(
        envoy.container.is_running().await.unwrap(),
        "envoy kept running"
    );
    let dump = envoy
        .admin("/config_dump?resource=dynamic_active_secrets")
        .await;
    assert!(
        !dump.contains(planner_v2.as_str()),
        "the rotated value is in /config_dump"
    );
    let (status, _, _) = envoy.call(PLANNER_ON_B).await;
    assert_eq!(status, 401);

    server.rm().await.unwrap();
    envoy.container.rm().await.unwrap();
}
