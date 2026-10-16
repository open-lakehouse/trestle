//! The `unity-catalog` module's knobs: what each one renders, and the values it rejects.

use std::collections::BTreeMap;

use olai_stack_topology::{ModuleId, Plan, PlanCtx, PlanError, Selection, baseline_catalog};

fn plan_with(knobs: &[(&str, &str)]) -> Result<Plan, PlanError> {
    let mut selection = Selection::modules(["unity-catalog"]);
    selection.knob_overrides.insert(
        "unity-catalog".into(),
        knobs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    );
    baseline_catalog().plan(&selection, &PlanCtx::default())
}

/// The rendered files, by path.
fn files(plan: &Plan) -> BTreeMap<String, String> {
    plan.materialize()
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn service(plan: &Plan) -> serde_yaml::Value {
    let fragment = &files(plan)["modules/unity-catalog/compose.yaml"];
    let yaml: serde_yaml::Value = serde_yaml::from_str(fragment).expect("fragment is YAML");
    yaml["services"]["unitycatalog"].clone()
}

fn rejected_key(err: PlanError) -> String {
    match err {
        PlanError::RejectedKnob { module, key, .. } => {
            assert_eq!(module, ModuleId::from("unity-catalog"));
            key
        }
        other => panic!("expected RejectedKnob, got {other:?}"),
    }
}

#[test]
fn defaults_render_the_postgres_backed_server() {
    let plan = plan_with(&[]).unwrap();
    for id in ["postgres", "rustfs"] {
        assert!(plan.graph.module(&ModuleId::from(id)).is_some(), "{id}");
    }
    let svc = service(&plan);
    assert_eq!(svc["init"], serde_yaml::Value::Bool(true));
    assert!(svc.get("container_name").is_none());
    assert!(svc.get("build").is_none());
    assert_eq!(svc["secrets"].as_sequence().unwrap().len(), 2);
    assert!(files(&plan).contains_key("modules/unity-catalog/secrets/hibernate.properties"));
}

#[test]
fn embedded_metadata_drops_both_stores() {
    let plan = plan_with(&[("embedded_metadata", "true")]).unwrap();
    for id in ["postgres", "rustfs"] {
        assert!(plan.graph.module(&ModuleId::from(id)).is_none(), "{id}");
    }
    assert!(
        plan.graph
            .needs(&ModuleId::from("unity-catalog"))
            .is_empty()
    );
    let svc = service(&plan);
    assert!(svc.get("depends_on").is_none());
    assert!(svc.get("environment").is_none());
    let secrets = svc["secrets"].as_sequence().unwrap();
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0]["source"], "unitycatalog_server_properties");
    let files = files(&plan);
    assert!(!files.contains_key("modules/unity-catalog/secrets/hibernate.properties"));
    let props = &files["modules/unity-catalog/secrets/server.properties"];
    assert!(!props.contains("s3.") && !props.contains("aws."), "{props}");
}

#[test]
fn container_name_build_and_mounts_render() {
    let plan = plan_with(&[
        ("container_name", "unitycatalog"),
        ("build", "image"),
        ("image", "uc-docs:local"),
        (
            "extra_mounts",
            "${UC_DOCS_ROOT:-/tmp/uc-docs}:${UC_DOCS_ROOT:-/tmp/uc-docs} ; ./conf:/conf:ro",
        ),
        ("embedded_metadata", "true"),
    ])
    .unwrap();
    let svc = service(&plan);
    assert_eq!(svc["container_name"], "unitycatalog");
    assert_eq!(svc["build"]["context"], "./image");
    assert_eq!(svc["pull_policy"], "build");
    assert_eq!(svc["image"], "uc-docs:local");
    let volumes: Vec<&str> = svc["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        volumes,
        [
            "${UC_DOCS_ROOT:-/tmp/uc-docs}:${UC_DOCS_ROOT:-/tmp/uc-docs}",
            "./conf:/conf:ro"
        ]
    );
}

#[test]
fn extra_mounts_join_the_trust_volume() {
    // RustFS answers the AWS hostnames over TLS, so UC mounts the gateway's CA.
    let plan = plan_with(&[("extra_mounts", "./data:/data")]).unwrap();
    let svc = service(&plan);
    let volumes = svc["volumes"].as_sequence().unwrap();
    assert_eq!(volumes.len(), 2, "{volumes:?}");
    assert_eq!(volumes[1], "./data:/data");
}

