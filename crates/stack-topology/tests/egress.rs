//! The egress gateway in the planner: routes from `[selection.egress]`, the egress Envoy and
//! secret server they render, the URLs their consumers get, and what binding a Neon branch must
//! supply for a function route.
//!
//! The Envoy golden refreshes with `UPDATE_GOLDENS=1 cargo test -p olai-stack-topology --test
//! egress`. `TRESTLE_DOCKER_TESTS=1` also runs the rendered bootstrap through
//! `envoy --mode validate` at the pinned image.

use std::path::PathBuf;
use std::sync::Arc;

use olai_stack_topology::{
    DataModule, DeclaredSecret, EgressRoute, EgressSelection, InjectedCredential,
    MaterializedOutput, McpRoute, ModuleId, Placement, Plan, PlanCtx, PlanError, Provides,
    RenderSpec, Role, SdsSecret, Secrets, Selection, ServiceSpec, baseline_catalog,
};

const PLANNER_KEY: &str = "WORKGRAPH_PLANNER_API_KEY";
const SEARCH_KEY: &str = "SEARCH_API_KEY";
const OAUTH_SECRET: &str = "MODELS_CLIENT_SECRET";

fn function_route() -> EgressRoute {
    EgressRoute {
        name: "workgraph-planner".into(),
        function: Some("workgraph".into()),
        upstream: None,
        path: "/mcp".into(),
        consumers: vec![ModuleId::from("omnigent")],
        credential: InjectedCredential::Bearer {
            secret: PLANNER_KEY.into(),
        },
        mcp: Some(McpRoute {
            title: "Work graph (planner)".into(),
            tools: vec!["read".into(), "submit".into()],
        }),
    }
}

fn upstream_route() -> EgressRoute {
    EgressRoute {
        name: "search".into(),
        function: None,
        upstream: Some("https://api.search.example.com".into()),
        path: "/v1".into(),
        consumers: vec![ModuleId::from("omnigent")],
        credential: InjectedCredential::Header {
            name: "x-api-key".into(),
            secret: SEARCH_KEY.into(),
            prefix: None,
        },
        mcp: None,
    }
}

fn oauth_route() -> EgressRoute {
    EgressRoute {
        name: "models".into(),
        function: None,
        upstream: Some("https://models.example.com".into()),
        path: "/".into(),
        consumers: vec![ModuleId::from("omnigent")],
        credential: InjectedCredential::OAuth2ClientCredentials {
            token_url: "https://login.example.com/oauth2/token".into(),
            client_id: "agents".into(),
            client_secret: OAUTH_SECRET.into(),
            scopes: vec!["models.invoke".into()],
        },
        mcp: None,
    }
}

/// Omnigent, with its own model keys (no MLflow), calling out through the egress gateway.
fn omnigent() -> Selection {
    let mut selection = Selection::modules(["omnigent"]);
    selection.knob_overrides.insert(
        ModuleId::from("omnigent"),
        [("upstream".to_string(), "keys".to_string())].into(),
    );
    selection
}

fn selection(routes: Vec<EgressRoute>) -> Selection {
    let mut selection = omnigent();
    selection.egress = EgressSelection {
        enabled: None,
        routes,
    };
    selection
}

/// What binding the branch supplies: the function's URL.
fn branch() -> Secrets {
    [(
        "WORKGRAPH_FUNCTION_URL",
        "https://br-x-workgraph.compute.example.neon.tech/",
    )]
    .into_iter()
    .collect()
}

fn plan(selection: &Selection, secrets: Option<Secrets>) -> Result<Plan, PlanError> {
    let ctx = PlanCtx {
        env_name: "egress".into(),
        secrets,
        ..PlanCtx::default()
    };
    baseline_catalog().plan(selection, &ctx)
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> &'a str {
    &out.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("no {path} rendered"))
        .contents
}

fn yaml(text: &str) -> serde_yaml::Value {
    serde_yaml::from_str(text).unwrap_or_else(|e| panic!("{e}:\n{text}"))
}

fn rendered(routes: Vec<EgressRoute>) -> (Plan, MaterializedOutput) {
    let plan = plan(&selection(routes), Some(branch())).unwrap();
    let out = plan.materialize().unwrap();
    (plan, out)
}

