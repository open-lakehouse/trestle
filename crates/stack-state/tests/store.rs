//! The store's records, revisions, leases and schema guard.

use std::collections::BTreeSet;
use std::sync::Arc;

use olai_stack_state::{
    Desired, Draft, Error, SCHEMA_VERSION, SecretRef, SourceBinding, Store, VarScope, VarSet,
    VarValue,
};
use olai_stack_topology::{EnvManifest, MAX_SLOT, PlanCtx, Selection};

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    (dir, store)
}

fn draft(modules: &[&str]) -> Draft {
    Draft::new(EnvManifest::new(
        Selection::modules(modules.iter().copied()),
        PlanCtx::default(),
    ))
}

#[test]
fn drafts_commit_to_numbered_revisions() {
    let (_dir, store) = store();
    let ws = store.create_workspace("demo", true).unwrap();
    let env = store
        .create_environment(&ws.id, "dev", draft(&["unity-catalog"]))
        .unwrap();
    assert_eq!(env.head, None);
    assert_eq!(env.draft.manifest.context.env_name, "dev");

    let r1 = store.commit_draft(&env.id).unwrap();
    assert_eq!(r1.number(), 1);

    let mut next = draft(&["unity-catalog", "mlflow"]);
    next.vars
        .set("LOG_LEVEL", VarValue::Literal("debug".into()));
    store.update_draft(&env.id, &next).unwrap();
    let r2 = store.commit_draft(&env.id).unwrap();
    assert_eq!(r2.number(), 2);
    assert_ne!(r1.digest(), r2.digest());

    // Revision 1 is as it was committed; the draft and head moved on.
    assert_eq!(store.revision(&env.id, 1).unwrap(), r1);
    assert_eq!(r1.manifest().selection.modules.len(), 1);
    assert_eq!(store.environment(&env.id).unwrap().head, Some(2));
    assert_eq!(store.revisions(&env.id).unwrap().len(), 2);

    // The same content digests the same.
    store.update_draft(&env.id, &next).unwrap();
    assert_eq!(store.commit_draft(&env.id).unwrap().digest(), r2.digest());
}

