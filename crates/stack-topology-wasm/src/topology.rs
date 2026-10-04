//! The functional topology behind the editor's "markitecture" diagram.
//!
//! The diagram explains *what the environment does*, not how its containers are wired: one
//! node per functional component (a module: the object store, the data catalog, the identity
//! provider, …), with the implementation (RustFS, Authelia, …) as a detail. Helper containers
//! a module needs to fulfil its function — an STS shim, a bucket-init job, a cert mint — are
//! part of that component and never appear on their own.
//!
//! Everything is read from the settled [`Plan`], so the picture follows the planner's actual
//! decisions:
//!
//! - **Gateway surfaces** — one per listener: the shared *platform* surface that stitches the
//!   components' APIs and UIs together under path prefixes, and a *service* surface per
//!   dedicated listener for a backend that must own its origin because clients speak its wire
//!   protocol (an S3 endpoint). Locally these are ports; hosted, each is its own subdomain.
//!   Every endpoint comes from the [`RoutePlan`](olai_stack_topology::RoutePlan), attributed to
//!   the backend it reaches, and flagged when forward-auth gates it.
//! - **Edges** — clients reaching each surface, the gateway routing to each backend, the gateway
//!   delegating authentication to the identity provider, and each component *using* the
//!   capability it demanded (a relational DB, an object store) from the provider the planner
//!   chose.

use std::collections::BTreeMap;

use olai_stack_topology::{
    Connection, Listener, Module, ObjectStoreCredential, Placement, Plan, Role, RouteIntent,
};
use serde::Serialize;

/// The id of the synthetic node standing for the environment's clients (browsers, SDKs, CLIs on
/// the host). `@` cannot appear in a module id, so it never collides with a component.
pub const CLIENTS_NODE_ID: &str = "@clients";

/// The functional diagram: components and the typed relationships between them.
#[derive(Debug, Clone, Serialize)]
pub struct GraphDto {
    /// The [`CLIENTS_NODE_ID`] node (when the gateway exposes anything), then every component
    /// that runs a service, in dependency order.
    pub nodes: Vec<GraphNodeDto>,
    /// Typed, request-direction edges between [`nodes`](Self::nodes).
    pub edges: Vec<TopologyEdgeDto>,
}

/// What a [`GraphNodeDto`] stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// The environment's clients ([`CLIENTS_NODE_ID`]).
    Clients,
    /// The gateway: the single front door, carrying the exposed surface.
    Gateway,
    /// A component the stack runs.
    Component,
    /// A component the plan relies on but does not run in compose (host-bound or in-process),
    /// e.g. an external identity provider.
    External,
}

/// One functional component (a module), or the clients node.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNodeDto {
    /// Module id, or [`CLIENTS_NODE_ID`].
    pub id: String,
    /// What the node stands for.
    pub kind: NodeKind,
    /// The implementation's human-readable name (e.g. `"RustFS (local S3 + STS)"`).
    pub display_name: Option<String>,
    /// One-line summary, for a tooltip.
    pub summary: Option<String>,
    /// Wizard category, if any.
    pub category: Option<String>,
    /// The function the component fills (its primary service's role, e.g. `"object_store"`,
    /// `"auth"`). Drives the node's icon, accent and headline.
    pub role: Option<String>,
    /// Where the component runs (`in_process` / `host` / `container:<service>`).
    pub placement: Option<String>,
    /// For the gateway: what it exposes, one surface per listener — the platform surface
    /// first, then each service surface.
    pub surfaces: Vec<SurfaceDto>,
    /// For a provider: the APIs it offers its consumers (e.g. `"PostgreSQL"`, `"S3 + STS"`).
    pub offers: Vec<String>,
    /// For a provider: the resources it provisions for its consumers (databases, buckets), in
    /// first-demand order.
    pub provisions: Vec<ProvisionedDto>,
}

/// What a [`SurfaceDto`] is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// The shared listener: the platform's own API + UI surface, path-multiplexed across
    /// components.
    Platform,
    /// A dedicated listener for one backend whose clients address it at the origin in its own
    /// wire protocol (e.g. S3 SDKs), so it cannot sit under a path prefix.
    Service,
}

/// One listener of the gateway — locally a host port, hosted its own subdomain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurfaceDto {
    /// The host port the listener publishes.
    pub host_port: u16,
    /// What the surface is for.
    pub kind: SurfaceKind,
    /// For a service surface: the protocols its backends offer (e.g. `"S3 + STS"`).
    pub protocols: Vec<String>,
    /// The endpoints it exposes, grouped by backend in dependency order.
    pub exposes: Vec<ExposedDto>,
}