#[test]
fn a_function_route_renders_an_egress_envoy_reached_only_in_network() {
    let (plan, out) = rendered(vec![function_route(), upstream_route()]);
    assert!(plan.stack_facts.has("egress-gateway"));

    let fragment = yaml(file(&out, "modules/egress-gateway/compose.yaml"));
    let envoy = &fragment["services"]["egress-gateway"];
    assert_eq!(envoy["image"].as_str(), Some("envoyproxy/envoy:v1.40.2"));
    for service in ["egress-gateway", "secret-server"] {
        let s = &fragment["services"][service];
        assert!(s["ports"].is_null(), "{service} publishes a port");
        assert!(s["networks"].is_null(), "{service} joins a network");
    }
    let probe = envoy["healthcheck"]["test"][3].as_str().unwrap();
    assert!(probe.contains("/dev/tcp/127.0.0.1/9901"), "{probe}");
    assert_eq!(
        envoy["volumes"][0].as_str(),
        Some("sds-egress-gateway:/run/trestle-sds")
    );
    assert_eq!(
        fragment["services"]["secret-server"]["volumes"][0].as_str(),
        Some("sds-egress-gateway:/run/trestle-sds/egress-gateway")
    );
    assert!(fragment["volumes"]["sds-egress-gateway"].is_null());
    assert!(
        fragment["volumes"]
            .as_mapping()
            .unwrap()
            .contains_key("sds-egress-gateway")
    );

    let bootstrap = yaml(file(&out, "modules/egress-gateway/envoy.yaml"));
    let admin = &bootstrap["admin"]["address"]["socket_address"];
    assert_eq!(admin["address"].as_str(), Some("127.0.0.1"));
    let listeners = bootstrap["static_resources"]["listeners"]
        .as_sequence()
        .unwrap();
    let ports: Vec<u64> = listeners
        .iter()
        .map(|l| {
            l["address"]["socket_address"]["port_value"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert_eq!(ports, [10001, 10002]);
    // The function's URL, resolved from the branch, reached over verified TLS.
    let text = file(&out, "modules/egress-gateway/envoy.yaml");
    assert!(
        text.contains("address: br-x-workgraph.compute.example.neon.tech"),
        "{text}"
    );
    assert!(
        text.contains("sni: br-x-workgraph.compute.example.neon.tech"),
        "{text}"
    );
    assert!(text.contains("type: LOGICAL_DNS"), "{text}");
    assert!(text.contains("auto_host_rewrite: true"), "{text}");
    assert!(text.contains("overwrite: true"), "{text}");
    assert!(
        text.contains("allow_request_without_credential: false"),
        "{text}"
    );
    assert!(
        text.contains("pipe: { path: /run/trestle-sds/sds.sock }"),
        "{text}"
    );

    // The secret server's configuration, as the server reads it.
    let config = yaml(file(&out, "modules/egress-gateway/secret-server.yaml"));
    assert_eq!(config["version"].as_u64(), Some(1));
    let listener = &config["listeners"][0];
    assert_eq!(
        listener["socket"].as_str(),
        Some("/run/trestle-sds/egress-gateway/sds.sock")
    );
    assert_eq!(listener["consumer"].as_str(), Some("egress-gateway"));
    assert_eq!(
        listener["secrets"][0]["name"].as_str(),
        Some("workgraph-planner")
    );
    assert_eq!(listener["secrets"][0]["env"].as_str(), Some(PLANNER_KEY));
    assert_eq!(listener["secrets"][1]["env"].as_str(), Some(SEARCH_KEY));
}

#[test]
fn consumers_get_each_route_url_and_nothing_else() {
    let (_, out) = rendered(vec![function_route(), upstream_route()]);
    let head = file(&out, "compose.yaml");
    assert!(
        head.contains(
            "  - path:\n      - ./modules/omnigent/compose.yaml\n      - ./modules/egress-gateway/consumers/omnigent.yaml\n"
        ),
        "{head}"
    );
    let overrides = yaml(file(&out, "modules/egress-gateway/consumers/omnigent.yaml"));
    let env = &overrides["services"]["omnigent"]["environment"];
    assert_eq!(
        env["EGRESS_WORKGRAPH_PLANNER_URL"].as_str(),
        Some("http://egress-gateway:10001/mcp")
    );
    assert_eq!(
        env["EGRESS_SEARCH_URL"].as_str(),
        Some("http://egress-gateway:10002/v1")
    );
    // Only the module's services: no stray service definitions for Compose to merge.
    let services: Vec<&str> = overrides["services"]
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert!(
        services.iter().all(|s| s.starts_with("omnigent")),
        "{services:?}"
    );
    // The host can't reach the egress gateway, so its clients get no route.
    assert!(!file(&out, "client/host.env").contains("EGRESS_"));
}

#[test]
fn a_function_route_is_a_neon_requirement() {
    let plan = plan(&selection(vec![function_route()]), None).unwrap();
    let functions = plan.neon_requirements().functions;
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].slug, "workgraph");
    assert_eq!(functions[0].module.as_str(), "egress-gateway");
    assert_eq!(
        functions[0].url_var.as_deref(),
        Some("WORKGRAPH_FUNCTION_URL")
    );
}

#[test]
fn an_unresolved_function_url_stops_the_gateway_until_rendered_with_it() {
    let plan = plan(&selection(vec![function_route()]), None).unwrap();
    let out = plan.materialize().unwrap();
    let envoy = file(&out, "modules/egress-gateway/envoy.yaml");
    assert!(
        envoy.contains("address: ${WORKGRAPH_FUNCTION_URL}"),
        "{envoy}"
    );
    let fragment = file(&out, "modules/egress-gateway/compose.yaml");
    assert!(
        fragment.contains("TRESTLE_EGRESS_RENDERED: \"${TRESTLE_EGRESS_RENDERED:?"),
        "{fragment}"
    );
    // With the branch's URL, the guard goes away.
    let (_, out) = rendered(vec![function_route()]);
    assert!(!file(&out, "modules/egress-gateway/compose.yaml").contains("TRESTLE_EGRESS"));
}

#[test]
fn route_secrets_are_declared_on_the_egress_module_alone() {
    let (plan, _) = rendered(vec![function_route(), upstream_route(), oauth_route()]);
    let declaring: Vec<&str> = plan
        .secrets
        .iter()
        .filter(|(_, s)| {
            s.iter()
                .any(|s| [PLANNER_KEY, SEARCH_KEY, OAUTH_SECRET].contains(&s.name.as_str()))
        })
        .map(|(m, _)| m.as_str())
        .collect();
    assert_eq!(declaring, ["egress-gateway"]);
    let declared = plan.declared_secrets();
    for var in [PLANNER_KEY, SEARCH_KEY, OAUTH_SECRET] {
        let secret = declared.iter().find(|s| s.name == var).unwrap();
        assert!(secret.required, "{var}");
    }
}

/// No rendered file names a route's secret but the secret server's: its service in the egress
/// fragment, and its configuration.
#[test]
fn only_the_secret_server_names_a_route_secret() {
    let (_, out) = rendered(vec![function_route(), upstream_route(), oauth_route()]);
    for f in &out.files {
        for var in [PLANNER_KEY, SEARCH_KEY, OAUTH_SECRET] {
            if !f.contents.contains(var) {
                continue;
            }
            match f.path.as_str() {
                "modules/egress-gateway/secret-server.yaml" => {}
                "modules/egress-gateway/compose.yaml" => {
                    let fragment = yaml(&f.contents);
                    let envoy =
                        serde_yaml::to_string(&fragment["services"]["egress-gateway"]).unwrap();
                    assert!(!envoy.contains(var), "the egress Envoy names {var}");
                    let server = &fragment["services"]["secret-server"]["environment"];
                    assert!(
                        server.as_mapping().unwrap().contains_key(var),
                        "{var} not passed to the secret server"
                    );
                }
                other => panic!("{other} names the route secret {var}"),
            }
        }
    }
}

#[test]
fn an_unknown_consumer_is_a_plan_error() {
    let mut route = function_route();
    route.consumers = vec![ModuleId::from("mlflow")];
    let err = plan(&selection(vec![route]), None).unwrap_err();
    assert!(
        matches!(&err, PlanError::UnknownEgressConsumer { route, module, .. }
            if route == "workgraph-planner" && module.as_str() == "mlflow"),
        "{err}"
    );
}

#[test]
fn a_consumer_running_nothing_in_the_environment_is_a_plan_error() {
    let mut selection = selection(vec![]);
    selection.modules.push(ModuleId::from("neon-storage"));
    let mut route = function_route();
    route.consumers = vec![ModuleId::from("neon-storage")];
    selection.egress.routes = vec![route];
    let err = plan(&selection, None).unwrap_err();
    assert!(
        matches!(&err, PlanError::UnknownEgressConsumer { module, .. } if module.as_str() == "neon-storage"),
        "{err}"
    );
}

#[test]
fn a_malformed_function_is_a_plan_error() {
    let mut route = function_route();
    route.function = Some("Work_Graph".into());
    let err = plan(&selection(vec![route]), None).unwrap_err();
    assert!(
        matches!(&err, PlanError::UnknownEgressFunction { function, .. } if function == "Work_Graph"),
        "{err}"
    );
}

#[test]
fn the_function_of_a_module_running_as_a_container_is_a_plan_error() {
    let mut selection = selection(vec![]);
    selection.modules.push(ModuleId::from("unity-catalog-rs"));
    let mut route = function_route();
    route.function = Some("unitycatalog".into());
    selection.egress.routes = vec![route];
    let err = plan(&selection, None).unwrap_err();
    assert!(
        matches!(&err, PlanError::UnknownEgressFunction { function, reason, .. }
            if function == "unitycatalog" && reason.contains("unity-catalog-rs")),
        "{err}"
    );
}

#[test]
fn malformed_routes_are_plan_errors() {
    let cases: Vec<(EgressRoute, &str)> = vec![
        (
            EgressRoute {
                upstream: Some("https://x.example.com".into()),
                ..function_route()
            },
            "exactly one",
        ),
        (
            EgressRoute {
                path: "mcp".into(),
                ..function_route()
            },
            "start with `/`",
        ),
        (
            EgressRoute {
                credential: InjectedCredential::Bearer {
                    secret: "sk-live-1234".into(),
                },
                ..function_route()
            },
            "never hold a value",
        ),
        (
            EgressRoute {
                function: None,
                upstream: Some("https://api.example.com/v1".into()),
                ..function_route()
            },
            "origin",
        ),
    ];
    for (route, reason) in cases {
        let err = plan(&selection(vec![route]), None).unwrap_err();
        assert!(
            matches!(&err, PlanError::InvalidEgressRoute { reason: r, .. } if r.contains(reason)),
            "{err}"
        );
    }
    let err = plan(&selection(vec![function_route(), function_route()]), None).unwrap_err();
    assert!(
        matches!(&err, PlanError::InvalidEgressRoute { reason, .. } if reason.contains("taken")),
        "{err}"
    );
}

#[test]
fn egress_turned_on_without_routes_still_runs_the_gateway() {
    let mut selection = selection(vec![]);
    selection.egress.enabled = Some(true);
    let plan = plan(&selection, None).unwrap();
    let out = plan.materialize().unwrap();
    let bootstrap = yaml(file(&out, "modules/egress-gateway/envoy.yaml"));
    assert!(
        bootstrap["static_resources"]["listeners"]
            .as_sequence()
            .unwrap()
            .is_empty()
    );
    // And without egress, nothing of it.
    let plan = self::plan(&omnigent(), None).unwrap();
    assert!(plan.egress.is_none());
    assert!(!plan.stack_facts.has("egress-gateway"));
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/egress")
}

#[test]
fn the_egress_bootstrap_matches_its_golden() {
    let (_, out) = rendered(vec![function_route(), upstream_route()]);
    let envoy = file(&out, "modules/egress-gateway/envoy.yaml");
    let path = golden_dir().join("envoy.yaml");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, envoy).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (UPDATE_GOLDENS=1 writes it)", path.display()));
    assert_eq!(
        envoy, golden,
        "the egress bootstrap changed; UPDATE_GOLDENS=1 refreshes it"
    );
}

