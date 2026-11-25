//! The host source on a real Docker daemon: an instance whose secret server takes its values
//! from a [`HostEndpoint`] in this process. A consumer reaches a TLS upstream through the egress
//! gateway, which adds the route's bearer token, and a rotation at the host reaches the gateway
//! with no container recreated or restarted:
//! `cargo test -p olai-stack-runtime --features testing,host-endpoint --test live_host_source -- --ignored`.
//!
//! `SECRET_SERVER_IMAGE` overrides the catalog's secret server image (e.g. one built from
//! `crates/secret-server/Dockerfile`); the pinned one reads the host source. The upstream's certificate is signed by a CA the test makes, mounted over the
//! egress gateway's system store by a test-only `compose.override.yaml`. The state directory
//! sits under the build's tmp dir, because Docker VMs such as Colima only share the user's
//! home directory with containers.
#![cfg(all(feature = "testing", feature = "host-endpoint"))]

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use olai_stack_runtime::{
    EnvProvider, HostEndpoint, HostSecretEndpoint, Instances, Reach, SecretError, SecretProvider,
    SecretProviders, SecretValue,
};
use olai_stack_state::{Draft, SecretRef, Store, VarSet, VarValue};
use olai_stack_topology::plan::egress::{
    EGRESS_SERVICE, HOST_SOURCE_TOKEN_VAR, SECRET_SERVER_SERVICE,
};
use olai_stack_topology::{
    DataModule, EgressRoute, EnvManifest, InjectedCredential, ModuleId, Placement, PlanCtx,
    Provides, RenderSpec, Role, SecretSource, Selection, ServiceSpec, egress_url_var,
};

const ROUTE: &str = "search";
const ROUTE_KEY: &str = "EGRESS_IT_API_KEY";
const MODULE: &str = "egress-it";
const MODULE_PATH: &str = "modules/egress-it/module.yaml";
const ECHO_IMAGE: &str = "mendhak/http-https-echo:39";
const PROBE_IMAGE: &str = "curlimages/curl:8.11.1";

/// The route's credential, which the test rotates, as provider `rotating`.
#[derive(Clone)]
struct Rotating(Arc<Mutex<String>>);

impl SecretProvider for Rotating {
    fn name(&self) -> &str {
        "rotating"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        if key != "route" {
            return Err(SecretError::Missing {
                provider: "rotating".into(),
                key: key.into(),
            });
        }
        Ok(SecretValue::new(self.0.lock().unwrap().clone()))
    }
}

fn token(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{label}SENTINEL{nanos:x}")
}

/// A free port on the loopback, for the endpoint.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// When `service`'s container started, and how often it restarted: unchanged means neither
/// recreated nor restarted.
fn started(project: &str, service: &str) -> String {
    docker(&[
        "inspect",
        "--format",
        "{{.Id}} {{.State.StartedAt}} {{.RestartCount}}",
        &container(project, service),
    ])
}