#[test]
fn invalid_knob_values_are_rejected() {
    for (key, value) in [
        ("container_name", "-leading-dash"),
        ("container_name", "has space"),
        ("extra_mounts", "./a:/a;;./b:/b"),
        ("extra_mounts", "no-target"),
        ("extra_mounts", "\"./a\":/a"),
        ("build", "../outside"),
        ("build", "/abs/path"),
        ("build", "./dot-prefixed"),
        ("build", " "),
        ("embedded_metadata", "sometimes"),
        ("master_role_arn", "uc-master"),
        ("master_role_arn", "arn:aws:iam::1:user/someone"),
    ] {
        let err = plan_with(&[(key, value)]).unwrap_err();
        let got = match err {
            PlanError::InvalidKnobValue { key, .. } => key,
            other => rejected_key(other),
        };
        assert_eq!(got, key, "{key} = {value:?}");
    }
}

fn compose(plan: &Plan) -> serde_yaml::Value {
    serde_yaml::from_str(&files(plan)["modules/unity-catalog/compose.yaml"]).unwrap()
}

#[test]
fn storage_is_registered_through_the_api_by_default() {
    let plan = plan_with(&[("default_catalog", "main")]).unwrap();
    let init = &compose(&plan)["services"]["unitycatalog-storage"];
    assert_eq!(init["image"], "python:3.13-alpine");
    let files = files(&plan);
    let storage = &files["modules/unity-catalog/storage.txt"];
    let entries: Vec<&str> = storage.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        entries,
        [
            "location unity s3://unity unity_storage arn:aws:iam::000000000000:role/local-storage",
            "catalog main",
        ]
    );
    assert!(files.contains_key("modules/unity-catalog/bootstrap_storage.py"));
    let props = &files["modules/unity-catalog/secrets/server.properties"];
    assert!(!props.contains("s3."), "{props}");
    assert!(props.contains("aws.accessKey="), "{props}");
}

#[test]
fn storage_bootstrap_none_renders_no_storage_helper() {
    let plan = plan_with(&[("storage_bootstrap", "none")]).unwrap();
    assert!(
        compose(&plan)["services"]
            .get("unitycatalog-storage")
            .is_none()
    );
    let files = files(&plan);
    assert!(!files.contains_key("modules/unity-catalog/storage.txt"));
    assert!(!files.contains_key("modules/unity-catalog/bootstrap_storage.py"));
    // The master identity stays: storage credentials created later still vend.
    let props = &files["modules/unity-catalog/secrets/server.properties"];
    assert!(props.contains("aws.masterRoleArn="), "{props}");
}

#[test]
fn embedded_metadata_renders_no_storage_helper() {
    let plan = plan_with(&[("embedded_metadata", "true")]).unwrap();
    assert!(
        compose(&plan)["services"]
            .get("unitycatalog-storage")
            .is_none()
    );
    assert!(!files(&plan).contains_key("modules/unity-catalog/storage.txt"));
}

#[test]
fn default_catalog_needs_api_registration_and_a_valid_name() {
    for knobs in [
        &[("default_catalog", "main"), ("storage_bootstrap", "none")][..],
        &[("default_catalog", "main"), ("embedded_metadata", "true")],
        &[("default_catalog", "1st")],
        &[("default_catalog", "has-dash")],
    ] {
        let err = plan_with(knobs).unwrap_err();
        assert_eq!(rejected_key(err), "default_catalog", "{knobs:?}");
    }
    let err = plan_with(&[("storage_bootstrap", "properties")]).unwrap_err();
    assert!(matches!(err, PlanError::InvalidKnobValue { .. }), "{err:?}");
}

#[test]
fn master_role_arn_overrides_the_store_role() {
    let props =
        |plan: &Plan| files(plan)["modules/unity-catalog/secrets/server.properties"].clone();
    let default = plan_with(&[]).unwrap();
    assert!(
        props(&default).contains("aws.masterRoleArn=arn:aws:iam::000000000000:role/local-storage")
    );
    let custom = plan_with(&[(
        "master_role_arn",
        "arn:aws:iam::123456789012:role/uc-master",
    )])
    .unwrap();
    assert!(props(&custom).contains("aws.masterRoleArn=arn:aws:iam::123456789012:role/uc-master"));
}

#[test]
fn bucket_renames_the_object_store_demand() {
    let buckets = |plan: &Plan| plan.provisioned[&ModuleId::from("rustfs")].clone();
    assert_eq!(buckets(&plan_with(&[]).unwrap()), ["unity"]);
    let plan = plan_with(&[("bucket", "uc-docs")]).unwrap();
    assert_eq!(buckets(&plan), ["uc-docs"]);
    let files = files(&plan);
    let init = &files["modules/rustfs/compose.yaml"];
    assert!(
        init.contains("s3://uc-docs") && !init.contains("s3://unity"),
        "{init}"
    );
}