/// `envoy --mode validate` on the rendered bootstrap, at the pinned image: every route kind,
/// including OAuth2. Needs Docker; runs with `TRESTLE_DOCKER_TESTS=1`.
#[test]
fn the_egress_bootstrap_validates_in_envoy() {
    if std::env::var_os("TRESTLE_DOCKER_TESTS").is_none() {
        eprintln!("skipped: set TRESTLE_DOCKER_TESTS=1 to validate with Docker");
        return;
    }
    let (plan, out) = rendered(vec![function_route(), upstream_route(), oauth_route()]);
    let image = plan.bindings[&ModuleId::from("egress-gateway")]
        .knobs
        .get("image")
        .unwrap()
        .to_string();
    // Inline, not mounted: a VM-backed engine (Colima) doesn't share the temp dir.
    let output = std::process::Command::new("docker")
        .args(["run", "--rm", &image, "--mode", "validate", "--config-yaml"])
        .arg(file(&out, "modules/egress-gateway/envoy.yaml"))
        .output()
        .expect("docker runs");
    assert!(
        output.status.success(),
        "envoy rejected the bootstrap:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_graph_draws_the_egress_gateway_between_its_consumers_and_upstreams() {
    use olai_stack_topology::EdgeAuth;
    use olai_stack_topology::projection::{NodeKind, TopologyEdgeDto, graph_dto, graph_mermaid};
    let (plan, _) = rendered(vec![function_route(), upstream_route()]);
    let graph = graph_dto(&plan);
    let node = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("no node {id}"))
    };
    assert_eq!(node("egress-gateway").kind, NodeKind::Egress);
    assert!(node("egress-gateway").surfaces.is_empty());
    let function = node("@neon-workgraph");
    assert_eq!(function.kind, NodeKind::External);
    assert_eq!(function.cloud.as_deref(), Some("neon"));
    assert_eq!(node("@upstream/api.search.example.com").cloud, None);

    let uses = |from: &str, to: &str| {
        graph
            .edges
            .iter()
            .find_map(|e| match e {
                TopologyEdgeDto::Uses {
                    from: f,
                    to: t,
                    resources,
                    auth,
                    ..
                } if f == from && t == to => Some((resources.clone(), *auth)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no uses edge {from} → {to}"))
    };
    assert_eq!(
        uses("omnigent", "egress-gateway"),
        (vec!["workgraph-planner".into(), "search".into()], None)
    );
    assert_eq!(
        uses("egress-gateway", "@neon-workgraph"),
        (vec!["workgraph-planner".into()], Some(EdgeAuth::Injected))
    );
    assert_eq!(
        uses("egress-gateway", "@upstream/api.search.example.com").1,
        Some(EdgeAuth::Injected)
    );
    // Nothing reaches the egress gateway from the host.
    assert!(!graph.edges.iter().any(|e| matches!(e,
        TopologyEdgeDto::Access { to, .. } if to == "egress-gateway")));

    let mermaid = graph_mermaid(&graph);
    assert!(
        mermaid.contains("HTTPS: workgraph-planner · injected"),
        "{mermaid}"
    );
}

#[test]
fn a_route_to_a_function_the_stack_hosts_reaches_its_module() {
    use olai_stack_topology::projection::{TopologyEdgeDto, graph_dto};
    use olai_stack_topology::{HostedConnection, HostedResource, HostedVendor};
    let mut selection = selection(vec![]);
    selection.modules.push(ModuleId::from("unity-catalog-rs"));
    selection.hosted.insert(
        ModuleId::from("unity-catalog-rs"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::NeonFunction),
            connection: HostedConnection::Function { url: None },
        },
    );
    let mut route = function_route();
    route.function = Some("unitycatalog".into());
    selection.egress.routes = vec![route];
    let plan = plan(&selection, None).unwrap();
    // Reached at the hosted module's own URL, and required once.
    let route = &plan.egress.as_ref().unwrap().routes[0];
    assert_eq!(route.origin, "${UNITYCATALOG_FUNCTION_URL}");
    let functions = plan.neon_requirements().functions;
    assert_eq!(
        functions
            .iter()
            .filter(|f| f.slug == "unitycatalog")
            .count(),
        1
    );
    let graph = graph_dto(&plan);
    assert!(graph.edges.iter().any(|e| matches!(e,
        TopologyEdgeDto::Uses { from, to, .. }
            if from == "egress-gateway" && to == "unity-catalog-rs")));
    assert!(!graph.nodes.iter().any(|n| n.id == "@neon-unitycatalog"));
}

/// A module that calls a fixed outside upstream declares it as an outside endpoint with an
/// injected credential: the planner adds the egress gateway and a route for it, with the
/// module as its one consumer.
#[test]
fn a_module_declaring_an_injected_credential_gets_a_route() {
    use std::sync::Arc;

    use olai_stack_topology::{
        DataModule, Endpoint, Placement, RenderSpec, Rewrite, Role, RouteIntent, RouteOptions,
        Scheme, ServiceSpec,
    };
    let caller = DataModule {
        id: ModuleId::from("caller"),
        model: None,
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![
            ServiceSpec {
                name: "caller".into(),
                role: Role::new("caller"),
                placement: Placement::Container {
                    service: "caller".into(),
                },
                endpoints: vec![],
                depends_on: vec![],
                base_path: String::new(),
            },
            ServiceSpec {
                name: "models".into(),
                role: Role::new("models"),
                placement: Placement::External {
                    origin: "https://models.example.com".into(),
                },
                endpoints: vec![
                    Endpoint {
                        id: "api".into(),
                        scheme: Scheme::Https,
                        internal_port: 443,
                        host_port: None,
                        intent: RouteIntent::Internal,
                        path: String::new(),
                        mount_prefix: None,
                        rewrite: Rewrite::Inherit,
                        route: RouteOptions::default(),
                    }
                    .inject_credential(InjectedCredential::Bearer {
                        secret: "MODELS_API_KEY".into(),
                    }),
                ],
                depends_on: vec![],
                base_path: "/v1".into(),
            },
        ],
        provides: Default::default(),
        knobs: vec![],
        renders: RenderSpec {
            fragment: "services:\n  caller:\n    image: busybox\n".into(),
            files: vec![],
        }
        .for_compose(),
    };
    let mut catalog = baseline_catalog();
    catalog.insert(Arc::new(caller));
    let plan = catalog
        .plan(&Selection::modules(["caller"]), &PlanCtx::default())
        .unwrap();
    plan.materialize().unwrap();
    let egress = plan.egress.as_ref().expect("the egress gateway joins");
    let route = &egress.routes[0];
    assert_eq!(route.name, "caller-api");
    assert_eq!(route.path, "/v1");
    assert_eq!(route.consumers, [ModuleId::from("caller")]);
    assert_eq!(route.upstream.host, "models.example.com");
    assert_eq!(
        egress.consumer_env(&ModuleId::from("caller"))["EGRESS_CALLER_API_URL"],
        "http://egress-gateway:10001/v1"
    );
    assert_eq!(
        plan.secrets[&ModuleId::from("egress-gateway")][0].name,
        "MODELS_API_KEY"
    );
}

// ---- Omnigent's MCP registry ----

const REGISTRY: &str = "modules/omnigent/mcp-registry.yaml";
const REGISTRY_TARGET: &str = "/etc/trestle/omnigent/mcp-registry.yaml";
const DOCS_KEY: &str = "DOCS_API_KEY";

/// A second MCP route, to an upstream origin.
fn docs_route() -> EgressRoute {
    EgressRoute {
        name: "docs_search".into(),
        function: None,
        upstream: Some("https://docs.example.com:8443".into()),
        path: "/v1/mcp".into(),
        consumers: vec![ModuleId::from("omnigent")],
        credential: InjectedCredential::Bearer {
            secret: DOCS_KEY.into(),
        },
        mcp: Some(McpRoute {
            title: "Docs search".into(),
            tools: vec!["query".into()],
        }),
    }
}

fn omnigent_server(out: &MaterializedOutput) -> serde_yaml::Value {
    yaml(file(out, "modules/omnigent/compose.yaml"))["services"]["omnigent"].clone()
}

#[test]
fn omnigent_registry_lists_each_mcp_route_through_the_egress() {
    let (_, out) = rendered(vec![function_route(), upstream_route(), docs_route()]);
    let registry = yaml(file(&out, REGISTRY));
    let expected = yaml(
        r#"
services:
  - id: workgraph-planner
    title: Work graph (planner)
    url: http://egress-gateway:10001/mcp
    auth: none
    allow_http: true
    tools: [read, submit]
  - id: docs_search
    title: Docs search
    url: http://egress-gateway:10003/v1/mcp
    auth: none
    allow_http: true
    tools: [query]
"#,
    );
    // The `search` route serves no MCP, so the registry leaves it out.
    assert_eq!(registry, expected);

    let server = omnigent_server(&out);
    assert_eq!(
        server["environment"]["OMNIGENT_MCP_REGISTRY"].as_str(),
        Some(REGISTRY_TARGET)
    );
    assert_eq!(
        server["configs"][0]["source"].as_str(),
        Some("omnigent_mcp_registry")
    );
    assert_eq!(
        server["configs"][0]["target"].as_str(),
        Some(REGISTRY_TARGET)
    );
    // The registry's calls leave from the server: the host gets none of it.
    let host = &yaml(file(&out, "modules/omnigent/compose.yaml"))["services"]["omnigent-host"];
    assert!(host["environment"]["OMNIGENT_MCP_REGISTRY"].is_null());
    let head = yaml(file(&out, "compose.yaml"));
    assert_eq!(
        head["configs"]["omnigent_mcp_registry"]["file"].as_str(),
        Some("./modules/omnigent/mcp-registry.yaml")
    );
}

#[test]
fn without_an_mcp_route_omnigent_gets_no_registry() {
    for routes in [vec![], vec![upstream_route()]] {
        let (_, out) = rendered(routes.clone());
        assert!(
            !out.files.iter().any(|f| f.path == REGISTRY),
            "{routes:?}: a registry was rendered"
        );
        let server = omnigent_server(&out);
        assert!(server["environment"]["OMNIGENT_MCP_REGISTRY"].is_null());
        assert!(server["configs"].is_null());
        assert!(!file(&out, "compose.yaml").contains("omnigent_mcp_registry"));
    }
}

/// With every route secret set, neither the registry nor anything Omnigent runs with holds a
/// secret's value or names its variable: the egress gateway adds the credential.
#[test]
fn omnigent_holds_no_route_secret() {
    let sentinels = [
        (PLANNER_KEY, "sentinel-planner-7f3a"),
        (DOCS_KEY, "sentinel-docs-91c2"),
    ];
    let mut secrets = branch();
    for (var, value) in sentinels {
        secrets.insert(var.to_string(), value.to_string());
    }
    let plan = plan(
        &selection(vec![function_route(), docs_route()]),
        Some(secrets),
    )
    .unwrap();
    let out = plan.materialize().unwrap();
    let omnigent_files: Vec<_> = out
        .files
        .iter()
        .filter(|f| {
            f.path.starts_with("modules/omnigent/")
                || f.path == "modules/egress-gateway/consumers/omnigent.yaml"
        })
        .collect();
    assert!(omnigent_files.iter().any(|f| f.path == REGISTRY));
    assert!(
        omnigent_files
            .iter()
            .any(|f| f.path == "modules/omnigent/.env/omnigent.env")
    );
    for f in omnigent_files {
        for (var, value) in sentinels {
            assert!(
                !f.contents.contains(value),
                "{} holds {var}'s value",
                f.path
            );
            assert!(!f.contents.contains(var), "{} names {var}", f.path);
        }
    }
    // Nor does the plan hand Omnigent's render anything about the credential.
    let binding = serde_json::to_string(&plan.bindings[&ModuleId::from("omnigent")]).unwrap();
    for (var, value) in sentinels {
        assert!(
            !binding.contains(value) && !binding.contains(var),
            "{binding}"
        );
    }
}

#[test]
fn an_mcp_route_omnigent_cannot_register_is_a_plan_error() {
    let too_long = format!("a{}", "b".repeat(40));
    for name in ["Planner", "a__b", too_long.as_str()] {
        let mut route = docs_route();
        route.name = name.into();
        let err = plan(&selection(vec![route]), None).unwrap_err();
        assert!(
            matches!(&err, PlanError::InvalidEgressRoute { route, reason }
                if route == name && reason.contains("Omnigent's MCP registry")),
            "{name}: {err}"
        );
    }
    // At the limit, the id is fine.
    let mut route = docs_route();
    route.name = format!("a{}", "b".repeat(39));
    assert!(plan(&selection(vec![route]), None).is_ok());
    let mut route = docs_route();
    route.mcp.as_mut().unwrap().tools.clear();
    let err = plan(&selection(vec![route]), None).unwrap_err();
    assert!(
        matches!(&err, PlanError::InvalidEgressRoute { route, reason }
            if route == "docs_search" && reason.contains("mcp.tools")),
        "{err}"
    );
}

/// Omnigent's rules bind only the routes it consumes: an MCP route named any way, with no tool
/// list, is fine for another consumer, and a non-MCP route for Omnigent is too.
#[test]
fn omnigent_rules_bind_only_its_mcp_routes() {
    let mut other = docs_route();
    other.name = "Catalog__Tools".into();
    other.consumers = vec![ModuleId::from("uc-mcp")];
    other.mcp.as_mut().unwrap().tools.clear();
    let mut not_mcp = upstream_route();
    not_mcp.name = "Search__Index".into();
    let mut selection = selection(vec![other, not_mcp]);
    selection
        .modules
        .extend([ModuleId::from("uc-mcp"), ModuleId::from("unity-catalog-rs")]);
    let out = plan(&selection, None).unwrap().materialize().unwrap();
    assert!(!out.files.iter().any(|f| f.path == REGISTRY));
}

/// The registry is an ordinary configuration file: the plan's projection lists it with the
/// environment's other files, so the editor shows it on Omnigent's node.
#[test]
fn the_registry_is_a_visible_file_of_the_plan() {
    use olai_stack_topology::projection::plan_result;
    let paths = |routes| -> Vec<String> {
        plan_result(&selection(routes), None)
            .unwrap()
            .files
            .into_iter()
            .map(|f| f.path)
            .collect()
    };
    assert!(paths(vec![docs_route()]).iter().any(|p| p == REGISTRY));
    assert!(!paths(vec![upstream_route()]).iter().any(|p| p == REGISTRY));
}

/// The variable holding the secret the `sds-app` test module reads over SDS.
const SDS_KEY: &str = "SDS_APP_SERVER_KEK";

/// A module whose `app` container reads `server-kek` over SDS, held by `SDS_KEY`, and also
/// reads `SDS_APP_OTHER` from its environment as usual.
fn sds_module(sds_secrets: Vec<SdsSecret>) -> DataModule {
    DataModule {
        id: ModuleId::from("sds-app"),
        model: None,
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "app".into(),
            role: Role::new("app"),
            placement: Placement::Container {
                service: "app".into(),
            },
            endpoints: vec![],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides: Provides {
            secrets: vec![
                DeclaredSecret::required(SDS_KEY, "The key-encryption key."),
                DeclaredSecret::optional("SDS_APP_OTHER", "Read from the environment."),
            ],
            sds_secrets,
            ..Provides::default()
        },
        knobs: vec![],
        renders: RenderSpec {
            fragment: "services:\n  app:\n    image: example/app:1\n    environment:\n      SDS_APP_OTHER:\n".into(),
            files: vec![],
        }
        .for_compose(),
    }
}

fn sds_plan(module: DataModule, routes: Vec<EgressRoute>) -> Result<Plan, PlanError> {
    let mut catalog = baseline_catalog();
    catalog.insert(Arc::new(module));
    let mut selection = Selection::modules(["sds-app"]);
    if !routes.is_empty() {
        selection.modules.push(ModuleId::from("omnigent"));
        selection.knob_overrides.insert(
            ModuleId::from("omnigent"),
            [("upstream".to_string(), "keys".to_string())].into(),
        );
        selection.egress.routes = routes;
    }
    let ctx = PlanCtx {
        env_name: "sds".into(),
        secrets: Some(branch()),
        ..PlanCtx::default()
    };
    catalog.plan(&selection, &ctx)
}

/// A module's SDS secret pulls in the secret server, which serves it on the module's own
/// socket; the variable moves to the secret server alone, and the module's containers mount
/// the socket.
#[test]
fn a_modules_sds_secret_appears_only_on_the_secret_server() {
    let plan = sds_plan(
        sds_module(vec![SdsSecret::new("server-kek", SDS_KEY)]),
        vec![upstream_route()],
    )
    .unwrap();
    assert!(plan.stack_facts.has("egress-gateway"));

    // Declared on the egress module alone, still required; the module keeps its other secret.
    let declaring: Vec<&str> = plan
        .secrets
        .iter()
        .filter(|(_, s)| s.iter().any(|s| s.name == SDS_KEY))
        .map(|(m, _)| m.as_str())
        .collect();
    assert_eq!(declaring, ["egress-gateway"]);
    let own: Vec<&str> = plan.secrets[&ModuleId::from("sds-app")]
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(own, ["SDS_APP_OTHER"]);
    assert!(
        plan.declared_secrets()
            .iter()
            .any(|s| s.name == SDS_KEY && s.required)
    );

    let out = plan.materialize().unwrap();
    // The server listens for both consumers: the Envoy (the route) and the module.
    let config = yaml(file(&out, "modules/egress-gateway/secret-server.yaml"));
    let listeners = config["listeners"].as_sequence().unwrap();
    assert_eq!(listeners.len(), 2);
    assert_eq!(listeners[0]["consumer"].as_str(), Some("egress-gateway"));
    assert_eq!(listeners[0]["secrets"][0]["name"].as_str(), Some("search"));
    assert_eq!(listeners[1]["consumer"].as_str(), Some("sds-app"));
    assert_eq!(
        listeners[1]["socket"].as_str(),
        Some("/run/trestle-sds/sds-app/sds.sock")
    );
    assert_eq!(
        listeners[1]["secrets"][0]["name"].as_str(),
        Some("server-kek")
    );
    assert_eq!(listeners[1]["secrets"][0]["env"].as_str(), Some(SDS_KEY));

    // The module's container mounts its socket volume, which the server mounts too.
    let overlay = yaml(file(&out, "modules/egress-gateway/consumers/sds-app.yaml"));
    let app = &overlay["services"]["app"];
    assert_eq!(
        app["volumes"][0].as_str(),
        Some("sds-sds-app:/run/trestle-sds")
    );
    assert_eq!(
        app["environment"]["TRESTLE_SDS_SOCKET"].as_str(),
        Some("/run/trestle-sds/sds.sock")
    );
    assert_eq!(
        app["depends_on"]["secret-server"]["condition"].as_str(),
        Some("service_started")
    );
    let fragment = yaml(file(&out, "modules/egress-gateway/compose.yaml"));
    let server_volumes =
        serde_yaml::to_string(&fragment["services"]["secret-server"]["volumes"]).unwrap();
    assert!(
        server_volumes.contains("sds-sds-app:/run/trestle-sds/sds-app"),
        "{server_volumes}"
    );
    // Envoy mounts only its own socket.
    let envoy = serde_yaml::to_string(&fragment["services"]["egress-gateway"]).unwrap();
    assert!(!envoy.contains("sds-sds-app"), "{envoy}");

    // No rendered file names the variable but the secret server's.
    for f in &out.files {
        if !f.contents.contains(SDS_KEY) {
            continue;
        }
        match f.path.as_str() {
            "modules/egress-gateway/secret-server.yaml" => {}
            "modules/egress-gateway/compose.yaml" => {
                let fragment = yaml(&f.contents);
                for (name, service) in fragment["services"].as_mapping().unwrap() {
                    let text = serde_yaml::to_string(service).unwrap();
                    assert!(
                        !text.contains(SDS_KEY) || name.as_str() == Some("secret-server"),
                        "{name:?} names {SDS_KEY}"
                    );
                }
                let env = &fragment["services"]["secret-server"]["environment"];
                assert!(env.as_mapping().unwrap().contains_key(SDS_KEY));
            }
            other => panic!("{other} names {SDS_KEY}"),
        }
    }
}

/// Without any route, a module's SDS secret still brings in the secret server.
#[test]
fn an_sds_secret_alone_plans_the_secret_server() {
    let plan = sds_plan(
        sds_module(vec![SdsSecret::new("server-kek", SDS_KEY)]),
        vec![],
    )
    .unwrap();
    let egress = plan.egress.as_ref().expect("the egress module is planned");
    assert!(egress.routes.is_empty());
    assert_eq!(egress.secrets, [SDS_KEY]);
    let names: Vec<&str> = egress
        .sds_consumers
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["egress-gateway", "sds-app"]);
    assert!(egress.sds_consumers[0].secrets.is_empty());
    let out = plan.materialize().unwrap();
    let config = yaml(file(&out, "modules/egress-gateway/secret-server.yaml"));
    assert_eq!(
        config["listeners"][1]["secrets"][0]["env"].as_str(),
        Some(SDS_KEY)
    );
}

