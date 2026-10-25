//! MLflow with Unity Catalog: one model registry per environment. When both are in the stack,
//! MLflow's server serves its registry from UC and clients are pointed at UC's API; MLflow
//! alone keeps its registry in its own database.

use std::collections::BTreeMap;

use olai_stack_topology::{
    ModuleId, Plan, PlanCtx, RenderOutput, Selection, Vantage, baseline_catalog, render_all,
};

fn plan_with(modules: &[&str], knobs: BTreeMap<ModuleId, BTreeMap<String, String>>) -> Plan {
    let selection = Selection {
        modules: modules.iter().map(|m| ModuleId::from(*m)).collect(),
        knob_overrides: knobs,
        ..Default::default()
    };
    baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .expect("plan should succeed")
}

fn plan(modules: &[&str]) -> Plan {
    plan_with(modules, BTreeMap::new())
}

fn mlflow(p: &Plan) -> RenderOutput {
    render_all(p)
        .unwrap()
        .modules
        .into_iter()
        .find(|(module, _)| module.as_str() == "mlflow")
        .map(|(_, out)| out)
        .expect("mlflow renders")
}

/// The `mlflow` service of the rendered fragment.
fn service(out: &RenderOutput) -> serde_yaml::Value {
    let fragment: serde_yaml::Value = serde_yaml::from_str(&out.fragment).expect("fragment parses");
    fragment["services"]["mlflow"].clone()
}

fn command(service: &serde_yaml::Value) -> Vec<String> {
    service["command"]
        .as_sequence()
        .expect("command list")
        .iter()
        .map(|v| v.as_str().expect("string arg").to_string())
        .collect()
}

#[test]
fn mlflow_alone_keeps_its_own_registry() {
    let p = plan(&["mlflow"]);
    let svc = service(&mlflow(&p));
    assert!(
        !command(&svc).iter().any(|a| a == "--registry-store-uri"),
        "{svc:?}"
    );
    assert!(svc["environment"].get("PYTHONPATH").is_none(), "{svc:?}");
    assert!(svc.get("configs").is_none(), "{svc:?}");
    assert!(svc["depends_on"].get("unitycatalog").is_none(), "{svc:?}");
    for vantage in [Vantage::Host, Vantage::Container] {
        assert_eq!(p.client_env(vantage).get("MLFLOW_REGISTRY_URI"), None);
    }
}

#[test]
fn with_unity_catalog_the_server_serves_its_registry_from_uc() {
    let p = plan(&["mlflow", "unity-catalog"]);
    let out = mlflow(&p);
    let svc = service(&out);

    // Direct on the compose network, like PuppyGraph: the gateway would reject it under
    // forward-auth.
    let args = command(&svc);
    let at = args
        .iter()
        .position(|a| a == "--registry-store-uri")
        .expect("registry store set");
    assert_eq!(args[at + 1], "uc:http://unitycatalog:8080");
    // And it starts once UC serves, beside its demanded stores.
    assert_eq!(
        svc["depends_on"]["unitycatalog"]["condition"].as_str(),
        Some("service_healthy")
    );
    assert!(svc["depends_on"].get("db").is_some(), "{svc:?}");

    // The package registering the `uc` scheme is mounted and on the path.
    let plugins = "/opt/trestle/mlflow-plugins";
    assert_eq!(svc["environment"]["PYTHONPATH"].as_str(), Some(plugins));
    let dist_info = format!("{plugins}/trestle_mlflow_uc_registry-0.0.0.dist-info");
    let targets: Vec<&str> = svc["configs"]
        .as_sequence()
        .expect("configs mounted")
        .iter()
        .map(|c| c["target"].as_str().expect("target"))
        .collect();
    assert_eq!(
        targets,
        [
            format!("{plugins}/trestle_mlflow_uc_registry.py"),
            format!("{dist_info}/METADATA"),
            format!("{dist_info}/entry_points.txt")
        ]
    );
    let entry_points = out
        .files
        .iter()
        .find(|f| f.path.ends_with("uc-registry/entry_points.txt"))
        .expect("entry points rendered");
    assert!(
        entry_points.contents.contains(
            "[mlflow.model_registry_store]\nuc = trestle_mlflow_uc_registry:UcRegistryStore"
        ),
        "{}",
        entry_points.contents
    );

    // Every mounted file is declared at the compose head.
    let head = render_all(&p).unwrap().head;
    for alias in [
        "mlflow_uc_registry_module",
        "mlflow_uc_registry_metadata",
        "mlflow_uc_registry_entry_points",
    ] {
        assert!(
            head.configs.iter().any(|c| c.alias == alias),
            "{alias} declared"
        );
    }
}

#[test]
fn with_unity_catalog_clients_use_uc_as_their_registry() {
    let p = plan(&["mlflow", "unity-catalog"]);
    for vantage in [Vantage::Host, Vantage::Container] {
        let env = p.client_env(vantage);
        // The MLflow client appends `/api/2.1/unity-catalog` to the `uc:` origin itself.
        let uc_url = env.get("UNITY_CATALOG_URL").expect("UC address");
        let origin = uc_url
            .strip_suffix("/api/2.1/unity-catalog")
            .expect("UC is mounted at the gateway root");
        assert_eq!(
            env.get("MLFLOW_REGISTRY_URI"),
            Some(format!("uc:{origin}").as_str()),
            "{vantage:?}"
        );
    }
}

#[test]
fn the_uc_url_knob_moves_the_server_side_registry() {
    let knobs = BTreeMap::from([(
        ModuleId::from("mlflow"),
        BTreeMap::from([("uc_url".to_string(), "http://uc.internal:9000".to_string())]),
    )]);
    let p = plan_with(&["mlflow", "unity-catalog"], knobs);
    let args = command(&service(&mlflow(&p)));
    assert!(
        args.iter().any(|a| a == "uc:http://uc.internal:9000"),
        "{args:?}"
    );
}
