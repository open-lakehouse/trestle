//! The `puppygraph` graph module: what it pulls into a stack, how its UI and query ports are
//! reached, and how its catalog connection to Unity Catalog and the object store is rendered.

use olai_stack_topology::{
    ClientSelection, ModuleId, Plan, PlanCtx, RenderOutput, Selection, Vantage, baseline_catalog,
};

fn plan_in(modules: &[&str], ctx: PlanCtx) -> Plan {
    let mut selection = Selection::modules(modules.iter().copied());
    selection.client = ClientSelection {
        python: true,
        examples: true,
    };
    baseline_catalog()
        .plan(&selection, &ctx)
        .expect("plan should succeed")
}

fn plan(modules: &[&str]) -> Plan {
    plan_in(modules, PlanCtx::default())
}

fn render(p: &Plan) -> RenderOutput {
    olai_stack_topology::render_all(p)
        .unwrap()
        .modules
        .into_iter()
        .find(|(module, _)| module.as_str() == "puppygraph")
        .map(|(_, out)| out)
        .expect("puppygraph renders")
}

fn file(out: &RenderOutput, path: &str) -> String {
    out.files
        .iter()
        .find(|f| f.path.ends_with(path))
        .unwrap_or_else(|| panic!("{path} rendered"))
        .contents
        .clone()
}

/// The rendered `catalog.json` (JSON is YAML, so the YAML parser checks it too).
fn catalog(p: &Plan) -> serde_yaml::Value {
    serde_yaml::from_str(&file(&render(p), "catalog.json")).expect("catalog.json parses")
}

#[test]
fn selecting_puppygraph_brings_the_catalog_and_its_stores() {
    let p = plan(&["puppygraph"]);
    for id in ["envoy", "unity-catalog", "postgres", "rustfs"] {
        assert!(
            p.services.contains_key(&ModuleId::from(id)),
            "{id} not planned"
        );
    }
}

#[test]
fn the_ui_gets_its_own_listener_and_the_query_ports_are_published() {
    let p = plan(&["puppygraph"]);
    let ui = p
        .gateway
        .listeners
        .iter()
        .skip(1)
        .find(|l| l.routes.iter().any(|r| r.cluster == "puppygraph"))
        .expect("a dedicated listener for the PuppyGraph UI");
    assert_eq!(ui.routes[0].prefix, "/");

    let host = p.client_env(Vantage::Host);
    assert_eq!(
        host.get("PUPPYGRAPH_URL"),
        Some(format!("http://localhost:{}", ui.host_port).as_str())
    );
    assert_eq!(host.get("PUPPYGRAPH_BOLT_ADDRESS"), Some("localhost:7687"));
    assert_eq!(
        host.get("PUPPYGRAPH_GREMLIN_ADDRESS"),
        Some("localhost:8182")
    );

    // Inside the network (Omnigent's agents, the hosted notebooks), the service is dialed
    // directly.
    let container = p.client_env(Vantage::Container);
    assert_eq!(
        container.get("PUPPYGRAPH_BOLT_ADDRESS"),
        Some("puppygraph:7687")
    );
    assert_eq!(
        container.get("PUPPYGRAPH_GREMLIN_ADDRESS"),
        Some("puppygraph:8182")
    );

    let fragment = render(&p).fragment;
    assert!(fragment.contains("\"7687:7687\""), "{fragment}");
    assert!(fragment.contains("\"8182:8182\""), "{fragment}");
}

