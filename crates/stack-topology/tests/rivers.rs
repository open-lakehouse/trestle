//! The `rivers` orchestration module: it runs the client workspace's packages against the
//! operator-owned pipelines, fronts its UI on a dedicated listener, and puts that listener
//! behind the gateway's sign-in when forward-auth is on.

use std::collections::BTreeMap;

use olai_stack_topology::{
    ClientSelection, ListenerConfig, ModuleId, Plan, PlanCtx, Selection, Vantage, baseline_catalog,
    render_all,
};

fn plan_in(modules: &[&str], auth: bool) -> Plan {
    let mut selection = Selection::modules(modules.iter().copied());
    selection.client = ClientSelection {
        python: true,
        examples: true,
    };
    if auth {
        selection.knob_overrides = BTreeMap::from([(
            ModuleId::from("envoy"),
            BTreeMap::from([("auth".to_string(), "true".to_string())]),
        )]);
    }
    baseline_catalog()
        .plan(
            &selection,
            &PlanCtx {
                env_name: "demo".into(),
                ..PlanCtx::default()
            },
        )
        .expect("plan should succeed")
}

fn plan(auth: bool) -> Plan {
    plan_in(&["rivers"], auth)
}

fn fragment(p: &Plan) -> String {
    render_all(p)
        .unwrap()
        .modules
        .into_iter()
        .find(|(module, _)| module.as_str() == "rivers")
        .map(|(_, out)| out.fragment.clone())
        .expect("rivers renders")
}

/// The dedicated listener fronting `cluster`.
fn listener<'p>(p: &'p Plan, cluster: &str) -> &'p ListenerConfig {
    p.gateway
        .listeners
        .iter()
        .skip(1)
        .find(|l| l.routes.iter().any(|r| r.cluster == cluster))
        .unwrap_or_else(|| panic!("a dedicated listener for {cluster}"))
}

/// The rendered Envoy listener binding `port`.
fn envoy_listener(p: &Plan, port: u16) -> serde_yaml::Value {
    let doc: serde_yaml::Value = serde_yaml::from_str(&render_all(p).unwrap().envoy).unwrap();
    doc["static_resources"]["listeners"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|l| l["address"]["socket_address"]["port_value"].as_u64() == Some(port.into()))
        .unwrap_or_else(|| panic!("an Envoy listener on {port}"))
        .clone()
}

fn has_ext_authz(listener: &serde_yaml::Value) -> bool {
    listener["filter_chains"][0]["filters"][0]["typed_config"]["http_filters"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|f| f["name"].as_str() == Some("envoy.filters.http.ext_authz"))
}

#[test]
fn selecting_rivers_generates_the_workspace_with_the_sdk() {
    let p = plan(false);
    assert!(p.services.contains_key(&ModuleId::from("envoy")));
    let workspace = p
        .python_client
        .as_ref()
        .expect("the workspace is generated");
    assert!(
        workspace
            .dependencies
            .iter()
            .any(|d| d.starts_with("rivers"))
    );
    assert!(
        workspace
            .notebooks
            .iter()
            .any(|(_, n)| n.file == "rivers_quickstart.py")
    );

    // Without `[selection.client]` the workspace still comes: the server runs from it.
    let bare = baseline_catalog()
        .plan(&Selection::modules(["rivers"]), &PlanCtx::default())
        .unwrap();
    assert!(bare.python_client.is_some());
}

#[test]
fn the_ui_gets_its_own_open_listener_without_auth() {
    let p = plan(false);
    let ui = listener(&p, "rivers");
    assert_eq!(ui.routes[0].prefix, "/");
    assert!(ui.routes[0].options.streaming);
    assert!(ui.gated);
    assert!(!has_ext_authz(&envoy_listener(&p, ui.internal_port)));

    assert_eq!(
        p.client_env(Vantage::Host).get("RIVERS_URL"),
        Some(format!("http://localhost:{}", ui.host_port).as_str())
    );
    assert!(!fragment(&p).contains("RIVERS_AUTH_MODE"));
}

#[test]
fn under_gateway_auth_the_ui_listener_is_signed_in_and_trusts_the_identity() {
    let p = plan(true);
    let ui = listener(&p, "rivers");
    let envoy = envoy_listener(&p, ui.internal_port);
    assert!(has_ext_authz(&envoy));
    // Like the shared listener: TLS for host-side browsers, plain HTTP in-network.
    let chains = envoy["filter_chains"].as_sequence().unwrap();
    assert_eq!(chains.len(), 2);
    assert_eq!(
        chains[0]["filter_chain_match"]["transport_protocol"].as_str(),
        Some("tls")
    );
    assert_eq!(chains[1]["filters"], chains[0]["filters"]);

    // Clients reach it at the secure origin the auth provider issues sessions for.
    assert_eq!(
        p.client_env(Vantage::Host).get("RIVERS_URL"),
        Some(format!("https://demo.localhost:{}", ui.host_port).as_str())
    );
    assert_eq!(
        p.client_env(Vantage::Container).get("RIVERS_URL"),
        Some(format!("http://demo.internal:{}", ui.internal_port).as_str())
    );

    let f = fragment(&p);
    assert!(f.contains("RIVERS_AUTH_MODE: forward"), "{f}");
    assert!(f.contains("RIVERS_AUTH_FORWARD_TRUSTED_PROXIES"), "{f}");
}

#[test]
fn an_ungated_dedicated_listener_stays_open_under_auth() {
    // PuppyGraph's UI has its own login: its listener is not behind the gateway's sign-in.
    let p = plan_in(&["rivers", "puppygraph"], true);
    let graph = listener(&p, "puppygraph");
    assert!(!graph.gated);
    assert!(!has_ext_authz(&envoy_listener(&p, graph.internal_port)));
    assert!(
        p.client_env(Vantage::Host)
            .get("PUPPYGRAPH_URL")
            .unwrap()
            .starts_with("http://localhost:")
    );
    assert!(has_ext_authz(&envoy_listener(
        &p,
        listener(&p, "rivers").internal_port
    )));
}

#[test]
fn the_server_runs_the_operator_pipelines_from_the_workspace_venv() {
    let p = plan(false);
    let f = fragment(&p);
    assert!(f.contains("rivers dev quickstart"), "{f}");
    assert!(f.contains("--host 0.0.0.0 --port 3000"), "{f}");
    assert!(f.contains("./client:/workspace"), "{f}");
    assert!(f.contains("./pipelines/rivers:/pipelines"), "{f}");
    assert!(f.contains("./client/container.env"), "{f}");
    assert!(f.contains("UV_PROJECT_ENVIRONMENT: /venv"), "{f}");
    assert!(f.contains("127.0.0.1:3000/healthz"), "{f}");
    serde_yaml::from_str::<serde_yaml::Value>(&f).expect("the fragment is valid YAML");

    let out = p.materialize().unwrap();
    let pipeline = out
        .files
        .iter()
        .find(|f| f.path == "pipelines/rivers/quickstart.py")
        .expect("the quickstart pipeline is written beside compose.yaml");
    assert!(pipeline.preserve && !pipeline.sensitive);
    assert!(pipeline.contents.contains("repo = rs.CodeRepository"));
}

#[test]
fn the_module_knob_picks_the_pipeline_module() {
    let mut selection = Selection::modules(["rivers"]);
    selection.knob_overrides = BTreeMap::from([(
        ModuleId::from("rivers"),
        BTreeMap::from([("module".to_string(), "nightly".to_string())]),
    )]);
    let p = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    assert!(fragment(&p).contains("rivers dev nightly"));
}
