//! An instance with an egress route on a [`FakeEngine`]: the route's secret resolves like any
//! declared secret, reaches Compose only as launch environment that the `secret-server`
//! service alone references, never lands in the tree, and a rotation recreates
//! `secret-server` alone. The same holds for a secret a module reads over SDS, and for the
//! keyring of a key a module seals under.
#![cfg(feature = "testing")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use olai_stack_runtime::{
    FakeCall, FakeEngine, Health, InstanceError, Instances, Keyring, SecretError, SecretProvider,
    SecretProviders, SecretValue,
};
use olai_stack_state::{Draft, InstanceId, SecretRef, Store, VarSet, VarValue};
use olai_stack_topology::plan::egress::{EGRESS_SERVICE, SECRET_SERVER_SERVICE};
use olai_stack_topology::{
    DataModule, DeclaredSecret, EgressRoute, EnvManifest, InjectedCredential, ModuleId, Placement,
    PlanCtx, Provides, RenderSpec, Role, SdsSecret, Selection, ServiceSpec,
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

/// The variable holding the secret the `sds-app` module reads over SDS.
const SDS_KEY: &str = "SDS_APP_TOKEN";
const SDS_MODULE: &str = "sds-app";
const SDS_MODULE_PATH: &str = "modules/sds-app/module.yaml";

/// `sds-app`: one `app` container that reads its token over SDS, so the fragment never passes
/// `SDS_KEY` through.
fn sds_module_yaml() -> String {
    let module = DataModule {
        id: ModuleId::from(SDS_MODULE),
        model: None,
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "app".into(),
            role: Role::new("app"),
            placement: Placement::Container {
                service: "app".into(),
            },
            endpoints: vec![],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides: Provides {
            secrets: vec![DeclaredSecret::required(
                SDS_KEY,
                "The app's upstream token.",
            )],
            sds_secrets: vec![SdsSecret::new("app-token", SDS_KEY)],
            ..Provides::default()
        },
        knobs: vec![],
        renders: RenderSpec {
            fragment: "services:\n  app:\n    image: example/app:1\n".into(),
            files: vec![],
        }
        .for_compose(),
    };
    serde_yaml::to_string(&module).unwrap()
}

/// `demo/dev` at revision 1: `sds-app` alone, its token set from the `rotating` provider.
fn sds_instance(f: &Fixture) -> InstanceId {
    let ws = f.store.create_workspace("demo", false).unwrap();
    let mut selection = Selection::modules([SDS_MODULE]);
    selection.gateway.enabled = false;
    let mut manifest = EnvManifest::new(selection, PlanCtx::default());
    manifest.catalog.manifests.push(SDS_MODULE_PATH.into());
    let mut draft = Draft::new(manifest);
    draft
        .modules
        .insert(SDS_MODULE_PATH.into(), sds_module_yaml());
    let env = f.store.create_environment(&ws.id, "dev", draft).unwrap();
    f.store.commit_draft(&env.id).unwrap();
    let mut vars = VarSet::new();
    vars.set(
        SDS_KEY,
        VarValue::Secret(SecretRef {
            provider: "rotating".into(),
            key: "route".into(),
        }),
    );
    f.store.set_environment_vars(&env.id, &vars).unwrap();
    let id = f.store.create_instance(&env.id, 1).unwrap().id;
    f.engine.script(
        "demo_dev_r1",
        [
            (EGRESS_SERVICE, Health::Healthy),
            (SECRET_SERVER_SERVICE, Health::None),
            ("app", Health::None),
        ],
    );
    id
}

#[tokio::test]
async fn a_module_sds_secret_reaches_only_the_secret_server() {
    let f = fixture();
    let id = sds_instance(&f);
    let started = f.instances.start(&id).await.unwrap();
    assert_eq!(
        last_env(&f.engine, false).get(SDS_KEY).map(String::as_str),
        Some(SENTINEL_V1)
    );
    assert_no_value(&started.dir);

    let mut files = Vec::new();
    walk(&started.dir, &mut files);
    let dir = started.dir.display().to_string();
    let mut referenced_by = Vec::new();
    let mut mounted = false;
    for (path, text) in &files {
        let relative = path.strip_prefix(&dir).unwrap().trim_start_matches('/');
        if relative == "modules/egress-gateway/consumers/sds-app.yaml" {
            // The module's container gets its socket, not the variable.
            let overlay: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
            let app = &overlay["services"]["app"];
            assert_eq!(
                app["volumes"][0].as_str(),
                Some("sds-sds-app:/run/trestle-sds")
            );
            mounted = true;
        }
        if !text.contains(SDS_KEY) {
            continue;
        }
        match relative {
            "modules/egress-gateway/secret-server.yaml" => {
                let config: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
                let listener = config["listeners"]
                    .as_sequence()
                    .unwrap()
                    .iter()
                    .find(|l| l["consumer"].as_str() == Some(SDS_MODULE))
                    .expect("a listener for the module");
                assert_eq!(listener["secrets"][0]["name"].as_str(), Some("app-token"));
                assert_eq!(listener["secrets"][0]["env"].as_str(), Some(SDS_KEY));
            }
            "modules/egress-gateway/compose.yaml" => {
                let fragment: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
                for (name, service) in fragment["services"].as_mapping().unwrap() {
                    if serde_yaml::to_string(service).unwrap().contains(SDS_KEY) {
                        referenced_by.push(name.as_str().unwrap().to_owned());
                    }
                }
            }
            other => panic!("{other} names {SDS_KEY}"),
        }
    }
    assert!(mounted, "no consumer overlay for the module");
    assert_eq!(referenced_by, [SECRET_SERVER_SERVICE]);
}

#[tokio::test]
async fn a_rotation_of_a_module_sds_secret_recreates_the_secret_server_alone() {
    let f = fixture();
    let id = sds_instance(&f);
    f.instances.start(&id).await.unwrap();
    let app = container_of(&f.engine, "app");
    let server = container_of(&f.engine, SECRET_SERVER_SERVICE);

    *f.secret.0.lock().unwrap() = SENTINEL_V2.into();
    let recreated = f
        .instances
        .recreate_service(&id, SECRET_SERVER_SERVICE)
        .await
        .unwrap();
    let env = last_env(&f.engine, true);
    assert_eq!(env.get(SDS_KEY).map(String::as_str), Some(SENTINEL_V2));
    assert_ne!(container_of(&f.engine, SECRET_SERVER_SERVICE), server);
    assert_eq!(container_of(&f.engine, "app"), app);
    assert_no_value(&recreated.dir);
}

const KEYS_MODULE: &str = "keys-app";
const KEYS_MODULE_PATH: &str = "modules/keys-app/module.yaml";
/// The variable the planner derives for `keys-app`'s key `server-kek`.
const KEYRING_VAR: &str = "TRESTLE_KEYRING_KEYS_APP__SERVER_KEK";
/// The 32 bytes of each keyring version: their text, and so their base64, are sentinels.
const KEY_V1: &[u8; 32] = b"keyringSENTINELv1-0123456789abcd";
const KEY_V2: &[u8; 32] = b"keyringSENTINELv2-9876543210dcba";

/// `keys-app`: one `app` container that seals under the key `server-kek` through the key
/// service. It declares no secret: the keyring's variable is the planner's.
fn keys_module_yaml() -> String {
    let mut module: DataModule = serde_yaml::from_str(&sds_module_yaml()).unwrap();
    module.id = ModuleId::from(KEYS_MODULE);
    module.provides = Provides {
        keys: vec!["server-kek".into()],
        ..Provides::default()
    };
    serde_yaml::to_string(&module).unwrap()
}

fn keyring(versions: &[&[u8; 32]]) -> Keyring {
    use base64::Engine;
    let encode = |k: &[u8; 32]| base64::engine::general_purpose::STANDARD.encode(k);
    let mut ring = Keyring::import(&encode(versions[0]), Some("kek-1")).unwrap();
    for _ in &versions[1..] {
        ring.rotate().unwrap();
    }
    // A rotation draws random material; rebuild the JSON with the sentinel keys instead.
    let mut json: serde_json::Value = serde_json::from_str(ring.to_secret().expose()).unwrap();
    for (i, k) in versions.iter().enumerate() {
        json["versions"][(i + 1).to_string()] = encode(k).into();
    }
    Keyring::from_json(&json.to_string()).unwrap()
}

/// `demo/dev` at revision 1: `keys-app` alone, its keyring set from the `rotating` provider.
fn keys_instance(f: &Fixture) -> InstanceId {
    *f.secret.0.lock().unwrap() = keyring(&[KEY_V1]).to_secret().expose().to_owned();
    let ws = f.store.create_workspace("demo", false).unwrap();
    let mut selection = Selection::modules([KEYS_MODULE]);
    selection.gateway.enabled = false;
    let mut manifest = EnvManifest::new(selection, PlanCtx::default());
    manifest.catalog.manifests.push(KEYS_MODULE_PATH.into());
    let mut draft = Draft::new(manifest);
    draft
        .modules
        .insert(KEYS_MODULE_PATH.into(), keys_module_yaml());
    let env = f.store.create_environment(&ws.id, "dev", draft).unwrap();
    f.store.commit_draft(&env.id).unwrap();
    let mut vars = VarSet::new();
    vars.set(
        KEYRING_VAR,
        VarValue::Secret(SecretRef {
            provider: "rotating".into(),
            key: "route".into(),
        }),
    );
    f.store.set_environment_vars(&env.id, &vars).unwrap();
    let id = f.store.create_instance(&env.id, 1).unwrap().id;
    f.engine.script(
        "demo_dev_r1",
        [
            (EGRESS_SERVICE, Health::Healthy),
            (SECRET_SERVER_SERVICE, Health::None),
            ("app", Health::None),
        ],
    );
    id
}

/// No file under `dir` holds a key's bytes, its base64, or the keyring's JSON.
fn assert_no_key_material(dir: &Path) {
    use base64::Engine;
    let mut files = Vec::new();
    walk(dir, &mut files);
    assert!(!files.is_empty());
    for (path, text) in &files {
        for key in [KEY_V1, KEY_V2] {
            let encoded = base64::engine::general_purpose::STANDARD.encode(key);
            assert!(!text.contains(&encoded), "a key's base64 in {path}");
            assert!(
                !text.contains(std::str::from_utf8(key).unwrap()),
                "a key in {path}"
            );
        }
        assert!(!text.contains("\"versions\""), "a keyring in {path}");
    }
}

#[tokio::test]
async fn a_keyring_with_no_value_fails_the_start_before_compose() {
    let f = fixture();
    let id = keys_instance(&f);
    let env = f.store.instance(&id).unwrap().environment;
    f.store.set_environment_vars(&env, &VarSet::new()).unwrap();
    let err = f.instances.start(&id).await.unwrap_err();
    assert!(
        matches!(&err, InstanceError::MissingSecrets(names) if names == &[KEYRING_VAR.to_owned()]),
        "{err}"
    );
    assert!(f.engine.calls().is_empty());
}

/// The keyring reaches Compose only as launch environment, which the secret server alone
/// references; the module gets its socket and the key's name, and no file holds material.
#[tokio::test]
async fn a_keyring_reaches_only_the_secret_server() {
    let f = fixture();
    let id = keys_instance(&f);
    let started = f.instances.start(&id).await.unwrap();
    let launched = last_env(&f.engine, false);
    let ring = Keyring::from_json(&launched[KEYRING_VAR]).unwrap();
    assert_eq!(ring, keyring(&[KEY_V1]));
    assert_no_key_material(&started.dir);

    let mut files = Vec::new();
    walk(&started.dir, &mut files);
    let dir = started.dir.display().to_string();
    let mut referenced_by = Vec::new();
    for (path, text) in &files {
        let relative = path.strip_prefix(&dir).unwrap().trim_start_matches('/');
        if relative == "modules/egress-gateway/consumers/keys-app.yaml" {
            let overlay: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
            let app = &overlay["services"]["app"];
            assert_eq!(
                app["volumes"][0].as_str(),
                Some("sds-keys-app:/run/trestle-sds")
            );
            assert_eq!(
                app["environment"]["TRESTLE_KEY_SERVER_KEK"].as_str(),
                Some("server-kek")
            );
        }
        if !text.contains(KEYRING_VAR) {
            continue;
        }
        match relative {
            "modules/egress-gateway/secret-server.yaml" => {
                let config: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
                let listener = config["listeners"]
                    .as_sequence()
                    .unwrap()
                    .iter()
                    .find(|l| l["consumer"].as_str() == Some(KEYS_MODULE))
                    .expect("a listener for the module");
                assert_eq!(listener["keys"][0]["name"].as_str(), Some("server-kek"));
                assert_eq!(listener["keys"][0]["env"].as_str(), Some(KEYRING_VAR));
            }
            "modules/egress-gateway/compose.yaml" => {
                let fragment: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
                for (name, service) in fragment["services"].as_mapping().unwrap() {
                    if serde_yaml::to_string(service)
                        .unwrap()
                        .contains(KEYRING_VAR)
                    {
                        referenced_by.push(name.as_str().unwrap().to_owned());
                    }
                }
            }
            other => panic!("{other} names {KEYRING_VAR}"),
        }
    }
    assert_eq!(referenced_by, [SECRET_SERVER_SERVICE]);
}

/// A rotated keyring reaches a recreated secret server; the module keeps running.
#[tokio::test]
async fn a_keyring_rotation_recreates_the_secret_server_alone() {
    let f = fixture();
    let id = keys_instance(&f);
    f.instances.start(&id).await.unwrap();
    let app = container_of(&f.engine, "app");
    let server = container_of(&f.engine, SECRET_SERVER_SERVICE);

    let rotated = keyring(&[KEY_V1, KEY_V2]);
    rotated.check_extends(&keyring(&[KEY_V1])).unwrap();
    *f.secret.0.lock().unwrap() = rotated.to_secret().expose().to_owned();
    let recreated = f
        .instances
        .recreate_service(&id, SECRET_SERVER_SERVICE)
        .await
        .unwrap();
    let env = last_env(&f.engine, true);
    assert_eq!(Keyring::from_json(&env[KEYRING_VAR]).unwrap(), rotated);
    assert_eq!(rotated.active(), 2);
    assert_ne!(container_of(&f.engine, SECRET_SERVER_SERVICE), server);
    assert_eq!(container_of(&f.engine, "app"), app);
    assert_no_key_material(&recreated.dir);
}