#[test]
fn unusable_sds_secrets_are_plan_errors() {
    let cases = [
        (
            vec![SdsSecret::new("server-kek", "SDS_APP_UNDECLARED")],
            "not one of the module's declared secrets",
        ),
        (
            vec![
                SdsSecret::new("server-kek", SDS_KEY),
                SdsSecret::new("server-kek", SDS_KEY),
            ],
            "named twice",
        ),
        (
            vec![SdsSecret::new("9-bad name", SDS_KEY)],
            "letters, digits",
        ),
    ];
    for (secrets, reason) in cases {
        let err = sds_plan(sds_module(secrets), vec![]).unwrap_err();
        assert!(
            matches!(&err, PlanError::InvalidSdsSecret { module, .. } if module.as_str() == "sds-app"),
            "{err}"
        );
        assert!(err.to_string().contains(reason), "{err}");
    }
}

/// The variable the planner derives for the `sds-app` module's key `server-kek`.
const KEYRING_VAR: &str = "TRESTLE_KEYRING_SDS_APP__SERVER_KEK";

/// The `sds-app` module sealing under `keys` through the key service, reading no SDS secret.
fn keys_module(keys: &[&str]) -> DataModule {
    let mut module = sds_module(vec![]);
    module.provides.secrets = vec![DeclaredSecret::optional(
        "SDS_APP_OTHER",
        "Read from the environment.",
    )];
    module.provides.keys = keys.iter().map(|k| k.to_string()).collect();
    module
}

