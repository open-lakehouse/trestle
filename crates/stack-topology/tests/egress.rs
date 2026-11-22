//! The egress gateway in the planner: routes from `[selection.egress]`, the egress Envoy and
//! secret server they render, the URLs their consumers get, and what binding a Neon branch must
//! supply for a function route.
//!
//! The Envoy golden refreshes with `UPDATE_GOLDENS=1 cargo test -p olai-stack-topology --test
//! egress`. `TRESTLE_DOCKER_TESTS=1` also runs the rendered bootstrap through
//! `envoy --mode validate` at the pinned image.

use std::path::PathBuf;

use olai_stack_topology::{
    EgressRoute, EgressSelection, InjectedCredential, MaterializedOutput, McpRoute, ModuleId, Plan,
    PlanCtx, PlanError, Secrets, Selection, baseline_catalog,
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