/// What kind of endpoint an [`ExposedDto`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposedKind {
    /// A REST API mounted under a path prefix on the shared listener.
    Api,
    /// A browser UI.
    Ui,
    /// A whole service on its own listener (e.g. an S3 endpoint SDKs address at the origin).
    Service,
}

/// One endpoint the gateway exposes to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExposedDto {
    /// The backend component serving it.
    pub module: String,
    /// The backend endpoint's id (e.g. `"tracking"`, `"ui"`, `"s3"`).
    pub endpoint: String,
    /// What kind of surface it is.
    pub kind: ExposedKind,
    /// The client-facing path prefix (`/` for a whole-service listener).
    pub prefix: String,
    /// Whether forward-auth gates it.
    pub gated: bool,
}

/// A resource a provider provisions for a consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisionedDto {
    /// The resource kind (the demanded role, e.g. `"relational_db"`, `"object_store"`).
    pub resource: String,
    /// The resource's name (a database, a bucket).
    pub name: String,
}

/// A typed, request-direction edge (`from` calls `to`).
///
/// Internally tagged, with the endpoints on every variant, so it serializes to a flat JS object
/// (`{ kind, from, to, … }`) through `serde-wasm-bindgen` without needing map support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopologyEdgeDto {
    /// Clients reach one of the gateway's surfaces.
    Access {
        /// Always [`CLIENTS_NODE_ID`].
        from: String,
        /// The gateway component.
        to: String,
        /// The surface reached (its listener's host port).
        host_port: u16,
    },
    /// The gateway forwards part of one surface to a backend. The routed endpoints are that
    /// [`SurfaceDto`]'s `exposes` entries for `to`.
    Route {
        /// The gateway component.
        from: String,
        /// The backend component.
        to: String,
        /// The surface the routes belong to (its listener's host port).
        host_port: u16,
        /// Whether any of the routed endpoints is behind forward-auth.
        gated: bool,
    },
    /// The gateway delegates authentication of gated requests to the identity provider.
    Authenticates {
        /// The gateway component.
        from: String,
        /// The identity provider.
        to: String,
        /// The surface carrying the provider's login portal (its listener's host port).
        host_port: u16,
        /// Where the provider's login portal is exposed (never gated).
        portal_prefix: String,
    },
    /// A component uses a capability another component provides.
    Uses {
        /// The consumer.
        from: String,
        /// The provider the planner chose.
        to: String,
        /// The capability used (the demanded role, e.g. `"object_store"`).
        role: String,
        /// How it is spoken, at the functional level (e.g. `"PostgreSQL"`, `"S3"`).
        protocol: String,
        /// The resources used (database / bucket names).
        resources: Vec<String>,
    },
}

impl TopologyEdgeDto {
    /// The edge's endpoints.
    pub fn endpoints(&self) -> (&str, &str) {
        match self {
            Self::Access { from, to, .. }
            | Self::Route { from, to, .. }
            | Self::Authenticates { from, to, .. }
            | Self::Uses { from, to, .. } => (from, to),
        }
    }
}

/// Render a [`Placement`] to the display string the UI shows on a node.
fn placement_str(placement: &Placement) -> String {
    match placement {
        Placement::InProcess => "in_process".to_string(),
        Placement::Host => "host".to_string(),
        Placement::Container { service } => format!("container:{service}"),
    }
}

/// The functional name of how a [`Connection`] is spoken. With `full`, the provider's whole
/// offering (an S3 store that vends scoped credentials offers `S3 + STS`); without, just the
/// data protocol a consumer speaks to it.
fn protocol(connection: &Connection, full: bool) -> String {
    match connection {
        Connection::ObjectStore { credential, .. } => match credential {
            // A role to assume means the store vends scoped credentials through its STS API —
            // part of the object store's contract, however the stack fulfils it.
            ObjectStoreCredential::S3 {
                role_arn: Some(_), ..
            } if full => "S3 + STS".to_string(),
            ObjectStoreCredential::S3 { .. } => "S3".to_string(),
            ObjectStoreCredential::AzureBlob { .. } => "Azure Blob".to_string(),
            _ => "object store".to_string(),
        },
        Connection::RelationalDb { url, .. } => match url.split_once("://") {
            Some(("postgres" | "postgresql", _)) => "PostgreSQL".to_string(),
            Some((scheme, _)) => scheme.to_string(),
            None => "SQL".to_string(),
        },
        _ => "connection".to_string(),
    }
}