/// A module's key alone brings in the secret server, whose listener for the module lists it;
/// the keyring's variable is derived, declared on the secret server alone, and named by no
/// other rendered file, while the module gets its socket and the key's name.
#[test]
fn a_modules_key_is_held_by_the_secret_server_alone() {
    let plan = sds_plan(keys_module(&["server-kek"]), vec![]).unwrap();
    let egress = plan.egress.as_ref().expect("the egress module is planned");
    assert_eq!(
        olai_stack_topology::plan::keyring_var(&ModuleId::from("sds-app"), "server-kek"),
        KEYRING_VAR
    );
    assert_eq!(egress.secrets, [KEYRING_VAR]);
    let keys: Vec<(&str, &str, &str)> = egress
        .keys()
        .map(|(m, k)| (m.as_str(), k.name.as_str(), k.env.as_str()))
        .collect();
    assert_eq!(keys, [("sds-app", "server-kek", KEYRING_VAR)]);

    // Declared on the egress module alone, required; the module keeps its own secret.
    let declaring: Vec<&str> = plan
        .secrets
        .iter()
        .filter(|(_, s)| s.iter().any(|s| s.name == KEYRING_VAR))
        .map(|(m, _)| m.as_str())
        .collect();
    assert_eq!(declaring, ["egress-gateway"]);
    assert!(
        plan.declared_secrets()
            .iter()
            .any(|s| s.name == KEYRING_VAR && s.required)
    );
    let own: Vec<&str> = plan.secrets[&ModuleId::from("sds-app")]
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(own, ["SDS_APP_OTHER"]);

    let out = plan.materialize().unwrap();
    let config = yaml(file(&out, "modules/egress-gateway/secret-server.yaml"));
    let listener = &config["listeners"][1];
    assert_eq!(listener["consumer"].as_str(), Some("sds-app"));
    assert_eq!(listener["secrets"].as_sequence().map(Vec::len), Some(0));
    assert_eq!(listener["keys"][0]["name"].as_str(), Some("server-kek"));
    assert_eq!(listener["keys"][0]["env"].as_str(), Some(KEYRING_VAR));
    // The Envoy's listener lists no keys.
    assert!(config["listeners"][0].get("keys").is_none());

    let overlay = yaml(file(&out, "modules/egress-gateway/consumers/sds-app.yaml"));
    let app = &overlay["services"]["app"];
    assert_eq!(
        app["volumes"][0].as_str(),
        Some("sds-sds-app:/run/trestle-sds")
    );
    assert_eq!(
        app["environment"]["TRESTLE_SDS_SOCKET"].as_str(),
        Some("/run/trestle-sds/sds.sock")
    );
    assert_eq!(
        app["environment"]["TRESTLE_KEY_SERVER_KEK"].as_str(),
        Some("server-kek")
    );
    let fragment = yaml(file(&out, "modules/egress-gateway/compose.yaml"));
    let envoy = serde_yaml::to_string(&fragment["services"]["egress-gateway"]).unwrap();
    assert!(!envoy.contains("sds-sds-app"), "{envoy}");

    for f in &out.files {
        if !f.contents.contains(KEYRING_VAR) {
            continue;
        }
        match f.path.as_str() {
            "modules/egress-gateway/secret-server.yaml" => {}
            "modules/egress-gateway/compose.yaml" => {
                let fragment = yaml(&f.contents);
                for (name, service) in fragment["services"].as_mapping().unwrap() {
                    let text = serde_yaml::to_string(service).unwrap();
                    assert!(
                        !text.contains(KEYRING_VAR) || name.as_str() == Some("secret-server"),
                        "{name:?} names {KEYRING_VAR}"
                    );
                }
                let env = &fragment["services"]["secret-server"]["environment"];
                assert!(env.as_mapping().unwrap().contains_key(KEYRING_VAR));
            }
            other => panic!("{other} names {KEYRING_VAR}"),
        }
    }
}

