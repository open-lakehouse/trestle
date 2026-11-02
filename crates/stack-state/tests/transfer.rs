//! Import and export: a workspace directory round-trips through the store, modulo slots.

use std::path::Path;

use olai_stack_state::{Store, read_env_dir};
use olai_stack_topology::{EnvManifest, PlanCtx, Selection, WorkspaceManifest};

const PROBE: &str =
    "id: probe\nmodel: probe\nprovider_of: probe_role\nrenders:\n  compose:\n    fragment: \"\"\n";

/// A workspace directory as `trestle workspace` writes it: `demo` with a mesh, `dev` on slot 0
/// with a module manifest of its own, `prod` on slot 1 as a version-4 member (placement in its
/// `[context]`).
fn workspace_dir(root: &Path) {
    let mut ws = WorkspaceManifest::new("demo", true).unwrap();
    ws.add_env("dev").unwrap();
    ws.add_env("prod").unwrap();
    ws.write_to(&root.join("trestle-workspace.toml")).unwrap();

    let mut dev = EnvManifest::new(
        Selection::modules(["unity-catalog", "probe"]),
        PlanCtx {
            env_name: "dev".into(),
            ..PlanCtx::default()
        },
    );
    dev.catalog.manifests = vec!["modules/probe.module.yaml".into()];
    std::fs::create_dir_all(root.join("dev/modules")).unwrap();
    std::fs::write(root.join("dev/modules/probe.module.yaml"), PROBE).unwrap();
    dev.write_to(&root.join("dev/env.toml")).unwrap();

    std::fs::create_dir_all(root.join("prod")).unwrap();
    std::fs::write(
        root.join("prod/env.toml"),
        "version = 4\n[selection]\nmodules = [\"envoy\", \"rustfs\"]\n[context]\n\
         env_name = \"prod\"\nproject_name = \"demo-prod\"\nmesh_network = \"trestle-demo-mesh\"\n\
         port_offset = 1000\ngateway_host_port = 10080\ngateway_admin_port = 10901\n\
         dedicated_listener_port_base = 10100\n",
    )
    .unwrap();
}

#[test]
fn import_then_export_round_trips_modulo_slots() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, out) = (tmp.path().join("src"), tmp.path().join("out"));
    workspace_dir(&src);
    let store = Store::open(&tmp.path().join("state.db")).unwrap();

    // Another workspace already holds slot 0 on this host.
    let other = store.create_workspace("other", false).unwrap();
    let env = store
        .create_environment(
            &other.id,
            "dev",
            olai_stack_state::Draft::new(EnvManifest::new(
                Selection::default(),
                PlanCtx::default(),
            )),
        )
        .unwrap();
    store.commit_draft(&env.id).unwrap();
    let busy = store.create_instance(&env.id, 1).unwrap();
    store.lease_slot(&busy.id).unwrap();

    let ws = store.import_workspace(&src).unwrap();
    assert!(ws.mesh);
    let envs = store.environments(&ws.id).unwrap();
    assert_eq!(
        envs.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["dev", "prod"]
    );
    let mut instances = Vec::new();
    for env in &envs {
        assert_eq!(env.head, Some(1));
        let instance = store.create_instance(&env.id, 1).unwrap();
        store.lease_slot(&instance.id).unwrap();
        instances.push(instance.id);
    }
    store.export_instances(&instances, &out).unwrap();

    // The workspace file matches but for the slots, which the store leased (0 was taken).
    let exported = WorkspaceManifest::read_from(&out.join("trestle-workspace.toml")).unwrap();
    let original = WorkspaceManifest::read_from(&src.join("trestle-workspace.toml")).unwrap();
    assert_eq!(exported.name, original.name);
    assert_eq!(exported.mesh, original.mesh);
    let slots: Vec<u16> = exported.envs.iter().map(|e| e.slot).collect();
    assert_eq!(slots, [1, 2]);
    let strip = |m: &WorkspaceManifest| -> Vec<(String, String)> {
        m.envs
            .iter()
            .map(|e| (e.name.clone(), e.path.clone()))
            .collect()
    };
    assert_eq!(strip(&exported), strip(&original));

    // Each member's environment and module manifests come back as they were (the v4 member
    // without its placement, as reading it gives).
    for member in ["dev", "prod"] {
        assert_eq!(
            read_env_dir(&out.join(member)).unwrap(),
            read_env_dir(&src.join(member)).unwrap(),
            "{member}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(out.join("dev/modules/probe.module.yaml")).unwrap(),
        PROBE
    );
    let prod = std::fs::read_to_string(out.join("prod/env.toml")).unwrap();
    assert!(!prod.contains("project_name"), "{prod}");

    // Re-importing the export as another workspace works.
    std::fs::write(
        out.join("trestle-workspace.toml"),
        std::fs::read_to_string(out.join("trestle-workspace.toml"))
            .unwrap()
            .replace("name = \"demo\"", "name = \"demo-copy\""),
    )
    .unwrap();
    store.import_workspace(&out).unwrap();
}

#[test]
fn a_failed_import_leaves_nothing_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    workspace_dir(&src);
    std::fs::remove_file(src.join("dev/modules/probe.module.yaml")).unwrap();
    let store = Store::open(&tmp.path().join("state.db")).unwrap();
    assert!(store.import_workspace(&src).is_err());
    assert!(store.workspaces().unwrap().is_empty());
}

#[test]
fn export_refuses_unleased_or_clashing_instances() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    workspace_dir(&src);
    let store = Store::open(&tmp.path().join("state.db")).unwrap();
    let ws = store.import_workspace(&src).unwrap();
    let dev = &store.environments(&ws.id).unwrap()[0];
    let a = store.create_instance(&dev.id, 1).unwrap();
    assert!(
        store
            .export_instances(std::slice::from_ref(&a.id), &tmp.path().join("x"))
            .is_err()
    );
    // Two instances of one environment would be one member twice.
    let b = store.create_instance(&dev.id, 1).unwrap();
    store.lease_slot(&a.id).unwrap();
    store.lease_slot(&b.id).unwrap();
    assert!(
        store
            .export_instances(&[a.id, b.id], &tmp.path().join("y"))
            .is_err()
    );
}
