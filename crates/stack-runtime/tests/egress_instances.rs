//! An instance with an egress route on a [`FakeEngine`]: the route's secret resolves like any
//! declared secret, reaches Compose only as launch environment that the `secret-server`
//! service alone references, never lands in the tree, and a rotation recreates
//! `secret-server` alone.
#![cfg(feature = "testing")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use olai_stack_runtime::{
    FakeCall, FakeEngine, Health, InstanceError, Instances, SecretError, SecretProvider,
    SecretProviders, SecretValue,
};
use olai_stack_state::{Draft, InstanceId, SecretRef, Store, VarSet, VarValue};
use olai_stack_topology::plan::egress::{EGRESS_SERVICE, SECRET_SERVER_SERVICE};
use olai_stack_topology::{
    EgressRoute, EnvManifest, InjectedCredential, ModuleId, PlanCtx, Selection,
};

/// The variable holding the route's credential.
const ROUTE_KEY: &str = "SEARCH_API_KEY";
const SENTINEL_V1: &str = "egressSENTINELv1-0123456789";
const SENTINEL_V2: &str = "egressSENTINELv2-9876543210";

/// One secret whose value a test can change, as rotating a keychain entry does, under the
/// provider name `rotating`.
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

/// Stands in for this process's environment (the `env` provider): empty.
struct NoEnv;

impl SecretProvider for NoEnv {
    fn name(&self) -> &str {
        olai_stack_runtime::ENV_PROVIDER
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        Err(SecretError::Missing {
            provider: "env".into(),
            key: key.into(),
        })
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    engine: FakeEngine,
    instances: Instances<FakeEngine>,
    secret: Rotating,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("state.db")).unwrap());
    let engine = FakeEngine::new();
    let secret = Rotating(Arc::new(Mutex::new(SENTINEL_V1.into())));
    let instances = Instances::new(Arc::clone(&store), engine.clone(), dir.path())
        .with_providers(SecretProviders::new().with(secret.clone()).with(NoEnv))
        .without_port_probe();
    Fixture {
        _dir: dir,
        store,
        engine,
        instances,
        secret,
    }
}

/// `demo/dev` at revision 1: envoy and rustfs, with one route to an outside upstream that
/// rustfs calls. Its credential is `ROUTE_KEY`, which nothing sets yet.
fn egress_instance(f: &Fixture) -> InstanceId {
    let ws = f.store.create_workspace("demo", false).unwrap();
    let mut selection = Selection::modules(["envoy", "rustfs"]);
    selection.egress.routes.push(EgressRoute {
        name: "search".into(),
        function: None,
        upstream: Some("https://api.search.example.com".into()),
        path: "/v1".into(),
        consumers: vec![ModuleId::from("rustfs")],
        credential: InjectedCredential::Bearer {
            secret: ROUTE_KEY.into(),
        },
        mcp: None,
    });
    let env = f
        .store
        .create_environment(
            &ws.id,
            "dev",
            Draft::new(EnvManifest::new(selection, PlanCtx::default())),
        )
        .unwrap();
    f.store.commit_draft(&env.id).unwrap();
    let id = f.store.create_instance(&env.id, 1).unwrap().id;
    f.engine.script(
        "demo_dev_r1",
        [
            (EGRESS_SERVICE, Health::Healthy),
            (SECRET_SERVER_SERVICE, Health::None),
            ("rustfs", Health::Healthy),
        ],
    );
    id
}