#[test]
fn a_revision_cant_be_updated_even_bypassing_the_api() {
    let (dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    store.commit_draft(&env.id).unwrap();

    let raw = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
    let err = raw
        .execute("UPDATE revision SET manifest = 'tampered'", [])
        .unwrap_err();
    assert!(err.to_string().contains("revisions are immutable"), "{err}");
}

#[test]
fn names_are_validated_and_unique() {
    let (_dir, store) = store();
    assert!(matches!(
        store.create_workspace("Demo", false),
        Err(Error::InvalidName(_))
    ));
    let ws = store.create_workspace("demo", false).unwrap();
    assert!(matches!(
        store.create_workspace("demo", false),
        Err(Error::Duplicate {
            kind: "workspace",
            ..
        })
    ));
    store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    assert!(matches!(
        store.create_environment(&ws.id, "dev", draft(&[])),
        Err(Error::Duplicate {
            kind: "environment",
            ..
        })
    ));
    // Another workspace may reuse the name.
    let other = store.create_workspace("other", false).unwrap();
    store
        .create_environment(&other.id, "dev", draft(&[]))
        .unwrap();
}

#[test]
fn vars_hold_literals_and_secret_references() {
    let (_dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let mut vars = VarSet::new();
    vars.set("REGION", VarValue::Literal("eu-west-1".into()));
    vars.set(
        "PG_PASSWORD",
        VarValue::Secret(SecretRef {
            provider: "keychain".into(),
            key: "demo/pg".into(),
        }),
    );
    store.set_workspace_vars(&ws.id, &vars).unwrap();
    assert_eq!(store.workspace("demo").unwrap().unwrap().vars, vars);
    // Debug never prints a value.
    let debug = format!("{vars:?}");
    assert!(!debug.contains("eu-west-1"), "{debug}");
}

#[test]
fn instances_lease_host_wide_slots() {
    let (_dir, store) = store();
    let mut instances = Vec::new();
    for ws_name in ["alpha", "beta"] {
        let ws = store.create_workspace(ws_name, false).unwrap();
        let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
        let rev = store.commit_draft(&env.id).unwrap();
        instances.push(store.create_instance(&env.id, rev.number()).unwrap());
    }
    // Both workspaces have a `dev`; they still get different slots.
    let a = store.lease_slot(&instances[0].id).unwrap();
    let b = store.lease_slot(&instances[1].id).unwrap();
    assert_eq!((a.slot, b.slot), (0, 1));
    // Leasing again returns the held slot.
    assert_eq!(store.lease_slot(&instances[0].id).unwrap(), a);
    assert_eq!(store.instance(&instances[1].id).unwrap().slot, Some(1));

    // A released slot is handed out again, lowest first.
    store.release_slot(&instances[0].id).unwrap();
    assert_eq!(store.instance(&instances[0].id).unwrap().slot, None);
    let env = instances[0].environment.clone();
    let third = store.create_instance(&env, 1).unwrap();
    assert_eq!(store.lease_slot(&third.id).unwrap().slot, 0);

    // Deleting an instance frees its slot.
    store.delete_instance(&instances[1].id).unwrap();
    assert!(store.leases().unwrap().iter().all(|l| l.slot != 1));

    store.set_desired(&third.id, Desired::Running).unwrap();
    assert_eq!(store.instance(&third.id).unwrap().desired, Desired::Running);
}

#[test]
fn an_instance_needs_an_existing_revision() {
    let (_dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    assert!(matches!(
        store.create_instance(&env.id, 1),
        Err(Error::NotFound {
            kind: "revision",
            ..
        })
    ));
}

#[test]
fn concurrent_leases_never_share_a_slot_and_run_out_cleanly() {
    let (dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    store.commit_draft(&env.id).unwrap();
    let total = usize::from(MAX_SLOT) + 1 + 8;
    let ids: Vec<_> = (0..total)
        .map(|_| store.create_instance(&env.id, 1).unwrap().id)
        .collect();
    let path = Arc::new(dir.path().join("state.db"));

    // Eight threads, each on its own connection, race for slots.
    let chunks: Vec<Vec<_>> = ids.chunks(total.div_ceil(8)).map(<[_]>::to_vec).collect();
    let handles: Vec<_> = chunks
        .into_iter()
        .map(|chunk| {
            let path = Arc::clone(&path);
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                chunk
                    .iter()
                    .map(|id| store.lease_slot(id))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    let slots: Vec<u16> = results
        .iter()
        .filter_map(|r| r.as_ref().ok().map(|l| l.slot))
        .collect();
    let distinct: BTreeSet<u16> = slots.iter().copied().collect();
    assert_eq!(slots.len(), usize::from(MAX_SLOT) + 1);
    assert_eq!(distinct.len(), slots.len(), "a slot was leased twice");
    let exhausted = results
        .iter()
        .filter(|r| matches!(r, Err(Error::SlotsExhausted)))
        .count();
    assert_eq!(exhausted, 8);
}

#[test]
fn a_newer_schema_is_refused() {
    let (dir, store) = store();
    drop(store);
    let path = dir.path().join("state.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(raw);
    assert!(matches!(
        Store::open(&path),
        Err(Error::UnsupportedVersion { found, supported })
            if found == SCHEMA_VERSION + 1 && supported == SCHEMA_VERSION
    ));
}

#[test]
fn reopening_keeps_the_data() {
    let (dir, store) = store();
    store.create_workspace("demo", false).unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    assert_eq!(store.workspaces().unwrap().len(), 1);
}

#[test]
fn trestle_home_overrides_the_data_dir() {
    // Only this test touches the variable in this binary.
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: no other thread in this test binary reads or writes TRESTLE_HOME.
    unsafe { std::env::set_var("TRESTLE_HOME", dir.path()) };
    assert_eq!(olai_stack_state::trestle_home().unwrap(), dir.path());
    let store = Store::open_default().unwrap();
    assert_eq!(store.path(), dir.path().join("state.db"));
    unsafe { std::env::remove_var("TRESTLE_HOME") };
}

#[test]
fn source_bindings_and_workspace_source_config_round_trip() {
    let (_dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    assert!(ws.sources.is_empty());
    let config = serde_json::json!({"project_id": "p-1"});
    store
        .set_workspace_source(&ws.id, "neon", Some(config.clone()))
        .unwrap();
    assert_eq!(
        store.workspace("demo").unwrap().unwrap().sources["neon"],
        config
    );
    store.set_workspace_source(&ws.id, "neon", None).unwrap();
    assert!(store.workspace("demo").unwrap().unwrap().sources.is_empty());

    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    store.commit_draft(&env.id).unwrap();
    let instance = store.create_instance(&env.id, 1).unwrap();
    let bindings = [
        SourceBinding::new("user"),
        SourceBinding::new("neon").with_config(serde_json::json!({"branch": "br-1"})),
    ];
    store.set_sources(&instance.id, &bindings).unwrap();
    assert_eq!(store.instance(&instance.id).unwrap().sources, bindings);

    let missing = olai_stack_state::WorkspaceId("nope".into());
    assert!(matches!(
        store.set_workspace_source(&missing, "neon", None),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn vars_change_in_place_per_scope() {
    let (_dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    for (scope, key) in [
        (VarScope::Workspace(ws.id.clone()), "WS"),
        (VarScope::Environment(env.id.clone()), "ENV"),
        (VarScope::Draft(env.id.clone()), "DRAFT"),
    ] {
        store
            .modify_vars(&scope, |vars| {
                vars.set(key, VarValue::Literal("1".into()));
                vars.set("GONE", VarValue::Literal("x".into()));
                Ok::<_, Error>(())
            })
            .unwrap();
        let written = store
            .modify_vars(&scope, |vars| {
                assert!(vars.remove("GONE").is_some());
                Ok::<_, Error>(())
            })
            .unwrap();
        assert_eq!(written.0.keys().collect::<Vec<_>>(), [key]);
    }
    assert!(
        store
            .workspace("demo")
            .unwrap()
            .unwrap()
            .vars
            .get("WS")
            .is_some()
    );
    let env = store.environment(&env.id).unwrap();
    assert!(env.vars.get("ENV").is_some());
    assert!(env.draft.vars.get("DRAFT").is_some());
    // The draft's vars go into the next revision; the environment's don't.
    let r1 = store.commit_draft(&env.id).unwrap();
    assert!(r1.vars().get("DRAFT").is_some() && r1.vars().get("ENV").is_none());

    // A failing change writes nothing.
    let scope = VarScope::Workspace(ws.id.clone());
    let failed = store.modify_vars(&scope, |vars| {
        vars.0.clear();
        Err(Error::Export("no".into()))
    });
    assert!(failed.is_err());
    assert!(
        store
            .workspace("demo")
            .unwrap()
            .unwrap()
            .vars
            .get("WS")
            .is_some()
    );
}

#[test]
fn modify_draft_keeps_the_name_and_concurrent_edits() {
    let (dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let env = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    let written = store
        .modify_draft(&env.id, |d| {
            d.manifest.context.env_name = "renamed".into();
            d.manifest.add_module("mlflow");
            Ok::<_, Error>(())
        })
        .unwrap();
    assert_eq!(written.manifest.context.env_name, "dev");
    assert_eq!(store.environment(&env.id).unwrap().draft, written);

    // Two writers each adding vars never lose the other's.
    let path = dir.path().join("state.db");
    let threads: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|who| {
            let (path, id) = (path.clone(), env.id.clone());
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                for n in 0..25 {
                    store
                        .modify_draft(&id, |d| {
                            d.vars
                                .set(format!("{who}{n}"), VarValue::Literal(n.to_string()));
                            Ok::<_, Error>(())
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(store.environment(&env.id).unwrap().draft.vars.0.len(), 50);
}

#[test]
fn deleting_an_environment_removes_its_revisions_instances_and_leases() {
    let (_dir, store) = store();
    let ws = store.create_workspace("demo", false).unwrap();
    let dev = store.create_environment(&ws.id, "dev", draft(&[])).unwrap();
    let prod = store
        .create_environment(&ws.id, "prod", draft(&[]))
        .unwrap();
    for env in [&dev, &prod] {
        store.commit_draft(&env.id).unwrap();
        let i = store.create_instance(&env.id, 1).unwrap();
        store.lease_slot(&i.id).unwrap();
    }
    assert_eq!(store.instances_of(&dev.id).unwrap().len(), 1);

    store.delete_environment(&dev.id).unwrap();
    assert!(matches!(
        store.environment(&dev.id),
        Err(Error::NotFound { .. })
    ));
    assert!(store.instances_of(&dev.id).unwrap().is_empty());
    assert_eq!(store.instances().unwrap().len(), 1);
    assert_eq!(store.leases().unwrap().len(), 1);
    assert!(matches!(
        store.delete_environment(&dev.id),
        Err(Error::NotFound { .. })
    ));
}
