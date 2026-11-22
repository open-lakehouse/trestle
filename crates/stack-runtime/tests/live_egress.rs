//! An instance with an egress route on a real Docker daemon: a consumer reaches a TLS upstream
//! in the same project through the egress gateway, which adds the route's bearer token, and a
//! rotation reaches the gateway by recreating the secret server alone:
//! `cargo test -p olai-stack-runtime --features testing --test live_egress -- --ignored`.
//!
//! The upstream's certificate is signed by a CA the test makes. The egress gateway trusts the
//! system store only, so a test-only `compose.override.yaml` in the instance directory mounts
//! that CA over it; nothing in the product renders it. The state directory sits under the
//! build's tmp dir (inside the checkout), because Docker VMs such as Colima only share the
//! user's home directory with containers.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use olai_stack_runtime::{
    EnvProvider, Instances, SecretError, SecretProvider, SecretProviders, SecretValue,
};
use olai_stack_state::{Draft, SecretRef, Store, VarSet, VarValue};
use olai_stack_topology::plan::egress::{EGRESS_SERVICE, SECRET_SERVER_SERVICE};
use olai_stack_topology::{
    DataModule, EgressRoute, EnvManifest, InjectedCredential, ModuleId, Placement, PlanCtx,
    Provides, RenderSpec, Role, Selection, ServiceSpec, egress_url_var,
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

#[tokio::test]
#[ignore = "needs a Docker daemon and the envoy, secret-server, echo and curl images"]
async fn a_route_injects_its_credential_and_rotates_without_restarting_the_gateway() {
    let state = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    // Outside the instance directory: the tree must hold nothing but what trestle renders
    // (and the override below).
    let certs = state.path().join("certs");
    write_certs(&certs);

    let store = Arc::new(Store::open(&state.path().join("state.db")).unwrap());
    let ws = store.create_workspace("egressit", false).unwrap();
    let mut selection = Selection::modules([MODULE]);
    selection.gateway.enabled = false;
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

    let v1 = token("v1");
    let v2 = token("v2");
    let secret = Rotating(Arc::new(Mutex::new(v1.clone())));
    let instances = Instances::new(
        Arc::clone(&store),
        olai_stack_runtime::DockerCli::new(),
        state.path(),
    )
    .with_providers(
        SecretProviders::new()
            .with(secret.clone())
            .with(EnvProvider),
    )
    .with_wait_timeout(Duration::from_secs(180));

    // Test-only: trust the test CA in the egress gateway. Compose reads the override next to
    // the head compose.yaml, and a render leaves files it didn't write alone.
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
    let result = async {
        instances.start(&id).await?;
        Ok::<_, olai_stack_runtime::InstanceError>(())
    }
    .await;
    let check = || async {
        result.as_ref().expect("the instance starts");
        // 1. The upstream sees the route's credential, in place of the caller's.
        wait_for_token(&project, &v1, Duration::from_secs(60));
        // 2. Only the secret server's container holds it.
        assert_eq!(containers_holding(&project, &v1), [SECRET_SERVER_SERVICE]);
        assert_no_value(&dir, &[&v1]);

        // 3. Rotation: a new value, and the secret server alone recreated.
        let envoy = container(&project, EGRESS_SERVICE);
        let server = container(&project, SECRET_SERVER_SERVICE);
        *secret.0.lock().unwrap() = v2.clone();
        instances
            .recreate_service(&id, SECRET_SERVER_SERVICE)
            .await
            .expect("the secret server is recreated");
        wait_for_token(&project, &v2, Duration::from_secs(60));
        assert_eq!(
            container(&project, EGRESS_SERVICE),
            envoy,
            "the egress gateway was restarted"
        );
        assert_ne!(container(&project, SECRET_SERVER_SERVICE), server);
        assert_eq!(containers_holding(&project, &v2), [SECRET_SERVER_SERVICE]);
        assert!(containers_holding(&project, &v1).is_empty());
        assert_no_value(&dir, &[&v1, &v2]);
    };
    // Run the checks, then always take the instance down.
    let checked = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(check())).await;
    instances.remove(&id, true).await.unwrap();
    if let Err(panic) = checked {
        std::panic::resume_unwind(panic);
    }
}
