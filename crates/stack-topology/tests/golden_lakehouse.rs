//! End-to-end planner validation against the inlined baseline catalog.
//!
//! This is the acceptance test for the module + planner layer: planning the default
//! lakehouse selection must re-derive — purely by rule — the gateway routes the
//! trestle `local-stack-*` components hand-author today. It also pins the two
//! behaviours the planner exists to guarantee: a real prefix collision fails loudly,
//! and the plan is deterministic regardless of selection order.

use olai_stack_topology::{
    Catalog, Endpoint, GatewayRoute, Module, ModuleId, Placement, PlanCtx, PlanError, Provides,
    RenderSpec, Role, RouteIntent, Scheme, Selection, ServiceSpec, TopologyCtx, Vantage, address,
    baseline_catalog, plan,
};

/// The always-on + common lakehouse modules.
fn default_selection() -> Selection {
    Selection::modules([
        "local-stack-envoy",
        "local-stack-postgres",
        "local-stack-seaweedfs",
        "local-stack-mlflow",
        "local-stack-unity-catalog",
    ])
}

fn route<'a>(routes: &'a [GatewayRoute], prefix: &str) -> &'a GatewayRoute {
    routes
        .iter()
        .find(|r| r.prefix == prefix)
        .unwrap_or_else(|| panic!("expected a gateway route for `{prefix}`"))
}