/// A CA, and a certificate it signs for `echo`: `(ca.crt, echo.crt, echo.key)` in `dir`.
fn write_certs(dir: &Path) {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca.distinguished_name
        .push(rcgen::DnType::CommonName, "trestle egress test CA");
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca, ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["echo".to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("ca.crt"), ca_cert.pem()).unwrap();
    std::fs::write(dir.join("echo.crt"), leaf.pem()).unwrap();
    std::fs::write(dir.join("echo.key"), leaf_key.serialize_pem()).unwrap();
    // The echo server runs as a non-root user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for f in ["ca.crt", "echo.crt", "echo.key"] {
            std::fs::set_permissions(dir.join(f), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}

/// The test's own module: a TLS echo upstream and a consumer container to call it from.
fn module_yaml(certs: &Path) -> String {
    let service = |name: &str| ServiceSpec {
        name: name.into(),
        role: Role::new(name),
        placement: Placement::Container {
            service: name.into(),
        },
        endpoints: vec![],
        depends_on: vec![],
        base_path: String::new(),
    };
    let fragment = format!(
        r#"services:
  echo:
    image: {ECHO_IMAGE}
    environment:
      HTTPS_PORT: "8443"
      HTTPS_CERT_FILE: /certs/echo.crt
      HTTPS_KEY_FILE: /certs/echo.key
    volumes:
      - {certs}:/certs:ro
  probe:
    image: {PROBE_IMAGE}
    entrypoint: ["sleep", "infinity"]
"#,
        certs = certs.display()
    );
    let module = DataModule {
        id: ModuleId::from(MODULE),
        model: None,
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![service("echo"), service("probe")],
        provides: Provides::default(),
        knobs: vec![],
        renders: RenderSpec {
            fragment,
            files: vec![],
        }
        .for_compose(),
    };
    serde_yaml::to_string(&module).unwrap()
}

fn docker(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "docker {}: {}",
        args.first().unwrap_or(&""),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The container id of `service` in `project`.
fn container(project: &str, service: &str) -> String {
    let id = docker(&["compose", "-p", project, "ps", "-q", service]);
    assert!(!id.is_empty(), "{service} has no container");
    id
}

/// Call the route from the probe container with a credential of the caller's own. Returns the
/// status and the `authorization` and `x-api-key` headers the upstream saw.
fn call(project: &str) -> (u16, Option<String>, Option<String>) {
    let probe = container(project, "probe");
    let url_var = egress_url_var(ROUTE);
    let script = format!(
        r#"curl -s -m 10 -H "Authorization: Bearer caller-supplied" -H "x-api-key: caller-supplied" -w '\n%{{http_code}}' "${url_var}/check""#
    );
    let out = Command::new("docker")
        .args(["exec", &probe, "sh", "-c", &script])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", &text));
    let status = status.trim().parse().unwrap_or(0);
    if status != 200 {
        return (status, None, None);
    }
    let body: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let header = |name: &str| body["headers"][name].as_str().map(str::to_owned);
    (status, header("authorization"), header("x-api-key"))
}

/// Poll the route until the upstream sees `Bearer <expected>`.
fn wait_for_token(project: &str, expected: &str, within: Duration) {
    let want = format!("Bearer {expected}");
    let deadline = Instant::now() + within;
    let mut last = (0, false);
    while Instant::now() < deadline {
        let (status, auth, api_key) = call(project);
        if status == 200 && auth.as_deref() == Some(want.as_str()) {
            assert_eq!(api_key, None, "the caller's x-api-key reached the upstream");
            return;
        }
        // Never print a value: only whether it was the expected one.
        last = (status, auth.is_some_and(|a| a != want));
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("the upstream never saw the expected token (last status, wrong token: {last:?})");
}

/// No file under `dir` holds any of `values`.
fn assert_no_value(dir: &Path, values: &[&str]) {
    let mut stack = vec![dir.to_owned()];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                files += 1;
                let text = String::from_utf8_lossy(&bytes);
                for v in values {
                    assert!(!text.contains(v), "a secret value is in {}", path.display());
                }
            }
        }
    }
    assert!(files > 0);
}

/// The services of `project` whose container environment holds `value`.
fn containers_holding(project: &str, value: &str) -> Vec<String> {
    let ids = docker(&[
        "ps",
        "-aq",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
    ]);
    let mut out = Vec::new();
    for id in ids.lines() {
        let inspect = docker(&[
            "inspect",
            "--format",
            "{{index .Config.Labels \"com.docker.compose.service\"}}\n{{json .Config.Env}}",
            id,
        ]);
        let (service, env) = inspect.split_once('\n').unwrap();
        if env.contains(value) {
            out.push(service.to_owned());
        }
    }
    out
}

