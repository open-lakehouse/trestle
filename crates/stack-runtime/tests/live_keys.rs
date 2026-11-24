//! A module that seals through the secret server's key service, on a real Docker daemon, from
//! an imported key through rotation, the rewrap sweep and retirement:
//! `cargo test -p olai-stack-runtime --features testing --test live_keys -- --ignored`.
//!
//! The module process is the `key_probe` example (see its docs): one `KeyServiceProvider` for
//! its whole life, over the socket and key name the planner gives it, behind an olai-store
//! `ManagedObjectStore`. The test drives it with `docker exec`. It builds the probe's image
//! from this checkout (`crates/stack-runtime/tests/key-probe.Dockerfile`) unless
//! `KEY_PROBE_IMAGE` names one; `~/.cargo/config.toml` is passed to the build as its
//! `cargo-config` secret when present. `SECRET_SERVER_IMAGE` overrides the catalog's secret
//! server image. The state directory sits under the build's tmp dir (inside the checkout),
//! because Docker VMs such as Colima only share the user's home directory with containers.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use olai_stack_runtime::{
    EnvProvider, Instances, Keyring, SecretError, SecretProvider, SecretProviders, SecretValue,
};
use olai_stack_state::{Draft, SecretRef, Store, VarSet, VarValue};
use olai_stack_topology::plan::egress::SECRET_SERVER_SERVICE;
use olai_stack_topology::plan::keyring_var;
use olai_stack_topology::{
    DataModule, EnvManifest, ModuleId, Placement, PlanCtx, Provides, RenderSpec, Role, Selection,
    ServiceSpec,
};

const MODULE: &str = "keys-it";
const MODULE_PATH: &str = "modules/keys-it/module.yaml";
const KEY: &str = "probe-kek";
const PROBE_IMAGE: &str = "trestle-key-probe:it";

/// The module's keyring, which the test rotates, as provider `rotating`.
#[derive(Clone)]
struct Rotating(Arc<Mutex<String>>);