/// Project a settled [`Plan`] into the functional [`GraphDto`].
pub fn graph_dto(plan: &Plan) -> GraphDto {
    let primary = |id: &str| {
        plan.services
            .iter()
            .find(|(mid, _)| mid.as_str() == id)
            .and_then(|(_, specs)| specs.first())
    };
    let gateway_module: Option<String> = plan
        .services
        .iter()
        .find(|(_, specs)| specs.iter().any(|s| s.role == Role::gateway()))
        .map(|(id, _)| id.as_str().to_string());
    let module_of_service = |service: &str| -> Option<String> {
        plan.services.iter().find_map(|(id, specs)| {
            specs
                .iter()
                .any(|s| s.name == service)
                .then(|| id.as_str().to_string())
        })
    };

    let auth = plan.gateway.auth.as_ref();
    let auth_module = auth.and_then(|a| {
        let host = plan
            .gateway
            .clusters
            .iter()
            .find(|c| c.name == a.cluster)
            .map_or(a.cluster.as_str(), |c| c.host.as_str());
        module_of_service(host)
    });
    // The shared listener is the first; its routes are the ones forward-auth gates.
    let shared_port = plan.gateway.listeners.first().map(|l| l.host_port);

    // Every exposed endpoint with its listener's host port, per backend in dependency order.
    let mut exposes: Vec<(u16, ExposedDto)> = Vec::new();
    for module in &plan.graph.nodes {
        let id = module.id().as_str();
        for spec in plan.services.get(module.id()).into_iter().flatten() {
            for endpoint in &spec.endpoints {
                let kind = match endpoint.intent {
                    RouteIntent::Internal => continue,
                    RouteIntent::Api => ExposedKind::Api,
                    RouteIntent::UiPrefixable | RouteIntent::UiFixed => ExposedKind::Ui,
                    RouteIntent::Gatewayed => ExposedKind::Service,
                };
                let Some(route) = plan.routes.get(&spec.name, &endpoint.id) else {
                    continue;
                };
                let (host_port, shared) = match route.listener {
                    Listener::Shared => (shared_port.unwrap_or_default(), true),
                    Listener::Dedicated { port } => (port, false),
                };
                exposes.push((
                    host_port,
                    ExposedDto {
                        module: id.to_string(),
                        endpoint: endpoint.id.clone(),
                        kind,
                        prefix: route.prefix.clone(),
                        gated: shared && auth.is_some(),
                    },
                ));
            }
        }
        // The identity provider's login portal is routed by the planner, not declared as an
        // endpoint; it is open so a logged-out user can reach it.
        if let (Some(a), Some(auth_id)) = (auth, &auth_module)
            && auth_id == id
        {
            exposes.push((
                shared_port.unwrap_or_default(),
                ExposedDto {
                    module: id.to_string(),
                    endpoint: "portal".to_string(),
                    kind: ExposedKind::Ui,
                    prefix: a.portal_prefix.clone(),
                    gated: false,
                },
            ));
        }
    }

    // What each component uses: one edge per (consumer, provider, capability), from the
    // demands and the provider the planner chose for each.
    let mut uses: Vec<TopologyEdgeDto> = Vec::new();
    let mut provisions: BTreeMap<String, Vec<ProvisionedDto>> = BTreeMap::new();
    for module in &plan.graph.nodes {
        for (idx, demand) in module.needs().iter().enumerate() {
            let Some(provider) = plan.graph.edges.iter().find_map(|e| {
                (e.from == *module.id())
                    .then(|| plan.graph.module(&e.to))
                    .flatten()
                    .filter(|p| p.provides().resource_kinds.contains_key(&demand.resource))
            }) else {
                continue;
            };
            let Some(connection) = plan.connections.get(&(module.id().clone(), idx)) else {
                continue;
            };
            let (from, to) = (module.id().as_str(), provider.id().as_str());
            provisions
                .entry(to.to_string())
                .or_default()
                .push(ProvisionedDto {
                    resource: demand.resource.clone(),
                    name: demand.name.clone(),
                });
            let existing = uses.iter_mut().find(|e| {
                matches!(e, TopologyEdgeDto::Uses { from: f, to: t, role, .. }
                    if f == from && t == to && *role == demand.resource)
            });
            match existing {
                Some(TopologyEdgeDto::Uses { resources, .. }) => {
                    resources.push(demand.name.clone());
                }
                _ => uses.push(TopologyEdgeDto::Uses {
                    from: from.to_string(),
                    to: to.to_string(),
                    role: demand.resource.clone(),
                    protocol: protocol(connection, false),
                    resources: vec![demand.name.clone()],
                }),
            }
        }
    }

    // The gateway's surfaces, one per listener in listener order (the shared one first). A
    // service surface names the protocols its backends offer.
    let offers_of = |id: &str| -> Vec<String> {
        plan.graph
            .nodes
            .iter()
            .find(|m| m.id().as_str() == id)
            .map(|m| {
                m.provides()
                    .resource_kinds
                    .values()
                    .map(|t| protocol(&t.0, true))
                    .collect()
            })
            .unwrap_or_default()
    };
    let surfaces: Vec<SurfaceDto> = plan
        .gateway
        .listeners
        .iter()
        .filter_map(|l| {
            let on_port: Vec<ExposedDto> = exposes
                .iter()
                .filter(|(port, _)| *port == l.host_port)
                .map(|(_, e)| e.clone())
                .collect();
            if on_port.is_empty() {
                return None;
            }
            let kind = if Some(l.host_port) == shared_port {
                SurfaceKind::Platform
            } else {
                SurfaceKind::Service
            };
            let mut protocols: Vec<String> = Vec::new();
            if kind == SurfaceKind::Service {
                for e in &on_port {
                    for p in offers_of(&e.module) {
                        if !protocols.contains(&p) {
                            protocols.push(p);
                        }
                    }
                }
            }
            Some(SurfaceDto {
                host_port: l.host_port,
                kind,
                protocols,
                exposes: on_port,
            })
        })
        .collect();

    // Components: every module that runs a service. (An env-only contract module contributes
    // configuration, not a function, so it has no node.)
    let mut nodes: Vec<GraphNodeDto> = Vec::new();
    for module in &plan.graph.nodes {
        let m: &std::sync::Arc<dyn Module> = module;
        let id = m.id().as_str();
        let Some(svc) = primary(id) else {
            continue;
        };
        let kind = if gateway_module.as_deref() == Some(id) {
            NodeKind::Gateway
        } else if matches!(svc.placement, Placement::Container { .. }) {
            NodeKind::Component
        } else {
            NodeKind::External
        };
        nodes.push(GraphNodeDto {
            id: id.to_string(),
            kind,
            display_name: m.display_name().map(str::to_string),
            summary: m.summary().map(str::to_string),
            category: m.category().map(str::to_string),
            role: Some(svc.role.as_str().to_string()),
            placement: Some(placement_str(&svc.placement)),
            offers: offers_of(id),
            surfaces: if kind == NodeKind::Gateway {
                surfaces.clone()
            } else {
                Vec::new()
            },
            provisions: provisions.remove(id).unwrap_or_default(),
        });
    }

    let mut edges: Vec<TopologyEdgeDto> = Vec::new();
    if let Some(gw) = &gateway_module {
        if !surfaces.is_empty() {
            nodes.insert(
                0,
                GraphNodeDto {
                    id: CLIENTS_NODE_ID.to_string(),
                    kind: NodeKind::Clients,
                    display_name: Some("Clients".to_string()),
                    summary: Some("Browsers, SDKs and CLIs on the host.".to_string()),
                    category: None,
                    role: None,
                    placement: None,
                    offers: Vec::new(),
                    surfaces: Vec::new(),
                    provisions: Vec::new(),
                },
            );
            for surface in &surfaces {
                edges.push(TopologyEdgeDto::Access {
                    from: CLIENTS_NODE_ID.to_string(),
                    to: gw.clone(),
                    host_port: surface.host_port,
                });
            }
        }
        // One route edge per (surface, backend), in surface order. The IdP's portal is folded
        // into the `Authenticates` edge rather than drawn as a second gateway → IdP arrow.
        for surface in &surfaces {
            let mut routed: Vec<(&str, bool)> = Vec::new();
            for e in &surface.exposes {
                if Some(&e.module) == auth_module.as_ref() {
                    continue;
                }
                match routed.iter_mut().find(|(m, _)| *m == e.module) {
                    Some((_, gated)) => *gated |= e.gated,
                    None => routed.push((&e.module, e.gated)),
                }
            }
            for (to, gated) in routed {
                edges.push(TopologyEdgeDto::Route {
                    from: gw.clone(),
                    to: to.to_string(),
                    host_port: surface.host_port,
                    gated,
                });
            }
        }
        if let (Some(a), Some(idp)) = (auth, &auth_module) {
            edges.push(TopologyEdgeDto::Authenticates {
                from: gw.clone(),
                to: idp.clone(),
                host_port: shared_port.unwrap_or_default(),
                portal_prefix: a.portal_prefix.clone(),
            });
        }
    }
    edges.extend(uses);

    GraphDto { nodes, edges }
}

