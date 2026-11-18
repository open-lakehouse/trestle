//! The `uc-mcp` module: mangrove's `uc mcp serve` behind the gateway. What it pulls into a
//! stack, how it is routed, where clients find it, and which `Host` values it accepts.

use olai_stack_topology::{
    GatewaySettings, HostedConnection, HostedResource, HostedVendor, ModuleId, Plan, PlanCtx,
    PlanError, Secrets, Selection, Vantage, baseline_catalog,
};

fn selection() -> Selection {
    Selection::modules(["envoy", "uc-mcp"])
}

fn plan(selection: Selection, ctx: PlanCtx) -> Plan {
    baseline_catalog()
        .plan(&selection, &ctx)
        .expect("plan should succeed")
}

fn with_auth(mut selection: Selection) -> Selection {
    selection.knob_overrides.insert(
        "envoy".into(),
        [("auth".to_string(), "true".to_string())].into(),
    );
    selection
}

fn fragment(p: &Plan) -> serde_yaml::Value {
    let text = olai_stack_topology::render_all(p)
        .unwrap()
        .modules
        .into_iter()
        .find(|(module, _)| module.as_str() == "uc-mcp")
        .map(|(_, out)| out.fragment.clone())
        .expect("uc-mcp renders");
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("invalid YAML ({e}):\n{text}"))
}

fn command(p: &Plan) -> Vec<String> {
    fragment(p)["services"]["uc-mcp"]["command"]
        .as_sequence()
        .expect("a command list")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

/// The values of every `--allowed-host` flag, in order.
fn allowed_hosts(p: &Plan) -> Vec<String> {
    let cmd = command(p);
    cmd.windows(2)
        .filter(|w| w[0] == "--allowed-host")
        .map(|w| w[1].clone())
        .collect()
}

#[test]
fn selecting_uc_mcp_brings_mangroves_catalog_server() {
    let p = plan(selection(), PlanCtx::default());
    // It requires the Rust server (its SCIM API backs principal search), which brings the
    // relational store it demands.
    for id in ["unity-catalog-rs", "postgres"] {
        assert!(
            p.services.contains_key(&ModuleId::from(id)),
            "{id} not planned"
        );
    }
    let services = &p.services[&ModuleId::from("uc-mcp")];
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "uc-mcp");
    assert_eq!(services[0].role.as_str(), "mcp_server");
}

#[test]
fn uc_mcp_conflicts_with_the_oss_catalog() {
    let selection = Selection::modules(["envoy", "unity-catalog-oss", "uc-mcp"]);
    let err = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap_err();
    assert_eq!(
        err,
        PlanError::ConflictingApiProviders {
            api: "unity-catalog".into(),
            providers: vec!["unity-catalog-oss".into(), "unity-catalog-rs".into()],
        }
    );
}

#[test]
fn the_mcp_endpoint_is_forwarded_unchanged_and_streams() {
    let p = plan(selection(), PlanCtx::default());
    let route = p.gateway.listeners[0]
        .routes
        .iter()
        .find(|r| r.prefix == "/uc-mcp/mcp")
        .expect("a route for /uc-mcp/mcp");
    assert_eq!(route.cluster, "uc-mcp");
    assert_eq!(route.rewrite, None);
    // The GET event stream stays open, so the route has no request timeout.
    assert!(route.options.streaming);
}

#[test]
fn clients_find_the_mcp_endpoint_through_the_gateway() {
    let p = plan(selection(), PlanCtx::default());
    assert_eq!(
        p.client_env(Vantage::Host).get("UC_MCP_URL").unwrap(),
        "http://localhost:9080/uc-mcp/mcp"
    );
    assert_eq!(
        p.client_env(Vantage::Container).get("UC_MCP_URL").unwrap(),
        "http://envoy:10000/uc-mcp/mcp"
    );
}

#[test]
fn the_server_serves_the_gateway_path_against_the_in_network_catalog() {
    let p = plan(selection(), PlanCtx::default());
    let f = fragment(&p);
    let svc = &f["services"]["uc-mcp"];
    assert_eq!(svc["entrypoint"][0].as_str(), Some("/usr/local/bin/uc"));
    let cmd = command(&p);
    let flag = |name: &str| {
        let i = cmd
            .iter()
            .position(|a| a == name)
            .unwrap_or_else(|| panic!("no {name}"));
        cmd[i + 1].clone()
    };
    assert_eq!(flag("--server"), "http://unitycatalog:8080");
    assert_eq!(flag("--http"), "0.0.0.0:8090");
    assert_eq!(flag("--path"), "/uc-mcp/mcp");
    // No token: the local catalog is anonymous.
    assert!(!cmd.iter().any(|a| a == "--token"), "{cmd:?}");
    assert_eq!(
        svc["depends_on"]["unitycatalog"]["condition"].as_str(),
        Some("service_healthy")
    );
    // Its own `/health`, on its port: the image's inherited check probes the catalog's 8080.
    let test: Vec<&str> = svc["healthcheck"]["test"]
        .as_sequence()
        .expect("a healthcheck test")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        test,
        [
            "CMD",
            "/usr/local/bin/uc-server",
            "healthcheck",
            "--url",
            "http://127.0.0.1:8090/health"
        ]
    );
    assert!(svc["healthcheck"]["disable"].is_null());
}

