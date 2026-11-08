//! Neon branches as a source: the CLI's output parsed (a stub `neon` replays the spike's
//! fixtures), binding against `FakeNeon`, the overlay, ownership, and an instance of the
//! `neon-hybrid` scenario run on a bound branch.
//!
//! The stub's branch hosts a second database and a second role, as a branch that already runs
//! other services does; the CLI then picks no default and needs both named.
#![cfg(all(feature = "neon", feature = "testing"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use olai_stack_runtime::neon::{
    FakeNeon, NEON_SOURCE, NeonBinding, NeonBranchSource, NeonCall, NeonCli, NeonControl,
    NeonError, StorageCredentialRef, bind, create_owned_branch, delete_owned_branch, expires_in,
    reset_owned_branch, unbind,
};
use olai_stack_runtime::{
    FakeCall, FakeEngine, Instances, Layer, SecretError, SecretProvider, SecretProviders,
    SecretValue, Source, SourceCtx,
};
use olai_stack_state::{Draft, InstanceId, SecretRef, SourceBinding, Store, read_env_dir};
use olai_stack_topology::{FunctionRequirement, ModuleId, NeonRequirements};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/neon/8.16.0")
}

/// A `neon` that answers from the fixtures and logs its arguments to `log`.
fn stub_neon(dir: &Path, fail_auth: bool) -> (NeonCli, PathBuf) {
    let fx = fixtures();
    let log = dir.join("calls.log");
    let script = dir.join("neon");
    let body = if fail_auth {
        "echo 'ERROR: Unauthorized. Run neon auth' >&2; exit 1".to_owned()
    } else {
        format!(
            r#"case "$*" in
  "me "*) cat "{fx}/me.json" ;;
  api\ */storage*) cat "{fx}/api-storage.json" ;;
  "api "*) cat "{fx}/api-branches.json" ;;
  "branches create"*) cat "{fx}/branches-create.json" ;;
  "branches delete"*) cat "{fx}/branches-delete.json" ;;
  "branches reset"*) cat "{fx}/branches-reset.json" ;;
  connection-string*--database-name*) cat "{fx}/cs-extended.json" ;;
  connection-string*) echo 'ERROR: Multiple databases found for the branch, please provide one with the --database-name option: neondb, unitycatalog' >&2; exit 1 ;;
  "functions list"*) cat "{fx}/functions-list.json" ;;
  "buckets list"*) cat "{fx}/buckets-list.json" ;;
  "buckets create"*) cat "{fx}/buckets-create.json" ;;
  "credentials create"*) cat "{fx}/credentials-create.json" ;;
  "credentials revoke"*) ;;
  "databases list"*) cat "{fx}/databases-list.json" ;;
  "databases create"*--owner-name*) echo '{{}}' ;;
  "databases create"*) echo 'ERROR: More than one role found in branch br-autumn-mouse-baubmqk8. Please specify the owner name. Roles: neondb_owner, authz_reader' >&2; exit 1 ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac"#,
            fx = fx.display()
        )
    };
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho \"$*\" >> \"{}\"\n{body}\n", log.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (NeonCli::with_program(script), log)
}