#[cfg(test)]
mod tests {
    use olai_stack_topology::{Catalog, PlanCtx, Selection, baseline_catalog};

    use super::*;

    fn plan(selection: &Selection) -> Plan {
        let catalog: Catalog = baseline_catalog();
        catalog
            .plan(selection, &PlanCtx::default())
            .expect("plan should succeed")
    }

    fn lakehouse(auth: bool) -> Selection {
        let mut selection = Selection::modules([
            "envoy",
            "postgres",
            "rustfs",
            "unity-catalog",
            "mlflow",
            "jaeger",
            "headwaters",
        ]);
        if auth {
            selection
                .knob_overrides
                .entry("envoy".into())
                .or_default()
                .insert("auth".into(), "true".into());
        }
        selection
    }

    fn node<'a>(g: &'a GraphDto, id: &str) -> &'a GraphNodeDto {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("no node {id}"))
    }

    fn uses<'a>(g: &'a GraphDto, from: &str, to: &str) -> Option<(&'a str, &'a [String])> {
        g.edges.iter().find_map(|e| match e {
            TopologyEdgeDto::Uses {
                from: f,
                to: t,
                protocol,
                resources,
                ..
            } if f == from && t == to => Some((protocol.as_str(), resources.as_slice())),
            _ => None,
        })
    }

    #[test]
    fn nodes_are_functional_components_not_containers() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids[0], CLIENTS_NODE_ID);
        for module in ["envoy", "postgres", "rustfs", "unity-catalog", "mlflow"] {
            assert!(ids.contains(&module), "{module} missing: {ids:?}");
        }
        // Helper containers are part of their component, never nodes of their own.
        for helper in [
            "sts-shim",
            "rustfs-init",
            "gateway-certs",
            "headwaters-migrate",
            "db",
        ] {
            assert!(
                !ids.contains(&helper),
                "{helper} should not be a node: {ids:?}"
            );
        }
        assert_eq!(node(&g, "envoy").kind, NodeKind::Gateway);
        assert_eq!(node(&g, "rustfs").role.as_deref(), Some("object_store"));
    }

    #[test]
    fn components_use_the_capabilities_they_demand() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let (proto, dbs) = uses(&g, "mlflow", "postgres").expect("mlflow uses postgres");
        assert_eq!(proto, "PostgreSQL");
        assert_eq!(dbs, ["mlflow"]);
        let (proto, buckets) = uses(&g, "unity-catalog", "rustfs").expect("UC uses rustfs");
        assert_eq!(proto, "S3");
        assert_eq!(buckets, ["unity"]);
        // STS is part of what the object store offers, not a separate shim component.
        assert_eq!(node(&g, "rustfs").offers, ["S3 + STS"]);
        assert_eq!(node(&g, "postgres").offers, ["PostgreSQL"]);
        assert!(uses(&g, "mlflow", "rustfs").is_some());
        // Providers list what they provision.
        let names: Vec<&str> = node(&g, "postgres")
            .provisions
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(
            names.contains(&"mlflow") && names.contains(&"unitycatalog"),
            "{names:?}"
        );
        // `requires: [envoy]` is not a use: the gateway routes *to* the component instead.
        assert!(uses(&g, "mlflow", "envoy").is_none());
    }

    /// Every exposed endpoint across the gateway's surfaces, with its surface.
    fn exposed(g: &GraphDto) -> Vec<(&SurfaceDto, &ExposedDto)> {
        node(g, "envoy")
            .surfaces
            .iter()
            .flat_map(|s| s.exposes.iter().map(move |e| (s, e)))
            .collect()
    }

    #[test]
    fn gateway_surfaces_split_the_platform_api_from_service_endpoints() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let surfaces = &node(&g, "envoy").surfaces;
        // The platform surface comes first, on the shared listener, then the S3 service.
        assert_eq!(surfaces[0].kind, SurfaceKind::Platform);
        assert_eq!(surfaces[0].host_port, PlanCtx::default().gateway_host_port);
        let s3: Vec<&SurfaceDto> = surfaces
            .iter()
            .filter(|s| s.kind == SurfaceKind::Service)
            .collect();
        assert_eq!(s3.len(), 1, "{surfaces:?}");
        assert_ne!(s3[0].host_port, surfaces[0].host_port);
        assert_eq!(s3[0].protocols, ["S3 + STS"]);
        assert!(
            s3[0]
                .exposes
                .iter()
                .all(|e| e.module == "rustfs" && e.kind == ExposedKind::Service)
        );
        // The platform surface stitches the components' APIs and UIs together.
        let platform = &surfaces[0].exposes;
        assert!(
            platform
                .iter()
                .any(|e| e.module == "unity-catalog" && e.prefix == "/api/2.1/unity-catalog")
        );
        assert!(
            platform
                .iter()
                .any(|e| e.module == "mlflow" && e.kind == ExposedKind::Ui)
        );
        assert!(platform.iter().all(|e| e.module != "rustfs"));
        assert!(surfaces[0].protocols.is_empty());
        // Postgres is internal-only: never exposed.
        assert!(!exposed(&g).iter().any(|(_, e)| e.module == "postgres"));
        // Clients reach each surface; routes are per (surface, backend).
        for s in surfaces {
            assert!(g.edges.iter().any(|e| matches!(
                e, TopologyEdgeDto::Access { host_port, .. } if *host_port == s.host_port
            )));
        }
        for (backend, port) in [
            ("unity-catalog", surfaces[0].host_port),
            ("mlflow", surfaces[0].host_port),
            ("jaeger", surfaces[0].host_port),
            ("rustfs", s3[0].host_port),
        ] {
            assert!(
                g.edges.iter().any(|e| matches!(
                    e, TopologyEdgeDto::Route { to, host_port, .. } if to == backend && *host_port == port
                )),
                "{backend} on :{port}"
            );
        }
    }

    #[test]
    fn identity_provider_is_a_component_the_gateway_delegates_to() {
        let g = graph_dto(&plan(&lakehouse(true)));
        assert_eq!(node(&g, "authelia").role.as_deref(), Some("auth"));
        assert!(g.edges.iter().any(|e| matches!(
            e,
            TopologyEdgeDto::Authenticates { from, to, .. } if from == "envoy" && to == "authelia"
        )));
        // The login portal is on the platform surface and open; the rest of the platform surface
        // is gated; service surfaces never are.
        for (surface, e) in exposed(&g) {
            match (surface.kind, e.module.as_str()) {
                (SurfaceKind::Platform, "authelia") => assert!(!e.gated, "{e:?}"),
                (SurfaceKind::Platform, _) => assert!(e.gated, "{e:?}"),
                (SurfaceKind::Service, _) => assert!(!e.gated, "{e:?}"),
            }
        }
        assert!(
            exposed(&g)
                .iter()
                .any(|(s, e)| s.kind == SurfaceKind::Platform && e.module == "authelia")
        );
        // No separate route edge to the IdP: the portal folds into `Authenticates`.
        assert!(!g.edges.iter().any(|e| matches!(
            e, TopologyEdgeDto::Route { to, .. } if to == "authelia"
        )));
    }

    #[test]
    fn a_selected_identity_provider_is_connected_to_the_gateway() {
        // Enabling the IdP module alone (gateway `auth` knob untouched) must not leave it as an
        // unconnected node.
        let g = graph_dto(&plan(&Selection::modules(["envoy", "authelia", "mlflow"])));
        assert!(g.edges.iter().any(|e| matches!(
            e,
            TopologyEdgeDto::Authenticates { from, to, .. } if from == "envoy" && to == "authelia"
        )));
    }

    #[test]
    fn every_edge_references_a_node() {
        for auth in [false, true] {
            let g = graph_dto(&plan(&lakehouse(auth)));
            for e in &g.edges {
                let (from, to) = e.endpoints();
                for id in [from, to] {
                    assert!(g.nodes.iter().any(|n| n.id == id), "dangling {e:?}");
                }
            }
        }
    }
}
