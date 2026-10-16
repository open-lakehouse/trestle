//! Postgres data storage knob tests: `bind` keeps data under DATA_ROOT, `volume` uses a
//! named Docker volume.

use olai_stack_topology::{PlanCtx, Selection, baseline_catalog};

#[test]
fn postgres_data_bind_uses_data_root_bind_mount() {
    let selection = Selection::modules(["postgres"]);
    let p = baseline_catalog()
        .plan(
            &selection,
            &PlanCtx {
                env_name: "test".into(),
                ..Default::default()
            },
        )
        .expect("plan should succeed");
    let out = p.materialize().expect("plan materializes");

    let compose_file = out
        .files
        .iter()
        .find(|f| f.path == "modules/postgres/compose.yaml")
        .expect("postgres compose file should exist");
    let compose: serde_yaml::Value =
        serde_yaml::from_str(&compose_file.contents).expect("compose must be valid YAML");

    // Default: bind mount to DATA_ROOT
    let volumes = &compose["services"]["db"]["volumes"];
    assert!(volumes.is_sequence(), "volumes must be a sequence");
    let vol_list = volumes.as_sequence().unwrap();
    assert!(!vol_list.is_empty(), "volumes list should not be empty");
    let vol_str = vol_list[0].as_str().unwrap();
    assert!(
        vol_str.contains("./.data/postgres"),
        "default should use DATA_ROOT bind mount, got: {}",
        vol_str
    );

    // No top-level volumes section when using bind mount
    let top_volumes = &compose["volumes"];
    assert!(
        top_volumes.is_null(),
        "bind mount should not declare a top-level volumes section"
    );
}

#[test]
fn postgres_data_volume_uses_named_volume() {
    let mut selection = Selection::modules(["postgres"]);
    selection.knob_overrides.insert(
        "postgres".into(),
        vec![("data".into(), "volume".into())].into_iter().collect(),
    );

    let p = baseline_catalog()
        .plan(
            &selection,
            &PlanCtx {
                env_name: "test".into(),
                ..Default::default()
            },
        )
        .expect("plan should succeed");
    let out = p.materialize().expect("plan materializes");

    let compose_file = out
        .files
        .iter()
        .find(|f| f.path == "modules/postgres/compose.yaml")
        .expect("postgres compose file should exist");
    let compose: serde_yaml::Value =
        serde_yaml::from_str(&compose_file.contents).expect("compose must be valid YAML");

    // Named volume mount
    let volumes = &compose["services"]["db"]["volumes"];
    assert!(volumes.is_sequence(), "volumes must be a sequence");
    let vol_list = volumes.as_sequence().unwrap();
    assert!(!vol_list.is_empty(), "volumes list should not be empty");
    let vol_str = vol_list[0].as_str().unwrap();
    assert_eq!(
        vol_str, "postgres-data:/var/lib/postgresql/data",
        "volume knob should use named volume, got: {}",
        vol_str
    );

    // Top-level volumes section declares the named volume
    let top_volumes = &compose["volumes"];
    assert!(
        top_volumes.is_mapping(),
        "volume knob should declare top-level volumes section"
    );
    assert!(
        top_volumes["postgres-data"].is_mapping() || top_volumes["postgres-data"].is_null(),
        "postgres-data should be declared in volumes"
    );
}