#[test]
fn the_catalog_reads_unity_catalog_directly() {
    let p = plan(&["puppygraph"]);
    let catalog = catalog(&p);
    let uc = &catalog[0];
    assert_eq!(uc["name"].as_str(), Some("uc"));
    assert_eq!(uc["type"].as_str(), Some("deltalake"));
    // Direct on the compose network: the gateway would reject it under forward-auth.
    assert_eq!(uc["metastore"]["type"].as_str(), Some("unity"));
    assert_eq!(
        uc["metastore"]["host"].as_str(),
        Some("http://unitycatalog:8080")
    );
    assert_eq!(
        uc["metastore"]["databricksCatalogName"].as_str(),
        Some("unity")
    );
    // Table data is read with credentials UC vends, not the store's keys.
    assert!(uc.get("storage").is_none(), "{catalog:?}");
    assert!(
        !file(&render(&p), "catalog.json").contains("rustfs-root"),
        "no store keys"
    );
}

#[test]
fn it_trusts_the_gateway_ca_for_vended_s3_reads() {
    let f = render(&plan(&["puppygraph"])).fragment;
    // The JVM (metastore, Delta log) and the native engine (Parquet, OS trust store) both.
    assert!(f.contains("-Djavax.net.ssl.trustStore=/certs/"), "{f}");
    assert!(f.contains("update-ca-certificates"), "{f}");
    assert!(f.contains("user: root"), "{f}");
    assert!(f.contains("exec bash ./entrypoint.sh"), "{f}");
    assert!(f.contains("gateway-certs:/certs:ro"), "{f}");
    // The schema upload waits for the metastore it validates the mappings against.
    assert!(
        f.contains("      unitycatalog:\n        condition: service_healthy"),
        "{f}"
    );
}

#[test]
fn the_catalog_follows_its_knob() {
    let mut selection = Selection::modules(["puppygraph"]);
    selection
        .knob_overrides
        .entry(ModuleId::from("puppygraph"))
        .or_default()
        .insert("catalog".into(), "main".into());
    let p = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    assert_eq!(
        catalog(&p)[0]["metastore"]["databricksCatalogName"].as_str(),
        Some("main")
    );
}

#[test]
fn an_azure_store_still_plans_without_the_ca_dance() {
    let p = plan_in(
        &["puppygraph"],
        PlanCtx {
            provider_preference: [("object_store".to_string(), vec!["azurite".into()])].into(),
            ..PlanCtx::default()
        },
    );
    assert_eq!(catalog(&p)[0]["metastore"]["type"].as_str(), Some("unity"));
    let f = render(&p).fragment;
    assert!(
        !f.contains("user: root") && !f.contains("JAVA_TOOL_OPTIONS"),
        "{f}"
    );
}

#[test]
fn the_graph_schema_is_operator_owned_and_the_catalog_regenerated() {
    let out = plan(&["puppygraph"]).materialize().unwrap();
    let get = |path: &str| {
        out.files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} written"))
    };
    let schema = get("graphs/puppygraph/schema.json");
    assert!(schema.preserve && !schema.sensitive);
    let catalog = get("modules/puppygraph/catalog.json");
    assert!(!catalog.sensitive && !catalog.preserve);
    assert!(get("modules/puppygraph/.env/puppygraph.env").sensitive);
    assert!(
        out.files
            .iter()
            .any(|f| f.path == "modules/puppygraph/seed_schema.py")
    );
}

#[test]
fn the_client_env_carries_no_puppygraph_login() {
    let p = plan(&["puppygraph"]);
    for vantage in [Vantage::Host, Vantage::Container] {
        for (key, value) in p.client_env(vantage).iter() {
            assert!(
                !value.contains("puppygraph123") && !key.contains("PASSWORD"),
                "{key}={value}"
            );
        }
    }
}

#[test]
fn the_client_workspace_gets_the_graph_drivers_and_quickstart() {
    let p = plan(&["puppygraph"]);
    let workspace = p.python_client.as_ref().expect("workspace selected");
    for dep in ["neo4j", "gremlinpython"] {
        assert!(
            workspace.dependencies.iter().any(|d| d.starts_with(dep)),
            "{dep}: {:?}",
            workspace.dependencies
        );
    }
    assert!(
        workspace
            .notebooks
            .iter()
            .any(|(_, n)| n.file == "puppygraph_quickstart.py")
    );
}
