//! End-to-end planner validation against the inlined baseline catalog.
//!
//! This is the acceptance test for the module + planner layer: planning the default
//! lakehouse selection must re-derive — purely by rule — the gateway routes the
//! trestle `local-stack-*` components hand-author today. It also pins the two
//! behaviours the planner exists to guarantee: a real prefix collision fails loudly,
//! and the plan is deterministic regardless of selection order.

use std::sync::Arc;

use olai_stack_topology::{
    Catalog, DataModule, Endpoint, GatewayRoute, Module, ModuleId, Placement, PlanCtx, PlanError,
    Provides, RenderSpec, Rewrite, Role, RouteIntent, Scheme, Selection, ServiceSpec, Vantage,
    baseline_catalog,
};

/// The always-on + common lakehouse modules.
fn default_selection() -> Selection {
    Selection::modules(["envoy", "postgres", "seaweedfs", "mlflow", "unity-catalog"])
}

fn route<'a>(routes: &'a [GatewayRoute], prefix: &str) -> &'a GatewayRoute {
    routes
        .iter()
        .find(|r| r.prefix == prefix)
        .unwrap_or_else(|| panic!("expected a gateway route for `{prefix}`"))
}

#[test]
fn default_lakehouse_rederives_the_working_gateway_routes() {
    let p = baseline_catalog()
        .plan(&default_selection(), &PlanCtx::default())
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
            .get(&ModuleId::from("mlflow"))
            .and_then(|e| e.get("BASE_PATH")),
        Some("/mlflow")
    );

    // The head file includes every module, in dependency order (deps before
    // dependents): postgres/seaweedfs/envoy precede mlflow and unitycatalog.
    let order: Vec<&str> = p.head.includes.iter().map(|i| i.module.as_str()).collect();
    let pos = |id: &str| order.iter().position(|x| *x == id).unwrap();
    assert!(pos("postgres") < pos("mlflow"));
    assert!(pos("envoy") < pos("unity-catalog"));
}

#[test]
fn real_prefix_collision_fails_loudly() {
    // Two modules whose Api endpoints both claim `/api` with nothing to distinguish
    // them — exactly the silent-shadowing the planner refuses.
    fn api_module(id: &str) -> Arc<dyn Module> {
        // Both modules' `rest` endpoints claim the `/api` mount prefix (a typed
        // `Endpoint.mount_prefix`), so the planner must reject the collision.
        Arc::new(DataModule {
            id: ModuleId::from(id),
            display_name: None,
            summary: None,
            category: None,
            provider_of: None,
            requires: vec![],
            conflicts_with: vec![],
            needs: vec![],
            service_specs: vec![ServiceSpec {
                name: id.to_string(),
                role: Role::new("svc"),
                placement: Placement::Container {
                    service: id.to_string(),
                },
                base_path: String::new(),
                endpoints: vec![Endpoint {
                    id: "rest".into(),
                    scheme: Scheme::Http,
                    internal_port: 8080,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                    mount_prefix: Some("/api".into()),
                    rewrite: Rewrite::Inherit,
                }],
                depends_on: vec![],
            }],
            provides: Provides::default(),
            knobs: vec![],
            render: RenderSpec::default(),
        })
    }

    let catalog = Catalog::from_modules([api_module("svc-a"), api_module("svc-b")]);
    let err = catalog
        .plan(&Selection::modules(["svc-a", "svc-b"]), &PlanCtx::default())
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
    let p = baseline_catalog()
        .plan(&default_selection(), &PlanCtx::default())
        .unwrap();
    // The plan carries the gateway facts, so addressing needs no separate context: a
    // `ServiceRef` from the plan resolves a URL from just a vantage + endpoint id.
    let mlflow = p.service(&ModuleId::from("mlflow")).unwrap();

    // From a container, the gateway is `envoy:10000`; the tracking API resolves to the
    // single mount prefix — not `/api/2.0/mlflow/api/2.0/mlflow`.
    let tracking = mlflow.address(Vantage::Container, "tracking").unwrap();
    assert_eq!(tracking.as_str(), "http://envoy:10000/api/2.0/mlflow");

    // From the host, the UI resolves at the gateway's host port under its base path.
    let ui = mlflow.address(Vantage::Host, "ui").unwrap();
    assert_eq!(ui.as_str(), "http://localhost:9080/mlflow");

    // Unity Catalog REST, container vantage — single, un-doubled path. Addressed by role
    // here, to exercise that path too ("the data catalog", whichever module fills it).
    let uc = p.service_by_role(&Role::data_catalog()).unwrap();
    let rest = uc.address(Vantage::Container, "rest").unwrap();
    assert_eq!(rest.as_str(), "http://envoy:10000/api/2.1/unity-catalog");
}

#[test]
fn service_by_role_resolves_uniquely_and_reports_misses() {
    use olai_stack_topology::AddressError;

    let p = baseline_catalog()
        .plan(&default_selection(), &PlanCtx::default())
        .unwrap();

    // A role filled by exactly one service resolves to it.
    let catalog = p.service_by_role(&Role::data_catalog()).unwrap();
    assert_eq!(catalog.spec().name, "unitycatalog");

    // A role no service fills is a clean `NoSuchRole`, not a panic.
    let err = p
        .service_by_role(&Role::new("nonexistent_role"))
        .unwrap_err();
    assert!(matches!(err, AddressError::NoSuchRole(r) if r == "nonexistent_role"));
}

#[test]
fn plan_is_byte_identical_regardless_of_selection_order() {
    let cat = baseline_catalog();
    let forward = cat.plan(&default_selection(), &PlanCtx::default()).unwrap();
    let reversed = cat
        .plan(
            &Selection::modules(["unity-catalog", "mlflow", "seaweedfs", "postgres", "envoy"]),
            &PlanCtx::default(),
        )
        .unwrap();
    // `Plan` is no longer `Eq` (its graph holds trait objects), so compare the
    // observable artifacts the planner is contracted to produce deterministically: the
    // gateway config, the routing plan, the head file, and the include order.
    assert_eq!(forward.gateway, reversed.gateway);
    assert_eq!(forward.head, reversed.head);
    assert_eq!(forward.routes, reversed.routes);
    let order = |p: &olai_stack_topology::Plan| {
        p.head
            .includes
            .iter()
            .map(|i| i.module.0.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&forward), order(&reversed));
}
