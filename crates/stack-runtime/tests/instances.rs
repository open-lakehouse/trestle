//! Running store instances on a [`FakeEngine`]: revisions side by side, credentials kept out
//! of the tree, events and health, and reconciling after a crash.
#![cfg(feature = "testing")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use olai_stack_runtime::{
    ComposeEngine, EventAction, FakeCall, FakeEngine, Health, InstanceError, Instances, Layer,
    ReconcileOptions, SecretError, SecretProvider, SecretProviders, SecretValue, UserVars,
    instance_project,
};
use olai_stack_state::{Draft, InstanceId, SecretRef, Store, VarSet, VarValue};
use olai_stack_topology::{EnvManifest, PlanCtx, Selection, port_conflicts};

/// Secrets from a map, as a provider called `vault`.
struct Vault(BTreeMap<&'static str, &'static str>);

impl SecretProvider for Vault {
    fn name(&self) -> &str {
        "vault"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        self.0
            .get(key)
            .map(|v| SecretValue::new(*v))
            .ok_or_else(|| SecretError::Missing {
                provider: "vault".into(),
                key: key.into(),
            })
    }
}

const PG_SENTINEL: &str = "pgSENTINELvalue0123456789";
const S3_SENTINEL: &str = "s3SENTINELvalue0123456789";

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    engine: FakeEngine,
    instances: Instances<FakeEngine>,
    state: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().to_owned();
    let store = Arc::new(Store::open(&state.join("state.db")).unwrap());
    let engine = FakeEngine::new();
    let instances = instances(&store, &engine, &state);
    Fixture {
        _dir: dir,
        store,
        engine,
        instances,
        state,
    }
}

fn instances(store: &Arc<Store>, engine: &FakeEngine, state: &Path) -> Instances<FakeEngine> {
    Instances::new(Arc::clone(store), engine.clone(), state)
        .with_providers(
            SecretProviders::new().with(Vault([("pg", PG_SENTINEL), ("s3", S3_SENTINEL)].into())),
        )
        .with_source(UserVars::new(
            [("FROM_USER".to_owned(), "yes".to_owned())].into(),
        ))
        .without_port_probe()
}

/// `demo` (meshed) with `dev` at revisions 1 (envoy + rustfs) and 2 (+ postgres, mlflow).
fn two_revisions(f: &Fixture) -> (InstanceId, InstanceId) {
    let ws = f.store.create_workspace("demo", true).unwrap();
    let manifest = |modules: &[&str]| {
        EnvManifest::new(
            Selection::modules(modules.iter().copied()),
            PlanCtx::default(),
        )
    };
    let env = f
        .store
        .create_environment(&ws.id, "dev", Draft::new(manifest(&["envoy", "rustfs"])))
        .unwrap();
    f.store.commit_draft(&env.id).unwrap();
    f.store
        .update_draft(
            &env.id,
            &Draft::new(manifest(&["envoy", "rustfs", "postgres", "mlflow"])),
        )
        .unwrap();
    f.store.commit_draft(&env.id).unwrap();
    let r1 = f.store.create_instance(&env.id, 1).unwrap().id;
    let r2 = f.store.create_instance(&env.id, 2).unwrap().id;
    (r1, r2)
}

fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
            out.push((path.display().to_string(), text));
        }
    }
}

