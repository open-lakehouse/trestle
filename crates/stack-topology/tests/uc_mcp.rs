//! The `uc-mcp` module: mangrove's `uc mcp serve` behind the gateway. What it pulls into a
//! stack, how it is routed, where clients find it, and which `Host` values it accepts.

use olai_stack_topology::{
    GatewaySettings, ModuleId, Plan, PlanCtx, PlanError, Selection, Vantage, baseline_catalog,
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
    // The image's own healthcheck probes the catalog server's port, so it is turned off.
    assert_eq!(svc["healthcheck"]["disable"].as_bool(), Some(true));
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