/// Set the route's credential on the environment, from the `rotating` provider.
fn set_route_key(f: &Fixture, id: &InstanceId) {
    let env = f.store.instance(id).unwrap().environment;
    let mut vars = VarSet::new();
    vars.set(
        ROUTE_KEY,
        VarValue::Secret(SecretRef {
            provider: "rotating".into(),
            key: "route".into(),
        }),
    );
    f.store.set_environment_vars(&env, &vars).unwrap();
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

/// No file under `dir` holds a sentinel value.
fn assert_no_value(dir: &Path) {
    let mut files = Vec::new();
    walk(dir, &mut files);
    assert!(!files.is_empty());
    for (path, text) in &files {
        for sentinel in [SENTINEL_V1, SENTINEL_V2] {
            assert!(!text.contains(sentinel), "{sentinel} in {path}");
        }
    }
}

/// The env of the last call matching `pick`.
fn last_env(engine: &FakeEngine, recreate: bool) -> BTreeMap<String, String> {
    engine
        .calls()
        .into_iter()
        .rev()
        .find_map(|c| match c {
            FakeCall::Up { env, .. } if !recreate => Some(env),
            FakeCall::Recreate { env, .. } if recreate => Some(env),
            _ => None,
        })
        .expect("the engine was called")
}

fn container_of(engine: &FakeEngine, service: &str) -> String {
    futures::executor::block_on(olai_stack_runtime::ComposeEngine::ps(engine, "demo_dev_r1"))
        .unwrap()
        .into_iter()
        .find(|s| s.service == service)
        .unwrap_or_else(|| panic!("no {service}"))
        .container
}

#[tokio::test]
async fn a_route_secret_set_nowhere_fails_the_start_before_compose() {
    let f = fixture();
    let id = egress_instance(&f);
    let err = f.instances.start(&id).await.unwrap_err();
    assert!(
        matches!(&err, InstanceError::MissingSecrets(names) if names == &[ROUTE_KEY.to_owned()]),
        "{err}"
    );
    assert!(f.engine.calls().is_empty(), "{:?}", f.engine.calls());
    assert!(f.engine.projects().is_empty());
}

#[tokio::test]
async fn a_route_secret_reaches_only_the_secret_server() {
    let f = fixture();
    let id = egress_instance(&f);
    set_route_key(&f, &id);
    let started = f.instances.start(&id).await.unwrap();

    // Compose gets the value in its process environment...
    assert_eq!(
        last_env(&f.engine, false)
            .get(ROUTE_KEY)
            .map(String::as_str),
        Some(SENTINEL_V1)
    );
    // ...and no file holds it.
    assert_no_value(&started.dir);

    // The secret server alone references the variable. The only other file naming it is the
    // server's own config, which maps an SDS name to it.
    let mut files = Vec::new();
    walk(&started.dir, &mut files);
    let dir = started.dir.display().to_string();
    let mut referenced_by = Vec::new();
    for (path, text) in &files {
        if !text.contains(ROUTE_KEY) {
            continue;
        }
        let relative = path.strip_prefix(&dir).unwrap().trim_start_matches('/');
        match relative {
            "modules/egress-gateway/secret-server.yaml" => {
                let config: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
                let listeners = config["listeners"].as_sequence().unwrap();
                assert_eq!(listeners.len(), 1);
                assert_eq!(listeners[0]["consumer"].as_str(), Some(EGRESS_SERVICE));
                assert_eq!(listeners[0]["secrets"][0]["env"].as_str(), Some(ROUTE_KEY));
            }
            "modules/egress-gateway/compose.yaml" => {
                let fragment: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
                for (name, service) in fragment["services"].as_mapping().unwrap() {
                    let service = serde_yaml::to_string(service).unwrap();
                    if service.contains(ROUTE_KEY) {
                        referenced_by.push(name.as_str().unwrap().to_owned());
                    }
                }
                // Listed by name only: Compose takes the value from the launch environment.
                let env = &fragment["services"][SECRET_SERVER_SERVICE]["environment"];
                assert!(env[ROUTE_KEY].is_null(), "{env:?}");
            }
            other => panic!("{other} names {ROUTE_KEY}"),
        }
    }
    assert_eq!(referenced_by, [SECRET_SERVER_SERVICE]);
}

#[tokio::test]
async fn a_rotation_recreates_the_secret_server_alone() {
    let f = fixture();
    let id = egress_instance(&f);
    set_route_key(&f, &id);

    // Not running yet: nothing to recreate.
    assert!(matches!(
        f.instances.recreate_service(&id, SECRET_SERVER_SERVICE).await,
        Err(InstanceError::NotRunning(i)) if i == id
    ));
    f.instances.start(&id).await.unwrap();
    let envoy = container_of(&f.engine, EGRESS_SERVICE);
    let server = container_of(&f.engine, SECRET_SERVER_SERVICE);

    *f.secret.0.lock().unwrap() = SENTINEL_V2.into();
    let recreated = f
        .instances
        .recreate_service(&id, SECRET_SERVER_SERVICE)
        .await
        .unwrap();
    let env = last_env(&f.engine, true);
    assert_eq!(env.get(ROUTE_KEY).map(String::as_str), Some(SENTINEL_V2));
    // The whole launch environment, since Compose interpolates every file.
    assert!(env.contains_key("COMPOSE_PROJECT_NAME"));
    assert!(matches!(
        f.engine.calls().last(),
        Some(FakeCall::Recreate { project, service, .. })
            if project == "demo_dev_r1" && service == SECRET_SERVER_SERVICE
    ));
    assert_ne!(container_of(&f.engine, SECRET_SERVER_SERVICE), server);
    assert_eq!(container_of(&f.engine, EGRESS_SERVICE), envoy);
    assert_no_value(&recreated.dir);

    // A service the plan doesn't run is refused before Compose.
    let calls = f.engine.calls().len();
    assert!(matches!(
        f.instances.recreate_service(&id, "no-such-service").await,
        Err(InstanceError::UnknownService { service }) if service == "no-such-service"
    ));
    assert_eq!(f.engine.calls().len(), calls);

    // With the secret gone, the recreate fails before Compose, as a start would.
    let env_id = f.store.instance(&id).unwrap().environment;
    f.store
        .set_environment_vars(&env_id, &VarSet::new())
        .unwrap();
    assert!(matches!(
        f.instances.recreate_service(&id, SECRET_SERVER_SERVICE).await,
        Err(InstanceError::MissingSecrets(names)) if names == [ROUTE_KEY.to_owned()]
    ));
    assert_eq!(f.engine.calls().len(), calls);
}