#[test]
fn default_lakehouse_rederives_the_working_gateway_routes() {
    let p = plan(
        &default_selection(),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .expect("default lakehouse should plan cleanly");

    // The shared listener is the gateway's host-published port.
    let shared = &p.gateway.listeners[0];
    assert_eq!(shared.host_port, 9080);
    assert_eq!(shared.internal_port, 10000);
    let routes = &shared.routes;

    // MLflow tracking API: Databricks-shaped client path, rewritten under /mlflow.
    let tracking = route(routes, "/api/2.0/mlflow");
    assert_eq!(tracking.cluster, "mlflow");
    assert_eq!(tracking.rewrite.as_deref(), Some("/mlflow/api/2.0/mlflow"));

    // MLflow OTel: the override exception — passes through unchanged (no rewrite).
    assert_eq!(route(routes, "/api/2.0/otel").rewrite, None);

    // MLflow UI: served at its base path, no rewrite.
    let ui = route(routes, "/mlflow");
    assert_eq!(ui.cluster, "mlflow");
    assert_eq!(ui.rewrite, None);

    // Unity Catalog REST: served at root → no rewrite; plus its short alias.
    let uc = route(routes, "/api/2.1/unity-catalog");
    assert_eq!(uc.cluster, "unitycatalog");
    assert_eq!(uc.rewrite, None);
    assert_eq!(route(routes, "/unity-catalog").cluster, "unitycatalog");

    // Routes are ordered most-specific-first (Envoy match priority).
    let lens: Vec<usize> = routes.iter().map(|r| r.prefix.len()).collect();
    let mut by_len = lens.clone();
    by_len.sort_by(|a, b| b.cmp(a));
    assert_eq!(lens, by_len, "routes must be longest-prefix-first");

    // Clusters are synthesized from each service's placement + port.
    let mlflow = p
        .gateway
        .clusters
        .iter()
        .find(|c| c.name == "mlflow")
        .unwrap();
    assert_eq!((mlflow.host.as_str(), mlflow.port), ("mlflow", 5000));

    // The UI's chosen base path is injected back for the module's render.
    assert_eq!(
        p.injected
            .get(&ModuleId::from("local-stack-mlflow"))
            .and_then(|e| e.get("BASE_PATH")),
        Some("/mlflow")
    );

    // The head file includes every module, in dependency order (deps before
    // dependents): postgres/seaweedfs/envoy precede mlflow and unitycatalog.
    let order: Vec<&str> = p.head.includes.iter().map(|i| i.module.as_str()).collect();
    let pos = |id: &str| order.iter().position(|x| *x == id).unwrap();
    assert!(pos("local-stack-postgres") < pos("local-stack-mlflow"));
    assert!(pos("local-stack-envoy") < pos("local-stack-unity-catalog"));
}

#[test]
fn real_prefix_collision_fails_loudly() {
    // Two modules whose Api endpoints both claim `/api` with nothing to distinguish
    // them — exactly the silent-shadowing the planner refuses.
    fn api_module(id: &str) -> Module {
        // The API mount prefix is declared in extras (`api_prefix:<endpoint_id>`),
        // the same convention the baseline modules use; both modules claim `/api`.
        let mut provides = Provides::default();
        provides
            .extras
            .insert("api_prefix:rest".into(), "/api".into());
        Module {
            id: ModuleId::from(id),
            display_name: None,
            summary: None,
            category: None,
            provider_of: None,
            requires: vec![],
            conflicts_with: vec![],
            services: vec![ServiceSpec {
                name: id.to_string(),
                role: Role::new("svc"),
                placement: Placement::Container {
                    service: id.to_string(),
                },
                endpoints: vec![Endpoint {
                    id: "rest".into(),
                    scheme: Scheme::Http,
                    internal_port: 8080,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                }],
                depends_on: vec![],
            }],
            provides,
            knobs: vec![],
            render: RenderSpec::default(),
        }
    }

    let catalog = Catalog::from_modules([api_module("svc-a"), api_module("svc-b")]);
    let err = plan(
        &Selection::modules(["svc-a", "svc-b"]),
        &catalog,
        &PlanCtx::default(),
    )
    .expect_err("two endpoints claiming /api must collide");

    match err {
        PlanError::PrefixCollision { prefix, .. } => assert_eq!(prefix, "/api"),
        other => panic!("expected PrefixCollision, got {other:?}"),
    }
}

#[test]
fn planner_routes_round_trip_through_the_address_resolver() {
    // The planner's RoutePlan must be consumable by the existing `address` resolver
    // without doubling the path: the resolver composes `join(prefix, endpoint.path)`,
    // so an API endpoint's `path` must stay empty while the mount lives in the route
    // prefix. This is the round-trip the GatewayConfig-only golden test does not cover.
    let p = plan(
        &default_selection(),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .unwrap();
    let ctx = TopologyCtx {
        gateway_service: "envoy".into(),
        gateway_internal_port: 10000,
        gateway_host_port: 9080,
    };
    let mlflow = p
        .graph
        .module(&ModuleId::from("local-stack-mlflow"))
        .unwrap()
        .service("mlflow")
        .unwrap();

    // From a container, the gateway is `envoy:10000`; the tracking API resolves to the
    // single mount prefix — not `/api/2.0/mlflow/api/2.0/mlflow`.
    let tracking = address(Vantage::Container, mlflow, "tracking", &p.routes, &ctx).unwrap();
    assert_eq!(tracking.as_str(), "http://envoy:10000/api/2.0/mlflow");

    // From the host, the UI resolves at the gateway's host port under its base path.
    let ui = address(Vantage::Host, mlflow, "ui", &p.routes, &ctx).unwrap();
    assert_eq!(ui.as_str(), "http://localhost:9080/mlflow");

    // Unity Catalog REST, container vantage — single, un-doubled path.
    let uc = p
        .graph
        .module(&ModuleId::from("local-stack-unity-catalog"))
        .unwrap()
        .service("unitycatalog")
        .unwrap();
    let rest = address(Vantage::Container, uc, "rest", &p.routes, &ctx).unwrap();
    assert_eq!(rest.as_str(), "http://envoy:10000/api/2.1/unity-catalog");
}

#[test]
fn plan_is_byte_identical_regardless_of_selection_order() {
    let cat = baseline_catalog();
    let forward = plan(&default_selection(), &cat, &PlanCtx::default()).unwrap();
    let reversed = plan(
        &Selection::modules([
            "local-stack-unity-catalog",
            "local-stack-mlflow",
            "local-stack-seaweedfs",
            "local-stack-postgres",
            "local-stack-envoy",
        ]),
        &cat,
        &PlanCtx::default(),
    )
    .unwrap();
    assert_eq!(forward, reversed);
}