impl SecretProvider for Rotating {
    fn name(&self) -> &str {
        "rotating"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        if key != "keyring" {
            return Err(SecretError::Missing {
                provider: "rotating".into(),
                key: key.into(),
            });
        }
        Ok(SecretValue::new(self.0.lock().unwrap().clone()))
    }
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

/// The probe image: `KEY_PROBE_IMAGE`, else built from this checkout.
fn probe_image() -> String {
    if let Ok(image) = std::env::var("KEY_PROBE_IMAGE") {
        return image;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = vec![
        "build".to_string(),
        "-f".into(),
        "crates/stack-runtime/tests/key-probe.Dockerfile".into(),
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
            keys: vec![KEY.into()],
            ..Provides::default()
        },
        knobs: vec![],
        // The probe gets its socket and key name from the planner; no keyring passes through.
        renders: RenderSpec {
            fragment: format!("services:\n  probe:\n    image: {image}\n"),
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

/// One probe command: `Ok(result)` for `ok <result>`, `Err(reason)` for `err <reason>`.
fn probe(project: &str, command: &str) -> Result<String, String> {
    let id = container(project, "probe");
    let mut args = vec!["exec", id.as_str(), "/key_probe", "ctl"];
    args.extend(command.split_whitespace());
    let out = Command::new("docker").args(&args).output().unwrap();
    let answer = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    match answer.split_once(' ') {
        Some(("ok", result)) => Ok(result.to_owned()),
        Some(("err", reason)) => Err(reason.to_owned()),
        _ => Err(format!(
            "no answer: {answer} {}",
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}

/// Retry `command` until `done` holds of its outcome.
fn wait_for(
    project: &str,
    command: &str,
    within: Duration,
    done: impl Fn(&Result<String, String>) -> bool,
) -> Result<String, String> {
    let deadline = Instant::now() + within;
    loop {
        let outcome = probe(project, command);
        if done(&outcome) || Instant::now() > deadline {
            return outcome;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn usage(project: &str) -> serde_json::Value {
    serde_json::from_str(&probe(project, "usage").expect("usage")).unwrap()
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
                    assert!(!text.contains(v), "key material is in {}", path.display());
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

/// The base64 of each version's key in `ring`, as the keyring's JSON holds them.
fn material(ring: &Keyring) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(ring.to_secret().expose()).unwrap();
    json["versions"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
#[ignore = "needs a Docker daemon and the envoy and secret-server images; builds the probe image"]
async fn a_module_seals_through_rotation_sweep_and_retirement() {
    let image = probe_image();
    let state = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let store = Arc::new(Store::open(&state.path().join("state.db")).unwrap());
    let ws = store.create_workspace("keysit", false).unwrap();
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
        keyring_var(&ModuleId::from(MODULE), KEY),
        VarValue::Secret(SecretRef {
            provider: "rotating".into(),
            key: "keyring".into(),
        }),
    );
    store.set_environment_vars(&env.id, &vars).unwrap();
    let id = store.create_instance(&env.id, 1).unwrap().id;

    // Version 1 is an existing key, imported with the id it had as a `LocalKeyProvider` KEK.
    let mut existing = [0u8; 32];
    getrandom::fill(&mut existing).unwrap();
    let existing = base64::engine::general_purpose::STANDARD.encode(existing);
    let mut ring = Keyring::import(&existing, Some("kek-1")).unwrap();
    let keyring = Rotating(Arc::new(Mutex::new(ring.to_secret().expose().to_owned())));
    let instances = Instances::new(
        Arc::clone(&store),
        olai_stack_runtime::DockerCli::new(),
        state.path(),
    )
    .with_providers(
        SecretProviders::new()
            .with(keyring.clone())
            .with(EnvProvider),
    )
    .with_wait_timeout(Duration::from_secs(180));
    let dir = instances.dir(&id);
    let project = instances.placement(&id).unwrap().project_name;
    let v1 = format!("{KEY}@v1");
    let v2 = format!("{KEY}@v2");

    let result = instances.start(&id).await.map(|_| ());
    let check = async {
        result.as_ref().expect("the instance starts");
        // 1. Sealed under v1; only the secret server's container environment holds the key.
        let active = wait_for(&project, "active", Duration::from_secs(60), Result::is_ok);
        assert_eq!(active.as_deref(), Ok(v1.as_str()));
        assert_eq!(probe(&project, "seal first one").as_deref(), Ok("first"));
        assert_eq!(probe(&project, "seal-raw r1").as_deref(), Ok(v1.as_str()));
        assert_eq!(
            usage(&project)["by_kek"],
            serde_json::json!({ v1.as_str(): 1 })
        );
        assert_eq!(
            containers_holding(&project, &existing),
            [SECRET_SERVER_SERVICE]
        );
        assert_no_value(&dir, &[&existing]);

        // 2. Rotation: a new version, and the secret server alone recreated. The probe keeps
        //    running, and its provider's watch picks the new active version up.
        let stored = ring.clone();
        assert_eq!(ring.rotate().unwrap(), 2);
        ring.check_extends(&stored).unwrap();
        *keyring.0.lock().unwrap() = ring.to_secret().expose().to_owned();
        let probe_id = container(&project, "probe");
        let probe_started = started_at(&probe_id);
        let server = container(&project, SECRET_SERVER_SERVICE);
        instances
            .recreate_service(&id, SECRET_SERVER_SERVICE)
            .await
            .expect("the secret server is recreated");
        let active = wait_for(&project, "active", Duration::from_secs(60), |o| {
            o.as_deref() == Ok(v2.as_str())
        });
        assert_eq!(active.as_deref(), Ok(v2.as_str()));
        assert_eq!(
            container(&project, "probe"),
            probe_id,
            "the probe was recreated"
        );
        assert_eq!(
            started_at(&probe_id),
            probe_started,
            "the probe was restarted"
        );
        assert_ne!(container(&project, SECRET_SERVER_SERVICE), server);
        assert_eq!(probe(&project, "seal second two").as_deref(), Ok("second"));
        assert_eq!(
            usage(&project)["by_kek"],
            serde_json::json!({ v1.as_str(): 1, v2.as_str(): 1 })
        );

        // 3. What v1 sealed still opens.
        assert_eq!(probe(&project, "open first").as_deref(), Ok("one"));
        assert_eq!(probe(&project, "open-raw r1").as_deref(), Ok("opened"));

        // 4. The sweep moves the store's v1 blob to v2.
        assert_eq!(probe(&project, "rewrap").as_deref(), Ok("1 0"));
        assert_eq!(
            usage(&project)["by_kek"],
            serde_json::json!({ v2.as_str(): 2 })
        );

        // 5. Retired below 2: the swept value still opens, the unswept v1 envelope no longer.
        let stored = ring.clone();
        assert!(ring.retire_below(2).unwrap());
        ring.check_extends(&stored).unwrap();
        *keyring.0.lock().unwrap() = ring.to_secret().expose().to_owned();
        instances
            .recreate_service(&id, SECRET_SERVER_SERVICE)
            .await
            .expect("the secret server is recreated");
        let refused = wait_for(&project, "open-raw r1", Duration::from_secs(60), |o| {
            o.as_ref().is_err_and(|e| !e.starts_with("no answer"))
        });
        assert!(refused.is_err(), "a v1 unwrap was not refused: {refused:?}");
        assert_eq!(probe(&project, "open first").as_deref(), Ok("one"));
        assert_eq!(probe(&project, "open second").as_deref(), Ok("two"));
        assert_eq!(probe(&project, "active").as_deref(), Ok(v2.as_str()));

        let keys = material(&ring);
        assert_eq!(keys.len(), 2, "retiring kept both versions");
        for key in &keys {
            assert_eq!(containers_holding(&project, key), [SECRET_SERVER_SERVICE]);
        }
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        assert_no_value(&dir, &keys);
    };
    // Run the checks, then always take the instance down.
    let checked = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(check)).await;
    instances.remove(&id, true).await.unwrap();
    if let Err(panic) = checked {
        std::panic::resume_unwind(panic);
    }
}