#[tokio::test]
async fn two_revisions_run_side_by_side() {
    let f = fixture();
    let (r1, r2) = two_revisions(&f);
    let a = f.instances.start(&r1).await.unwrap();
    let b = f.instances.start(&r2).await.unwrap();

    assert_eq!(
        f.engine.projects(),
        ["demo_dev_r1".to_owned(), "demo_dev_r2".to_owned()].into()
    );
    let (pa, pb) = (
        f.instances.placement(&r1).unwrap(),
        f.instances.placement(&r2).unwrap(),
    );
    assert_ne!(pa.slot, pb.slot);
    assert_eq!(pa.host_label, "demo-dev-r1");
    assert_eq!(pb.host_label, "demo-dev-r2");
    assert_eq!(pa.mesh_network.as_deref(), Some("trestle-demo-mesh"));
    assert!(port_conflicts([("r1", &a.plan), ("r2", &b.plan)]).is_empty());
    assert_eq!(a.plan.gateway_host_port(), Some(9080));
    assert_eq!(b.plan.gateway_host_port(), Some(10080));

    // Each tree is its own, under the state directory.
    assert!(a.dir.starts_with(f.state.join("instances")));
    let head = std::fs::read_to_string(b.dir.join("compose.yaml")).unwrap();
    assert!(head.contains("name: demo_dev_r2"), "{head}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&a.dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    // A second instance of a running revision would share its project.
    let env = f.store.instance(&r1).unwrap().environment;
    let again = f.store.create_instance(&env, 1).unwrap();
    assert!(matches!(
        f.instances.start(&again.id).await,
        Err(InstanceError::RevisionRunning { revision: 1, .. })
    ));

    f.instances.stop(&r1, false).await.unwrap();
    assert_eq!(f.engine.projects(), ["demo_dev_r2".to_owned()].into());
    // The slot stays leased while the instance exists, so its ports don't move.
    assert_eq!(f.instances.placement(&r1).unwrap().slot, pa.slot);
    f.instances.remove(&r2, true).await.unwrap();
    assert!(f.engine.projects().is_empty());
    assert!(!b.dir.exists());
}

#[tokio::test]
async fn a_sentinel_secret_never_reaches_the_tree() {
    let f = fixture();
    let (_, r2) = two_revisions(&f);
    // The Postgres and RustFS credentials come from the vault, through the workspace and
    // the instance's overrides.
    let ws = f.store.workspace("demo").unwrap().unwrap();
    let mut vars = VarSet::new();
    vars.set(
        "LOCAL_POSTGRES_PASSWORD",
        VarValue::Secret(SecretRef {
            provider: "vault".into(),
            key: "pg".into(),
        }),
    );
    f.store.set_workspace_vars(&ws.id, &vars).unwrap();
    let mut overrides = VarSet::new();
    overrides.set(
        "LOCAL_RUSTFS_SECRET_KEY",
        VarValue::Secret(SecretRef {
            provider: "vault".into(),
            key: "s3".into(),
        }),
    );
    f.store.set_overrides(&r2, &overrides).unwrap();

    let started = f.instances.start(&r2).await.unwrap();
    let mut files = Vec::new();
    walk(&started.dir, &mut files);
    assert!(!files.is_empty());
    for (path, text) in &files {
        for sentinel in [PG_SENTINEL, S3_SENTINEL] {
            assert!(!text.contains(sentinel), "{sentinel} in {path}");
        }
    }
    // They reached Compose through its environment instead.
    let env = f
        .engine
        .calls()
        .into_iter()
        .find_map(|c| match c {
            FakeCall::Up { env, .. } => Some(env),
            _ => None,
        })
        .unwrap();
    for sentinel in [PG_SENTINEL, S3_SENTINEL] {
        assert!(
            env.values().any(|v| v.contains(sentinel)),
            "{sentinel} not passed"
        );
    }
    // And the effective environment masks them.
    let effective = f.instances.effective_env(&r2).await.unwrap();
    let masked = effective.masked();
    let pg = &masked["LOCAL_POSTGRES_PASSWORD"];
    assert!(pg.secret);
    assert_eq!(pg.value, olai_stack_runtime::MASK);
    assert_eq!(pg.provenance, Layer::WorkspaceVars);
    assert_eq!(pg.shadowed, [Layer::CatalogDefaults]);
    assert!(!format!("{effective:?}").contains(PG_SENTINEL));
}

#[tokio::test]
async fn an_unknown_secret_provider_fails_without_fallback() {
    let f = fixture();
    let (r1, _) = two_revisions(&f);
    let mut overrides = VarSet::new();
    overrides.set(
        "LOCAL_POSTGRES_PASSWORD",
        VarValue::Secret(SecretRef {
            provider: "keychain".into(),
            key: "pg".into(),
        }),
    );
    f.store.set_overrides(&r1, &overrides).unwrap();
    assert!(matches!(
        f.instances.start(&r1).await,
        Err(InstanceError::Secret(SecretError::UnknownProvider(p))) if p == "keychain"
    ));
    assert!(f.engine.projects().is_empty());
}

#[tokio::test]
async fn sources_overlay_the_store_and_placement_keys_stay_put() {
    let f = fixture();
    let (r1, _) = two_revisions(&f);
    f.store.set_sources(&r1, &["user".to_owned()]).unwrap();
    let mut overrides = VarSet::new();
    overrides.set("COMPOSE_PROJECT_NAME", VarValue::Literal("hijacked".into()));
    f.store.set_overrides(&r1, &overrides).unwrap();
    let env = f.instances.effective_env(&r1).await.unwrap();
    let vars = env.vars();
    assert_eq!(vars["FROM_USER"].provenance, Layer::Source("user".into()));
    let project = &vars["COMPOSE_PROJECT_NAME"];
    assert_eq!(project.value.expose(), "demo_dev_r1");
    assert_eq!(project.provenance, Layer::Placement);
    assert_eq!(project.shadowed, [Layer::InstanceOverrides]);

    // A source the instance names but nobody configured is an error.
    f.store.set_sources(&r1, &["neon".to_owned()]).unwrap();
    assert!(matches!(
        f.instances.effective_env(&r1).await,
        Err(InstanceError::Source(_))
    ));
}

#[tokio::test]
async fn health_and_events_follow_the_engine() {
    let f = fixture();
    let (r1, _) = two_revisions(&f);
    f.engine.script(
        "demo_dev_r1",
        [("envoy", Health::Healthy), ("rustfs", Health::Starting)],
    );
    let mut events = f.instances.events(&r1).await.unwrap();
    f.instances.start(&r1).await.unwrap();
    assert_eq!(
        f.instances.health(&r1).await.unwrap(),
        [
            ("envoy".to_owned(), Health::Healthy),
            ("rustfs".to_owned(), Health::Starting)
        ]
        .into()
    );
    f.engine
        .set_health("demo_dev_r1", "rustfs", Health::Healthy);
    let mut saw_health = false;
    while let Some(Ok(event)) = events.next().await {
        assert_eq!(event.instance, r1);
        if event.action == EventAction::Health(Health::Healthy) {
            assert_eq!(event.service, "rustfs");
            saw_health = true;
            break;
        }
    }
    assert!(saw_health);
}

#[tokio::test]
async fn a_killed_runtime_reconciles_on_restart() {
    let f = fixture();
    let (r1, r2) = two_revisions(&f);
    f.instances.start(&r1).await.unwrap();
    f.instances.start(&r2).await.unwrap();
    // While no runtime ran: r2's project went away, and someone left an instance-shaped
    // project behind that the store doesn't know.
    f.engine
        .down(
            &f.instances.dir(&r2),
            &olai_stack_runtime::DownOptions::default(),
        )
        .await
        .unwrap();
    f.engine.adopt(&instance_project("demo", "ghost", 1));
    f.engine.adopt("someone-elses-project");
    let ghost = tempfile::tempdir().unwrap();
    std::fs::write(ghost.path().join("compose.yaml"), "name: demo_ghost_r2\n").unwrap();
    f.engine
        .up(ghost.path(), &olai_stack_runtime::UpOptions::default())
        .await
        .unwrap();
    // The store still says r1 runs but forgot it (a crash mid-start before recording).
    f.store
        .set_desired(&r1, olai_stack_state::Desired::Stopped)
        .unwrap();

    // A fresh runtime on the same store and Docker.
    let restarted = instances(&f.store, &f.engine, &f.state);
    let report = restarted
        .reconcile(ReconcileOptions {
            remove_orphans: true,
        })
        .await
        .unwrap();
    assert_eq!(report.adopted, std::slice::from_ref(&r1));
    assert_eq!(report.stale, std::slice::from_ref(&r2));
    assert_eq!(report.orphans, ["demo_ghost_r1", "demo_ghost_r2"]);
    // Only the orphan whose files Compose knows of can be taken down.
    assert_eq!(report.removed, ["demo_ghost_r2"]);
    assert_eq!(
        f.store.instance(&r1).unwrap().desired,
        olai_stack_state::Desired::Running
    );
    assert_eq!(
        f.store.instance(&r2).unwrap().desired,
        olai_stack_state::Desired::Stopped
    );
    assert!(f.engine.projects().contains("someone-elses-project"));
    assert!(!f.engine.projects().contains("demo_ghost_r2"));
}