#[cfg(unix)]
#[tokio::test]
async fn the_cli_output_parses_for_every_verb() {
    let dir = tempfile::tempdir().unwrap();
    let (neon, log) = stub_neon(dir.path(), false);
    let p = "orange-truth-58809040";

    assert_eq!(neon.auth_status().await.unwrap(), "<personal-4>");
    // Only the API lists annotations, so branches come from it.
    let branches = neon.branches(p).await.unwrap();
    let spike = branches.iter().find(|b| b.name == "spike-a").unwrap();
    assert!(spike.is_owned());
    assert_eq!(spike.annotations["trestle.instance"], "spike");

    let created = create_owned_branch(
        &neon,
        p,
        "spike-a",
        Some("main"),
        Some("2026-11-03T01:36:57Z".into()),
        Some("inst-1"),
    )
    .await
    .unwrap();
    assert_eq!(created.id, "br-autumn-mouse-baubmqk8");
    assert!(created.is_owned());

    // `--extended` is JSON; the password stays out of Debug.
    let conn = neon.connection(p, "spike-a", None, false).await.unwrap();
    assert_eq!(
        conn.host,
        "ep-snowy-mud-bar7hnwg.c-8.eu-central-1.aws.neon.tech"
    );
    assert_eq!(conn.role, "neondb_owner");
    assert!(!format!("{conn:?}").contains("<password-1>"));

    let functions = neon.functions(p, "main").await.unwrap();
    assert_eq!(functions[0].slug, "spikefn");
    let storage = neon.storage(p, "main").await.unwrap();
    assert!(storage.enabled && storage.force_path_style);
    assert_eq!(neon.buckets(p, "main").await.unwrap(), ["mlflow-artifacts"]);
    // A sentence, not JSON: fine.
    neon.create_bucket(p, "main", "x").await.unwrap();
    // JSON followed by a WARNING line.
    let credential = neon.create_credential(p, "main", "c").await.unwrap();
    assert_eq!(
        credential.token_id,
        "nak_live_00000000000000000000000000000001"
    );
    assert!(!format!("{credential:?}").contains("<s3-secret"));
    neon.revoke_credential(p, "main", &credential.token_id)
        .await
        .unwrap();
    assert_eq!(
        neon.databases(p, "main").await.unwrap(),
        ["neondb", "unitycatalog"]
    );
    // Owned by the main database's owner, which has to be named next to a second role.
    neon.create_database(p, "main", "mlflow").await.unwrap();
    neon.reset_branch(p, &created.id).await.unwrap();
    neon.delete_branch(p, &created.id).await.unwrap();

    let calls = std::fs::read_to_string(&log).unwrap();
    let create = calls
        .lines()
        .find(|l| l.starts_with("branches create"))
        .unwrap();
    let named = |verb: &str, arg: &str| {
        let line = calls.lines().find(|l| l.starts_with(verb)).unwrap();
        assert!(line.contains(arg), "{arg} in {line}");
    };
    named("connection-string", "--database-name neondb");
    named("databases create", "--owner-name neondb_owner");
    for arg in [
        "--no-secrets",
        "--parent main",
        "--expires-at 2026-11-03T01:36:57Z",
        "\"trestle.owner\":\"trestle\"",
        "--output json",
    ] {
        assert!(create.contains(arg), "{arg} in {create}");
    }
    assert!(
        calls.lines().all(|l| l.ends_with("--output json")),
        "{calls}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn signed_out_and_missing_cli_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let (neon, _) = stub_neon(dir.path(), true);
    assert!(matches!(
        neon.auth_status().await,
        Err(NeonError::NotAuthenticated(_))
    ));
    let missing = NeonCli::with_program(dir.path().join("no-such-neon"));
    assert!(matches!(
        missing.auth_status().await,
        Err(NeonError::NotInstalled)
    ));
}

/// Secrets from a map, as the keychain provider.
#[derive(Clone, Default)]
struct Keychain(Arc<std::sync::Mutex<BTreeMap<String, String>>>);

impl SecretProvider for Keychain {
    fn name(&self) -> &str {
        "keychain"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        self.0
            .lock()
            .unwrap()
            .get(key)
            .map(|v| SecretValue::new(v.clone()))
            .ok_or_else(|| SecretError::Missing {
                provider: "keychain".into(),
                key: key.into(),
            })
    }
}

/// What the `neon-hybrid` scenario needs of a branch.
fn requirements() -> NeonRequirements {
    let function = |module: &str, slug: &str| FunctionRequirement {
        module: ModuleId::from(module),
        slug: slug.into(),
        url_var: Some(format!("{}_FUNCTION_URL", slug.to_uppercase())),
        env: Default::default(),
        secrets: vec![],
    };
    NeonRequirements {
        databases: vec!["mlflow".into(), "omnigent".into()],
        buckets: vec!["mlflow".into()],
        functions: vec![
            function("headwaters", "headwaters"),
            function("unity-catalog", "unitycatalog"),
        ],
        vars: vec!["NEON_S3_ENDPOINT".into(), "PGHOST".into()],
    }
}

/// Bind to `branch`, keeping the credential's secret in `keychain` as the CLI would.
async fn bind_to(neon: &FakeNeon, branch: &str, keychain: &Keychain) -> NeonBinding {
    let (mut binding, credential) = bind(neon, "proj", branch, &requirements(), "trestle-inst")
        .await
        .unwrap();
    let credential = credential.expect("the plan uses storage");
    let key = format!("neon/{}", credential.token_id);
    keychain
        .0
        .lock()
        .unwrap()
        .insert(key.clone(), credential.secret_access_key.expose().into());
    binding.storage_credential = Some(StorageCredentialRef {
        token_id: credential.token_id,
        secret: SecretRef {
            provider: "keychain".into(),
            key,
        },
    });
    binding
}

#[tokio::test]
async fn binding_checks_functions_and_mints_a_credential_on_the_child() {
    let neon = FakeNeon::new("proj");
    // Nothing deployed yet: binding fails up front, naming both functions.
    let err = bind(&neon, "proj", "main", &requirements(), "c")
        .await
        .unwrap_err();
    let NeonError::MissingFunctions { slugs, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(slugs, &["headwaters", "unitycatalog"]);
    assert!(err.to_string().contains("deploy it on the branch"), "{err}");

    neon.deploy_function("main", "unitycatalog");
    neon.deploy_function("main", "headwaters");
    let parent_credential = neon
        .create_credential("proj", "main", "parent")
        .await
        .unwrap();
    let child = create_owned_branch(
        &neon,
        "proj",
        "dev",
        Some("main"),
        Some(expires_in(std::time::Duration::from_secs(3600))),
        Some("inst"),
    )
    .await
    .unwrap();
    assert!(child.is_owned() && child.expires_at.is_some());
    // The child inherits the parent's credentials; binding still mints one of its own.
    assert!(
        neon.credentials("dev")
            .contains(&parent_credential.token_id)
    );

    let keychain = Keychain::default();
    let binding = bind_to(&neon, "dev", &keychain).await;
    let minted = binding
        .storage_credential
        .as_ref()
        .unwrap()
        .token_id
        .clone();
    assert_ne!(minted, parent_credential.token_id);
    let calls = neon.calls();
    for call in [
        NeonCall::CreateDatabase(child.id.clone(), "mlflow".into()),
        NeonCall::CreateDatabase(child.id.clone(), "omnigent".into()),
        NeonCall::CreateBucket(child.id.clone(), "mlflow".into()),
        NeonCall::CreateCredential(child.id.clone(), minted.clone()),
    ] {
        assert!(calls.contains(&call), "{call:?} in {calls:?}");
    }
    // Binding again creates nothing twice.
    bind(&neon, "proj", "dev", &requirements(), "again")
        .await
        .unwrap();
    let databases = neon
        .calls()
        .iter()
        .filter(|c| matches!(c, NeonCall::CreateDatabase(..)))
        .count();
    assert_eq!(databases, 2);

    unbind(&neon, &binding).await.unwrap();
    assert!(!neon.credentials("dev").contains(&minted));
}

fn ctx(binding: &NeonBinding) -> SourceCtx {
    SourceCtx {
        instance: InstanceId("inst".into()),
        workspace: "demo".into(),
        environment: "dev".into(),
        revision: 1,
        config: serde_json::to_value(binding).unwrap(),
    }
}

#[tokio::test]
async fn the_overlay_has_postgres_storage_and_function_urls() {
    let neon = FakeNeon::new("proj");
    neon.deploy_function("main", "unitycatalog");
    neon.deploy_function("main", "headwaters");
    let keychain = Keychain::default();
    let binding = bind_to(&neon, "main", &keychain).await;
    let source = NeonBranchSource::new(
        Arc::new(neon.clone()),
        SecretProviders::new().with(keychain.clone()),
    );
    assert_eq!(source.name(), NEON_SOURCE);
    let overlay = source.overlay(&ctx(&binding)).await.unwrap();

    assert_eq!(
        overlay.vars["PGHOST"],
        "ep-main.c-8.eu-central-1.aws.neon.tech"
    );
    assert_eq!(overlay.vars["PGUSER"], "neondb_owner");
    assert_eq!(overlay.vars["NEON_S3_REGION"], "eu-central-1");
    assert!(
        overlay.credentials["DATABASE_URL"]
            .expose()
            .contains("ep-main-pooler.")
    );
    assert!(
        !overlay.credentials["DATABASE_URL_UNPOOLED"]
            .expose()
            .contains("-pooler")
    );
    assert_eq!(
        overlay.endpoints["UNITYCATALOG_FUNCTION_URL"],
        "https://br-main-unitycatalog.compute.fake.neon.tech"
    );
    assert_eq!(
        overlay.endpoints["NEON_S3_ENDPOINT"],
        "https://br-main.storage.c-8.eu-central-1.aws.neon.tech"
    );
    let secret = keychain.0.lock().unwrap().values().next().unwrap().clone();
    assert_eq!(
        overlay.credentials["NEON_S3_SECRET_ACCESS_KEY"].expose(),
        secret
    );
    // Credentials never show in Debug.
    let debug = format!("{overlay:?}");
    assert!(
        !debug.contains(&secret) && !debug.contains("fake-password"),
        "{debug}"
    );

    // A function gone since binding is an error that names it.
    let mut stale = binding.clone();
    stale
        .functions
        .insert("rivers".into(), "RIVERS_FUNCTION_URL".into());
    let err = source.overlay(&ctx(&stale)).await.unwrap_err();
    assert!(err.reason.contains("`rivers`"), "{err}");
    // An instance bound to nothing is told so.
    let err = source
        .overlay(&SourceCtx {
            config: serde_json::Value::Null,
            ..ctx(&binding)
        })
        .await
        .unwrap_err();
    assert!(err.reason.contains("binding"), "{err}");
}

#[tokio::test]
async fn only_owned_branches_are_reset_or_deleted() {
    let neon = FakeNeon::from_fixtures("proj", &fixtures()).unwrap();
    // The spike's `main` wasn't made by trestle; its `spike-a` was.
    let err = delete_owned_branch(&neon, "proj", "main")
        .await
        .unwrap_err();
    assert!(matches!(err, NeonError::NotOwned(_)), "{err:?}");
    assert!(matches!(
        reset_owned_branch(&neon, "proj", "main").await,
        Err(NeonError::NotOwned(_))
    ));
    let functions = neon.functions("proj", "main").await.unwrap();
    assert_eq!(functions[0].slug, "spikefn");

    let child = create_owned_branch(&neon, "proj", "mine", Some("main"), None, None)
        .await
        .unwrap();
    reset_owned_branch(&neon, "proj", "mine").await.unwrap();
    delete_owned_branch(&neon, "proj", &child.id).await.unwrap();
    assert!(
        neon.branches("proj")
            .await
            .unwrap()
            .iter()
            .all(|b| b.name != "mine")
    );
    assert!(matches!(
        delete_owned_branch(&neon, "proj", "mine").await,
        Err(NeonError::NoSuchBranch(_))
    ));
}

#[tokio::test]
async fn a_hybrid_instance_runs_on_its_branch() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("state.db")).unwrap());
    let ws = store.create_workspace("demo", false).unwrap();
    let scenario = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/scenarios/neon-hybrid");
    let mut draft: Draft = read_env_dir(&scenario).unwrap();
    draft.manifest.context = Default::default();
    let env = store.create_environment(&ws.id, "dev", draft).unwrap();
    store.commit_draft(&env.id).unwrap();
    let instance = store.create_instance(&env.id, 1).unwrap().id;

    let neon = FakeNeon::new("proj");
    neon.deploy_function("main", "unitycatalog");
    neon.deploy_function("main", "headwaters");
    let keychain = Keychain::default();
    let binding = bind_to(&neon, "main", &keychain).await;
    store
        .set_sources(
            &instance,
            &[
                SourceBinding::new(NEON_SOURCE)
                    .with_config(serde_json::to_value(&binding).unwrap()),
            ],
        )
        .unwrap();

    let engine = FakeEngine::new();
    let providers = SecretProviders::new().with(keychain.clone());
    let instances = Instances::new(Arc::clone(&store), engine.clone(), dir.path())
        .with_providers(providers.clone())
        .with_source(NeonBranchSource::new(Arc::new(neon.clone()), providers))
        .without_port_probe();

    let effective = instances.effective_env(&instance).await.unwrap();
    let vars = effective.vars();
    assert_eq!(vars["PGHOST"].provenance, Layer::Source(NEON_SOURCE.into()));
    assert_eq!(
        vars["UNITY_CATALOG_URL"].value.expose(),
        "https://br-main-unitycatalog.compute.fake.neon.tech/api/2.1/unity-catalog"
    );
    assert!(vars["NEON_S3_SECRET_ACCESS_KEY"].value.is_secret());

    let started = instances.start(&instance).await.unwrap();
    // Nothing of the functions runs locally, and no credential lands in the tree.
    let compose = std::fs::read_to_string(started.dir.join("compose.yaml")).unwrap();
    assert!(!compose.contains("unity-catalog") && !compose.contains("headwaters"));
    let secret = keychain.0.lock().unwrap().values().next().unwrap().clone();
    for entry in walkdir(&started.dir) {
        let text = std::fs::read_to_string(&entry).unwrap_or_default();
        assert!(!text.contains(&secret), "{}", entry.display());
        assert!(!text.contains("fake-password"), "{}", entry.display());
    }
    let Some(FakeCall::Up { env, .. }) = engine.calls().into_iter().next() else {
        panic!("no up");
    };
    assert_eq!(env["NEON_S3_SECRET_ACCESS_KEY"], secret);

    // Neon down (signed out): the instance still stops.
    neon.set_signed_in(false);
    instances.stop(&instance, false).await.unwrap();
    assert!(engine.projects().is_empty());
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walkdir(&path));
        } else {
            out.push(path);
        }
    }
    out
}