// Multi-threaded: the endpoint runs in this process, and the checks block their thread while
// they poll the route.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a Docker daemon, and pulls the secret server, envoy, echo and curl"]
async fn a_rotation_at_the_host_reaches_the_gateway_with_no_container_recreated() {
    let state = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let certs = state.path().join("certs");
    write_certs(&certs);

    let port = free_port();
    let store = Arc::new(Store::open(&state.path().join("state.db")).unwrap());
    let ws = store.create_workspace("hostsrc", false).unwrap();
    let mut selection = Selection::modules([MODULE]);
    selection.gateway.enabled = false;
    selection.egress.source = SecretSource::Host;
    selection.egress.host_port = Some(port);
    if let Ok(image) = std::env::var("SECRET_SERVER_IMAGE") {
        selection.knob_overrides.insert(
            ModuleId::from("egress-gateway"),
            [("secret_server_image".to_string(), image)].into(),
        );
    }
    selection.egress.routes.push(EgressRoute {
        name: ROUTE.into(),
        function: None,
        upstream: Some("https://echo:8443".into()),
        path: "/".into(),
        consumers: vec![ModuleId::from(MODULE)],
        credential: InjectedCredential::Bearer {
            secret: ROUTE_KEY.into(),
        },
        mcp: None,
    });
    let mut manifest = EnvManifest::new(selection, PlanCtx::default());
    manifest.catalog.manifests.push(MODULE_PATH.into());
    let mut draft = Draft::new(manifest);
    draft
        .modules
        .insert(MODULE_PATH.into(), module_yaml(&certs));
    let env = store.create_environment(&ws.id, "dev", draft).unwrap();
    store.commit_draft(&env.id).unwrap();
    let mut vars = VarSet::new();
    vars.set(
        ROUTE_KEY,
        VarValue::Secret(SecretRef {
            provider: "rotating".into(),
            key: "route".into(),
        }),
    );
    store.set_environment_vars(&env.id, &vars).unwrap();
    let id = store.create_instance(&env.id, 1).unwrap().id;

    let (v1, v2, v3) = (token("v1"), token("v2"), token("v3"));
    let secret = Rotating(Arc::new(Mutex::new(v1.clone())));
    let providers = SecretProviders::new()
        .with(secret.clone())
        .with(EnvProvider);
    // The host side: the endpoint, where the containers reach this host, on the plan's port.
    let endpoint = HostEndpoint::new(providers.clone());
    let reach = Reach::detect();
    let _listening = endpoint
        .listen(std::net::SocketAddr::new(reach.bind, port))
        .await
        .expect("the endpoint listens");
    let instances = Instances::new(
        Arc::clone(&store),
        olai_stack_runtime::DockerCli::new(),
        state.path(),
    )
    .with_providers(providers)
    .with_host_endpoint(Arc::new(endpoint.clone()) as Arc<dyn HostSecretEndpoint>)
    .with_wait_timeout(Duration::from_secs(180));

    let dir = instances.dir(&id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("compose.override.yaml"),
        format!(
            "services:\n  {EGRESS_SERVICE}:\n    volumes:\n      - {}:/etc/ssl/certs/ca-certificates.crt:ro\n",
            certs.join("ca.crt").display()
        ),
    )
    .unwrap();

    let project = instances.placement(&id).unwrap().project_name;
    let result = instances.start(&id).await;
    let check = || async {
        let started_with = result.as_ref().expect("the instance starts");
        assert_eq!(
            started_with
                .launch_var_names()
                .filter(|v| *v == ROUTE_KEY)
                .count(),
            0,
            "the route secret is a launch value"
        );
        assert_eq!(endpoint.instances(), std::slice::from_ref(&id));

        // 1. The upstream sees the route's credential, from the host.
        wait_for_token(&project, &v1, Duration::from_secs(60));
        eprintln!("step 1: the first value arrived");
        // 2. No container's environment holds it: the secret server's holds the token alone.
        assert!(containers_holding(&project, &v1).is_empty());
        let server_env = docker(&[
            "inspect",
            "--format",
            "{{json .Config.Env}}",
            &container(&project, SECRET_SERVER_SERVICE),
        ]);
        assert!(server_env.contains(&format!("{HOST_SOURCE_TOKEN_VAR}=")));
        assert!(
            !server_env.contains(ROUTE_KEY),
            "{ROUTE_KEY} in the server's env"
        );
        assert_no_value(&dir, &[&v1]);

        // 3. Rotation at the host: notified, it reaches Envoy with nothing recreated.
        let before: Vec<String> = [EGRESS_SERVICE, SECRET_SERVER_SERVICE]
            .iter()
            .map(|s| started(&project, s))
            .collect();
        *secret.0.lock().unwrap() = v2.clone();
        endpoint.notify(&id).await;
        let rotated = Instant::now();
        wait_for_token(&project, &v2, Duration::from_secs(30));
        eprintln!(
            "step 3: a notified rotation arrived in {:?}",
            rotated.elapsed()
        );
        // 4. Changed at the provider alone (no notify), it arrives when a hold ends.
        *secret.0.lock().unwrap() = v3.clone();
        wait_for_token(&project, &v3, Duration::from_secs(75));
        eprintln!(
            "step 4: a provider change arrived in {:?}",
            rotated.elapsed()
        );
        let after: Vec<String> = [EGRESS_SERVICE, SECRET_SERVER_SERVICE]
            .iter()
            .map(|s| started(&project, s))
            .collect();
        assert_eq!(before, after, "a container was recreated or restarted");
        for v in [&v1, &v2, &v3] {
            assert!(containers_holding(&project, v).is_empty());
        }
        assert_no_value(&dir, &[&v1, &v2, &v3]);
    };
    let checked = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(check())).await;
    if checked.is_err() {
        // The secret server never logs a value or the token, so its log is safe to show.
        let logs = Command::new("docker")
            .args(["compose", "-p", &project, "logs", SECRET_SERVER_SERVICE])
            .output()
            .unwrap();
        eprintln!(
            "secret-server logs:\n{}",
            String::from_utf8_lossy(&logs.stdout)
        );
    }
    instances.remove(&id, true).await.unwrap();
    // Stopped: the endpoint no longer serves it.
    assert!(endpoint.instances().is_empty());
    if let Err(panic) = checked {
        std::panic::resume_unwind(panic);
    }
}
