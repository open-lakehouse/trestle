//! Two revisions of one environment side by side on a real Docker daemon, with every
//! credential passed as process environment:
//! `cargo test -p olai-stack-runtime --test live_instances -- --ignored`.
//!
//! The state directory sits under the build's tmp dir (inside the checkout), because Docker
//! VMs such as Colima only share the user's home directory with containers.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use olai_stack_runtime::{ComposeEngine, DockerCli, Instances};
use olai_stack_state::{Draft, Store, VarSet, VarValue};
use olai_stack_topology::{EnvManifest, PlanCtx, Selection};

#[tokio::test]
#[ignore = "needs a Docker daemon and the envoy, rustfs and postgres images"]
async fn two_revisions_run_side_by_side_on_docker() {
    let state = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let store = Arc::new(Store::open(&state.path().join("state.db")).unwrap());
    let ws = store.create_workspace("live", false).unwrap();
    // A credential of our own, so the run shows it is passed as process env.
    let mut vars = VarSet::new();
    vars.set(
        "LOCAL_POSTGRES_PASSWORD",
        VarValue::Literal("liveSENTINELpw42".into()),
    );
    store.set_workspace_vars(&ws.id, &vars).unwrap();
    let manifest = |modules: &[&str]| {
        Draft::new(EnvManifest::new(
            Selection::modules(modules.iter().copied()),
            PlanCtx::default(),
        ))
    };
    let env = store
        .create_environment(&ws.id, "dev", manifest(&["envoy", "rustfs"]))
        .unwrap();
    store.commit_draft(&env.id).unwrap();
    store
        .update_draft(&env.id, &manifest(&["envoy", "rustfs", "postgres"]))
        .unwrap();
    store.commit_draft(&env.id).unwrap();
    let r1 = store.create_instance(&env.id, 1).unwrap().id;
    let r2 = store.create_instance(&env.id, 2).unwrap().id;

    let docker = DockerCli::new();
    let instances = Instances::new(Arc::clone(&store), docker.clone(), state.path())
        .with_wait_timeout(Duration::from_secs(180));
    let result = async {
        let a = instances.start(&r1).await?;
        let b = instances.start(&r2).await?;
        Ok::<_, olai_stack_runtime::InstanceError>((a, b))
    }
    .await;
    let check = || async {
        let (a, b) = result.as_ref().expect("both instances start");
        let projects: BTreeMap<String, bool> = docker
            .ls()
            .await
            .unwrap()
            .into_iter()
            .map(|p| (p.name.clone(), p.is_running()))
            .collect();
        assert_eq!(projects.get("live_dev_r1"), Some(&true), "{projects:?}");
        assert_eq!(projects.get("live_dev_r2"), Some(&true), "{projects:?}");
        assert_ne!(a.plan.gateway_host_port(), b.plan.gateway_host_port());
        let health = instances.health(&r2).await.unwrap();
        assert_eq!(
            health.get("db"),
            Some(&olai_stack_runtime::Health::Healthy),
            "{health:?}"
        );
        // The credential reached the containers but no file.
        let mut stack = vec![b.dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(text) = std::fs::read_to_string(&path) {
                    assert!(!text.contains("liveSENTINELpw42"), "{}", path.display());
                }
            }
        }
    };
    check().await;
    for id in [&r1, &r2] {
        instances.remove(id, true).await.unwrap();
    }
    let left: Vec<String> = docker
        .ls()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .filter(|n| n.starts_with("live_"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