/// A module's SDS secrets and keys share its one listener and socket.
#[test]
fn a_modules_secrets_and_keys_share_its_socket() {
    let mut module = sds_module(vec![SdsSecret::new("server-token", SDS_KEY)]);
    module.provides.keys = vec!["server-kek".into(), "audit-kek".into()];
    let plan = sds_plan(module, vec![]).unwrap();
    let egress = plan.egress.as_ref().unwrap();
    assert_eq!(
        egress.secrets,
        [SDS_KEY, KEYRING_VAR, "TRESTLE_KEYRING_SDS_APP__AUDIT_KEK"]
    );
    let out = plan.materialize().unwrap();
    let config = yaml(file(&out, "modules/egress-gateway/secret-server.yaml"));
    let listeners = config["listeners"].as_sequence().unwrap();
    assert_eq!(listeners.len(), 2);
    assert_eq!(
        listeners[1]["secrets"][0]["name"].as_str(),
        Some("server-token")
    );
    let keys: Vec<&str> = listeners[1]["keys"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|k| k["name"].as_str())
        .collect();
    assert_eq!(keys, ["server-kek", "audit-kek"]);
    let overlay = yaml(file(&out, "modules/egress-gateway/consumers/sds-app.yaml"));
    let env = &overlay["services"]["app"]["environment"];
    assert_eq!(env["TRESTLE_KEY_AUDIT_KEK"].as_str(), Some("audit-kek"));
    assert_eq!(env["TRESTLE_KEY_SERVER_KEK"].as_str(), Some("server-kek"));
}

#[test]
fn unusable_keys_are_plan_errors() {
    let long = "k".repeat(244);
    let cases: [(Vec<&str>, &str); 4] = [
        (vec!["9-bad name"], "letters, digits"),
        (vec!["server-kek", "server-kek"], "named twice"),
        (vec!["server-kek", "server_kek"], "share the variable"),
        (vec![long.as_str()], "at most 243 bytes"),
    ];
    for (keys, reason) in cases {
        let err = sds_plan(keys_module(&keys), vec![]).unwrap_err();
        assert!(
            matches!(&err, PlanError::InvalidModuleKey { module, .. } if module.as_str() == "sds-app"),
            "{err}"
        );
        assert!(err.to_string().contains(reason), "{err}");
    }
    // The longest name the secret server accepts plans.
    let longest = "k".repeat(243);
    sds_plan(keys_module(&[longest.as_str()]), vec![]).unwrap();
}
