//! `unity-catalog-rs`: mangrove's Rust Unity Catalog server. One configuration document for both
//! placements: its container mounts it, and a Neon Function is deployed with it. The
//! key-encryption key is never rendered.

use olai_stack_topology::{
    HostedConnection, HostedResource, HostedVendor, MaterializedOutput, ModuleId, Plan, PlanCtx,
    Selection, Vantage, baseline_catalog,
};

/// The KEK reference mangrove reads; the value only ever comes from the environment.
const KEK_REF: &str = "key: { env: UC_SERVER_KEK }";

fn plan(selection: &Selection) -> Plan {
    baseline_catalog()
        .plan(selection, &PlanCtx::default())
        .expect("plans")
}

fn local() -> Plan {
    plan(&Selection::modules(["unity-catalog-rs", "mlflow"]))
}

fn as_function() -> Plan {
    let mut selection = Selection::modules(["unity-catalog-rs", "mlflow"]);
    selection.hosted.insert(
        ModuleId::from("unity-catalog-rs"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::NeonFunction),
            connection: HostedConnection::Function { url: None },
        },
    );
    plan(&selection)
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> &'a str {
    out.files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.contents.as_str())
        .unwrap_or_else(|| panic!("expected {path}"))
}

#[test]
fn it_serves_the_unity_catalog_api_and_can_run_as_a_function() {
    let cat = baseline_catalog();
    let module = cat.get(&ModuleId::from("unity-catalog-rs")).unwrap();
    assert_eq!(module.provides().api.as_deref(), Some("unity-catalog"));
    assert_eq!(module.provides().function.as_deref(), Some("unitycatalog"));
    assert_eq!(module.provides().function_secrets, ["UC_SERVER_KEK"]);
}

#[test]
fn the_container_runs_migrate_then_serve_with_the_kek_passed_through() {
    let out = local().materialize().unwrap();
    let fragment: serde_yaml::Value =
        serde_yaml::from_str(file(&out, "modules/unity-catalog-rs/compose.yaml")).unwrap();
    let services = &fragment["services"];
    assert_eq!(
        services["unitycatalog"]["depends_on"]["unitycatalog-migrate"]["condition"],
        "service_completed_successfully"
    );
    for service in ["unitycatalog", "unitycatalog-migrate"] {
        // A bare name: Compose takes the value from the process that starts it.
        assert!(
            services[service]["environment"]["UC_SERVER_KEK"].is_null(),
            "{service}: {:?}",
            services[service]["environment"]
        );
    }
    let config = file(&out, "modules/unity-catalog-rs/config.yaml");
    assert!(config.contains(KEK_REF), "{config}");
    assert!(config.contains("url: { env: DATABASE_URL }"), "{config}");
    // The database URL lives in the git-ignored env file.
    let env = file(&out, "modules/unity-catalog-rs/.env/unitycatalog.env");
    assert!(env.starts_with("DATABASE_URL=postgres"), "{env}");
}

#[test]
fn no_rendered_file_holds_a_kek() {
    let out = local().materialize().unwrap();
    for f in &out.files {
        for line in f.contents.lines() {
            let line = line.trim();
            // Every mention is the reference or the bare pass-through name.
            if line.contains("UC_SERVER_KEK") && !line.starts_with('#') {
                assert!(
                    line.contains(KEK_REF)
                        || line == "UC_SERVER_KEK:"
                        || line.starts_with("export UC_SERVER_KEK=\"$(")
                        || line.contains("`UC_SERVER_KEK`"),
                    "{}: {line}",
                    f.path
                );
            }
        }
    }
    let readme = file(&out, "README.md");
    assert!(
        readme.contains("Set Unity Catalog's key-encryption key"),
        "{readme}"
    );
}

#[test]
fn as_a_function_it_is_deployed_with_the_containers_configuration() {
    let container = local().materialize().unwrap();
    let container_config = file(&container, "modules/unity-catalog-rs/config.yaml");

    let p = as_function();
    let neon = p.neon_requirements();
    let [function] = neon.functions.as_slice() else {
        panic!("{:?}", neon.functions)
    };
    assert_eq!(function.slug, "unitycatalog");
    assert_eq!(function.secrets, ["UC_SERVER_KEK"]);
    let deployed = &function.env["UC_SERVER_CONFIG_YAML"];
    // The container's file is the same document plus where it listens and its database.
    assert!(
        container_config.ends_with(deployed.as_str()),
        "{container_config}"
    );

    let out = p.materialize().unwrap();
    assert!(
        !out.files
            .iter()
            .any(|f| f.path.starts_with("modules/unity-catalog-rs/")),
        "nothing of it renders"
    );
    let readme = file(&out, "README.md");
    assert!(readme.contains("`UC_SERVER_CONFIG_YAML`"), "{readme}");
    assert!(readme.contains(KEK_REF), "{readme}");
    assert!(
        readme.contains("Set `UC_SERVER_KEK` on the deployment"),
        "{readme}"
    );
    // The container's KEK step is the function's job now.
    assert!(
        !readme.contains("Set Unity Catalog's key-encryption key"),
        "{readme}"
    );
}

#[test]
fn mlflow_serves_its_registry_from_it_either_way() {
    for p in [local(), as_function()] {
        let env = p.client_env(Vantage::Host);
        let uc = env.get("UNITY_CATALOG_URL").expect("UC address");
        let origin = uc.strip_suffix("/api/2.1/unity-catalog").expect("UC path");
        assert_eq!(
            env.get("MLFLOW_REGISTRY_URI"),
            Some(format!("uc:{origin}").as_str())
        );
    }
}

#[test]
fn the_data_catalog_capability_picks_one_server() {
    let selection = Selection {
        capabilities: vec!["data_catalog".into()],
        ..Selection::default()
    };
    let has = |p: &Plan, id: &str| p.graph.module(&ModuleId::from(id)).is_some();
    // The catalog's default: the OSS server.
    let p = plan(&selection);
    assert!(has(&p, "unity-catalog-oss") && !has(&p, "unity-catalog-rs"));
    // The environment's preference wins.
    let ctx = PlanCtx {
        provider_preference: [(
            "data_catalog".to_string(),
            vec![ModuleId::from("unity-catalog-rs")],
        )]
        .into(),
        ..PlanCtx::default()
    };
    let p = baseline_catalog().plan(&selection, &ctx).expect("plans");
    assert!(has(&p, "unity-catalog-rs") && !has(&p, "unity-catalog-oss"));
}
