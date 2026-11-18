//! The `puppygraph` graph module: what it pulls into a stack, how its UI and query ports are
//! reached, and how its catalog connection to Unity Catalog and the object store is rendered.

use olai_stack_topology::projection::{TopologyEdgeDto, graph_dto};
use olai_stack_topology::{
    ClientSelection, HostedConnection, HostedResource, HostedVendor, ModuleId, Plan, PlanCtx,
    RenderOutput, Secrets, Selection, Vantage, baseline_catalog,
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
    for id in ["envoy", "unity-catalog-oss", "postgres", "rustfs"] {
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

/// The Unity Catalog server `p`'s graph draws PuppyGraph using, with the capability labelled.
fn catalog_used(p: &Plan) -> Option<(String, String)> {
    graph_dto(p).edges.into_iter().find_map(|e| match e {
        TopologyEdgeDto::Uses { from, to, role, .. }
            if from == "puppygraph" && role == "data_catalog" =>
        {
            Some((to, role))
        }
        _ => None,
    })
}

#[test]
fn it_uses_mangroves_catalog_when_the_stack_runs_it() {
    // Selected directly, or pulled in by uc-mcp (which needs mangrove's server): either way no
    // second Unity Catalog server joins it.
    for modules in [
        &["unity-catalog-rs", "puppygraph"][..],
        &["envoy", "uc-mcp", "puppygraph"][..],
    ] {
        let p = plan(modules);
        assert!(
            !p.services
                .contains_key(&ModuleId::from("unity-catalog-oss")),
            "{modules:?} planned the OSS server too"
        );
        assert_eq!(
            catalog_used(&p),
            Some(("unity-catalog-rs".into(), "data_catalog".into())),
            "{modules:?}"
        );
        // The catalog starts before PuppyGraph.
        let order: Vec<&str> = p.graph.nodes.iter().map(|m| m.id().as_str()).collect();
        let at = |id: &str| order.iter().position(|m| *m == id).unwrap();
        assert!(at("unity-catalog-rs") < at("puppygraph"), "{order:?}");
    }
}

#[test]
fn without_a_catalog_it_brings_the_default_or_the_preferred_one() {
    assert_eq!(
        catalog_used(&plan(&["puppygraph"])),
        Some(("unity-catalog-oss".into(), "data_catalog".into()))
    );
    let ctx = PlanCtx {
        provider_preference: [("data_catalog".to_string(), vec!["unity-catalog-rs".into()])].into(),
        ..PlanCtx::default()
    };
    let p = plan_in(&["puppygraph"], ctx);
    assert!(
        !p.services
            .contains_key(&ModuleId::from("unity-catalog-oss"))
    );
    assert_eq!(
        catalog_used(&p).map(|(to, _)| to).as_deref(),
        Some("unity-catalog-rs")
    );
}

/// Unity Catalog's Neon Function URL, as a branch binds it (with a trailing `/`).
const UC_FUNCTION_URL: &str = "https://br-x-unitycatalog.compute.example.neon.tech/";

/// PuppyGraph on mangrove's Unity Catalog hosted as a Neon Function, with the branch's object
/// storage as the store; `uc_url` overrides PuppyGraph's knob when set.
fn plan_on_uc_function(uc_url: Option<&str>) -> Plan {
    let mut selection =
        Selection::modules(["envoy", "unity-catalog-rs", "neon-storage", "puppygraph"]);
    selection.hosted.insert(
        ModuleId::from("unity-catalog-rs"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::NeonFunction),
            connection: HostedConnection::Function { url: None },
        },
    );
    if let Some(url) = uc_url {
        selection
            .knob_overrides
            .entry(ModuleId::from("puppygraph"))
            .or_default()
            .insert("uc_url".into(), url.into());
    }
    let secrets: Secrets = [
        ("NEON_S3_ENDPOINT", "https://br-x.storage.example.neon.tech"),
        ("NEON_S3_REGION", "eu-central-1"),
        ("NEON_S3_ACCESS_KEY_ID", "key-id"),
        ("NEON_S3_SECRET_ACCESS_KEY", "s3-secret"),
        ("UNITYCATALOG_FUNCTION_URL", UC_FUNCTION_URL),
    ]
    .into_iter()
    .collect();
    let ctx = PlanCtx {
        env_name: "neon".into(),
        secrets: Some(secrets),
        ..PlanCtx::default()
    };
    baseline_catalog()
        .plan(&selection, &ctx)
        .expect("plan should succeed")
}

/// The `puppygraph-schema` one-shot's `depends_on` keys.
fn schema_upload_waits_for(p: &Plan) -> Vec<String> {
    let fragment: serde_yaml::Value = serde_yaml::from_str(&render(p).fragment).unwrap();
    fragment["services"]["puppygraph-schema"]["depends_on"]
        .as_mapping()
        .expect("a depends_on map")
        .keys()
        .map(|k| k.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_catalog_run_as_a_function_is_reached_at_its_url() {
    let p = plan_on_uc_function(None);
    assert!(!p.stack_facts.runs("unity-catalog-rs"));
    // No `unitycatalog` service renders, so nothing waits for one.
    let out = p.materialize().unwrap();
    assert!(
        out.files
            .iter()
            .all(|f| !f.path.starts_with("modules/unity-catalog-rs/")),
        "the function renders nothing"
    );
    assert_eq!(schema_upload_waits_for(&p), ["puppygraph"]);
    // The function's URL, without the trailing slash the metastore client would double. Still
    // no token: mangrove's server is anonymous as a function too.
    let metastore = &catalog(&p)[0]["metastore"];
    assert_eq!(
        metastore["host"].as_str(),
        Some("https://br-x-unitycatalog.compute.example.neon.tech")
    );
    assert_eq!(metastore["token"].as_str(), Some("local"));
}

#[test]
fn an_in_stack_catalog_is_waited_for_on_the_compose_network() {
    let p = plan(&["unity-catalog-rs", "puppygraph"]);
    assert_eq!(
        catalog(&p)[0]["metastore"]["host"].as_str(),
        Some("http://unitycatalog:8080")
    );
    assert_eq!(schema_upload_waits_for(&p), ["puppygraph", "unitycatalog"]);
}

#[test]
fn a_set_uc_url_wins_wherever_the_catalog_runs() {
    let custom = "https://uc.example.com";
    // Hosted as a function: the knob replaces the function's URL; nothing in-stack to wait for.
    let p = plan_on_uc_function(Some(custom));
    assert_eq!(catalog(&p)[0]["metastore"]["host"].as_str(), Some(custom));
    assert_eq!(schema_upload_waits_for(&p), ["puppygraph"]);
    // In the stack: the knob replaces the service address, and the upload still waits for it.
    let mut selection = Selection::modules(["unity-catalog-rs", "puppygraph"]);
    selection
        .knob_overrides
        .entry(ModuleId::from("puppygraph"))
        .or_default()
        .insert("uc_url".into(), custom.into());
    let p = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    assert_eq!(catalog(&p)[0]["metastore"]["host"].as_str(), Some(custom));
    assert_eq!(schema_upload_waits_for(&p), ["puppygraph", "unitycatalog"]);
}