#[test]
fn an_open_gateway_allows_loopback_its_own_name_and_the_gateway() {
    let p = plan(selection(), PlanCtx::default());
    // The host-side gateway name is `localhost`, already listed.
    assert_eq!(
        allowed_hosts(&p),
        ["localhost", "127.0.0.1", "uc-mcp", "envoy"]
    );
}

#[test]
fn under_auth_the_gateways_secure_names_are_allowed() {
    let p = plan(
        with_auth(selection()),
        PlanCtx {
            env_name: "demo".into(),
            ..PlanCtx::default()
        },
    );
    assert_eq!(
        allowed_hosts(&p),
        [
            "localhost",
            "127.0.0.1",
            "uc-mcp",
            "demo.localhost",
            "demo.internal"
        ]
    );
}

#[test]
fn without_a_gateway_only_direct_names_are_allowed() {
    let mut selection = Selection::modules(["uc-mcp"]);
    selection.gateway = GatewaySettings {
        enabled: false,
        ..GatewaySettings::default()
    };
    let p = plan(selection, PlanCtx::default());
    assert_eq!(allowed_hosts(&p), ["localhost", "127.0.0.1", "uc-mcp"]);
    // The endpoint is published on its own port, at the path the server serves.
    let env = p.client_env(Vantage::Host);
    let url = env.get("UC_MCP_URL").unwrap();
    assert!(
        url.starts_with("http://localhost:") && url.ends_with("/uc-mcp/mcp"),
        "{url}"
    );
}

/// `uc-mcp` beside mangrove's Unity Catalog hosted as a Neon Function, the URL bound by the
/// branch; `uc_url` overrides the MCP server's knob when set.
fn on_uc_function(uc_url: Option<&str>) -> Plan {
    let mut selection = selection();
    selection.hosted.insert(
        ModuleId::from("unity-catalog-rs"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::NeonFunction),
            connection: HostedConnection::Function { url: None },
        },
    );
    with_uc_url(&mut selection, uc_url);
    let secrets: Secrets = [(
        "UNITYCATALOG_FUNCTION_URL",
        "https://br-x-unitycatalog.compute.example.neon.tech/",
    )]
    .into_iter()
    .collect();
    plan(
        selection,
        PlanCtx {
            secrets: Some(secrets),
            ..PlanCtx::default()
        },
    )
}

fn with_uc_url(selection: &mut Selection, uc_url: Option<&str>) {
    if let Some(url) = uc_url {
        selection
            .knob_overrides
            .entry(ModuleId::from("uc-mcp"))
            .or_default()
            .insert("uc_url".into(), url.into());
    }
}

/// The value of the `--server` flag.
fn server(p: &Plan) -> String {
    let cmd = command(p);
    let i = cmd.iter().position(|a| a == "--server").expect("--server");
    cmd[i + 1].clone()
}

#[test]
fn a_catalog_run_as_a_function_is_served_from_its_url() {
    let p = on_uc_function(None);
    assert!(!p.stack_facts.runs("unity-catalog-rs"));
    assert_eq!(
        server(&p),
        "https://br-x-unitycatalog.compute.example.neon.tech"
    );
    // No `unitycatalog` service renders, so the MCP server waits for none.
    let f = fragment(&p);
    assert!(f["services"]["uc-mcp"]["depends_on"].is_null(), "{f:?}");
    assert!(!command(&p).iter().any(|a| a == "--token"));
}

#[test]
fn a_set_uc_url_wins_wherever_the_catalog_runs() {
    let custom = "https://uc.example.com";
    let p = on_uc_function(Some(custom));
    assert_eq!(server(&p), custom);
    assert!(fragment(&p)["services"]["uc-mcp"]["depends_on"].is_null());

    let mut selection = selection();
    with_uc_url(&mut selection, Some(custom));
    let p = plan(selection, PlanCtx::default());
    assert_eq!(server(&p), custom);
    assert_eq!(
        fragment(&p)["services"]["uc-mcp"]["depends_on"]["unitycatalog"]["condition"].as_str(),
        Some("service_healthy")
    );
}
