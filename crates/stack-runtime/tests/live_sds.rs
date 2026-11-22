//! A module that reads its bearer token over SDS, on a real Docker daemon: the token reaches
//! its container over the module's own socket and never through its environment, and a
//! rotation reaches it by recreating the secret server alone:
//! `cargo test -p olai-stack-runtime --features testing --test live_sds -- --ignored`.
//!
//! The consumer is olai-http's `sds_probe` example, which logs the SHA-256 of each value it
//! reads (never the value). The test builds its image from this checkout
//! (`crates/olai-http/tests/sds-probe.Dockerfile`) unless `SDS_PROBE_IMAGE` names one. Where
//! crates.io is only reachable through a mirror, `~/.cargo/config.toml` is passed to the build
//! as its `cargo-config` secret. `SECRET_SERVER_IMAGE` overrides the catalog's secret server
//! image (to test one built from this checkout). The state directory sits under the build's
//! tmp dir (inside the checkout), because Docker VMs such as Colima only share the user's home
//! directory with containers.

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
    DataModule, DeclaredSecret, EnvManifest, ModuleId, Placement, PlanCtx, Provides, RenderSpec,
    Role, SdsSecret, Selection, ServiceSpec,
};

const MODULE: &str = "sds-it";
const MODULE_PATH: &str = "modules/sds-it/module.yaml";
const TOKEN_VAR: &str = "SDS_IT_TOKEN";
const TOKEN_NAME: &str = "probe-token";
const PROBE_IMAGE: &str = "trestle-sds-probe:it";

/// The probe's token, which the test rotates, as provider `rotating`.
#[derive(Clone)]
struct Rotating(Arc<Mutex<String>>);

impl SecretProvider for Rotating {
    fn name(&self) -> &str {
        "rotating"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        if key != "token" {
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

fn sha256(value: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, value.as_bytes());
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
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

/// The probe image: `SDS_PROBE_IMAGE`, else built from this checkout.
fn probe_image() -> String {
    if let Ok(image) = std::env::var("SDS_PROBE_IMAGE") {
        return image;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = vec![
        "build".to_string(),
        "-f".into(),
        "crates/olai-http/tests/sds-probe.Dockerfile".into(),
        "-t".into(),
        PROBE_IMAGE.into(),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let config = Path::new(&home).join(".cargo/config.toml");
        if config.exists() {
            args.push("--secret".into());
            args.push(format!("id=cargo-config,src={}", config.display()));
        }
    }
    args.push(".".into());
    let status = Command::new("docker")
        .args(&args)
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success(), "building the probe image failed");
    PROBE_IMAGE.into()
}

fn module_yaml(image: &str) -> String {
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
        service_specs: vec![ServiceSpec {
            name: "probe".into(),
            role: Role::new("probe"),
            placement: Placement::Container {
                service: "probe".into(),
            },
            endpoints: vec![],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides: Provides {
            secrets: vec![DeclaredSecret::required(TOKEN_VAR, "The probe's token.")],
            sds_secrets: vec![SdsSecret::new(TOKEN_NAME, TOKEN_VAR)],
            ..Provides::default()
        },
        knobs: vec![],
        // The token reaches the probe over SDS only: the fragment never passes it through.
        renders: RenderSpec {
            fragment: format!(
                "services:\n  probe:\n    image: {image}\n    environment:\n      SDS_NAME: {TOKEN_NAME}\n"
            ),
            files: vec![],
        }
        .for_compose(),
    };
    serde_yaml::to_string(&module).unwrap()
}

/// The container id of `service` in `project`.
fn container(project: &str, service: &str) -> String {
    let id = docker(&["compose", "-p", project, "ps", "-q", service]);
    assert!(!id.is_empty(), "{service} has no container");
    id
}

fn started_at(id: &str) -> String {
    docker(&["inspect", "--format", "{{.State.StartedAt}}", id])
}

/// Wait until the probe has logged `value`'s digest.
fn wait_for_digest(project: &str, value: &str, within: Duration) {
    let want = format!("sds-probe sha256={}", sha256(value));
    let probe = container(project, "probe");
    let deadline = Instant::now() + within;
    let mut logs = String::new();
    while Instant::now() < deadline {
        let out = Command::new("docker")
            .args(["logs", &probe])
            .output()
            .unwrap();
        logs = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!logs.contains(value), "the probe logged a secret value");
        if logs.lines().any(|l| l.trim() == want) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("the probe never read the expected value; its log:\n{logs}");
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
#[ignore = "needs a Docker daemon and the envoy and secret-server images; builds the probe image"]
async fn a_module_reads_its_token_over_sds_and_rotates_without_restarting() {
    let image = probe_image();
    let state = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let store = Arc::new(Store::open(&state.path().join("state.db")).unwrap());
    let ws = store.create_workspace("sdsit", false).unwrap();
    let mut selection = Selection::modules([MODULE]);
    selection.gateway.enabled = false;
    if let Ok(server) = std::env::var("SECRET_SERVER_IMAGE") {
        selection.knob_overrides.insert(
            ModuleId::from("egress-gateway"),
            [("secret_server_image".to_string(), server)].into(),
        );
    }
    let mut manifest = EnvManifest::new(selection, PlanCtx::default());
    manifest.catalog.manifests.push(MODULE_PATH.into());
    let mut draft = Draft::new(manifest);
    draft
        .modules
        .insert(MODULE_PATH.into(), module_yaml(&image));
    let env = store.create_environment(&ws.id, "dev", draft).unwrap();
    store.commit_draft(&env.id).unwrap();
    let mut vars = VarSet::new();
    vars.set(
        TOKEN_VAR,
        VarValue::Secret(SecretRef {
            provider: "rotating".into(),
            key: "token".into(),
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
    let dir = instances.dir(&id);
    let project = instances.placement(&id).unwrap().project_name;

    let result = instances.start(&id).await.map(|_| ());
    let check = || async {
        result.as_ref().expect("the instance starts");
        // 1. The probe reads the token over its socket; only the secret server's container
        //    environment holds it.
        wait_for_digest(&project, &v1, Duration::from_secs(60));
        assert_eq!(containers_holding(&project, &v1), [SECRET_SERVER_SERVICE]);
        assert_no_value(&dir, &[&v1]);

        // 2. Rotation: a new value, and the secret server alone recreated. The probe keeps
        //    running and picks the new value up from its watch.
        let probe = container(&project, "probe");
        let probe_started = started_at(&probe);
        let envoy = container(&project, EGRESS_SERVICE);
        let server = container(&project, SECRET_SERVER_SERVICE);
        *secret.0.lock().unwrap() = v2.clone();
        let recreating = Instant::now();
        instances
            .recreate_service(&id, SECRET_SERVER_SERVICE)
            .await
            .expect("the secret server is recreated");
        eprintln!(
            "recreating the secret server took {:?}",
            recreating.elapsed()
        );
        wait_for_digest(&project, &v2, Duration::from_secs(60));
        assert_eq!(
            container(&project, "probe"),
            probe,
            "the probe was recreated"
        );
        assert_eq!(started_at(&probe), probe_started, "the probe was restarted");
        assert_eq!(container(&project, EGRESS_SERVICE), envoy);
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
