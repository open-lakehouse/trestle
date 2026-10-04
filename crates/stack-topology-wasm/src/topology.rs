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
//! - **Gateway surface** — every endpoint the gateway exposes (API prefixes, UIs, whole-service
//!   listeners such as the S3 endpoint), from the [`RoutePlan`](olai_stack_topology::RoutePlan),
//!   each attributed to the backend component it stitches in, and whether forward-auth gates it.
//! - **Edges** — clients reaching the gateway, the gateway routing to each backend, the gateway
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
    /// For the gateway: everything it exposes, grouped by backend in dependency order.
    pub exposes: Vec<ExposedDto>,
    /// For a provider: the APIs it offers its consumers (e.g. `"PostgreSQL"`, `"S3 + STS"`).
    pub offers: Vec<String>,
    /// For a provider: the resources it provisions for its consumers (databases, buckets), in
    /// first-demand order.
    pub provisions: Vec<ProvisionedDto>,
}

/// What kind of surface an [`ExposedDto`] is.
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
    /// The host port it is reachable on.
    pub host_port: u16,
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
    /// Clients reach the gateway on these host ports.
    Access {
        /// Always [`CLIENTS_NODE_ID`].
        from: String,
        /// The gateway component.
        to: String,
        /// The host ports the gateway's listeners publish.
        host_ports: Vec<u16>,
    },
    /// The gateway forwards part of its surface to a backend. The routed endpoints are the
    /// gateway node's [`exposes`](GraphNodeDto::exposes) entries for `to`.
    Route {
        /// The gateway component.
        from: String,
        /// The backend component.
        to: String,
        /// Whether any of the routed endpoints is behind forward-auth.
        gated: bool,
    },
    /// The gateway delegates authentication of gated requests to the identity provider.
    Authenticates {
        /// The gateway component.
        from: String,
        /// The identity provider.
        to: String,
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
        Connection::RelationalDb { url } => match url.split_once("://") {
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

    // The gateway's exposed surface, per backend, in dependency order.
    let mut exposes: Vec<ExposedDto> = Vec::new();
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
                exposes.push(ExposedDto {
                    module: id.to_string(),
                    endpoint: endpoint.id.clone(),
                    kind,
                    prefix: route.prefix.clone(),
                    host_port,
                    gated: shared && auth.is_some(),
                });
            }
        }
        // The identity provider's login portal is routed by the planner, not declared as an
        // endpoint; it is open so a logged-out user can reach it.
        if let (Some(a), Some(auth_id)) = (auth, &auth_module)
            && auth_id == id
        {
            exposes.push(ExposedDto {
                module: id.to_string(),
                endpoint: "portal".to_string(),
                kind: ExposedKind::Ui,
                prefix: a.portal_prefix.clone(),
                host_port: shared_port.unwrap_or_default(),
                gated: false,
            });
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
            offers: m
                .provides()
                .resource_kinds
                .values()
                .map(|t| protocol(&t.0, true))
                .collect(),
            exposes: if kind == NodeKind::Gateway {
                exposes.clone()
            } else {
                Vec::new()
            },
            provisions: provisions.remove(id).unwrap_or_default(),
        });
    }

    let mut edges: Vec<TopologyEdgeDto> = Vec::new();
    if let Some(gw) = &gateway_module {
        let host_ports: Vec<u16> = plan.gateway.listeners.iter().map(|l| l.host_port).collect();
        if !host_ports.is_empty() {
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
                    exposes: Vec::new(),
                    provisions: Vec::new(),
                },
            );
            edges.push(TopologyEdgeDto::Access {
                from: CLIENTS_NODE_ID.to_string(),
                to: gw.clone(),
                host_ports,
            });
        }
        // One route edge per backend, in surface order. The IdP's portal is folded into the
        // `Authenticates` edge rather than drawn as a second gateway → IdP arrow.
        let mut routed: Vec<(&str, bool)> = Vec::new();
        for e in &exposes {
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
                gated,
            });
        }
        if let (Some(a), Some(idp)) = (auth, &auth_module) {
            edges.push(TopologyEdgeDto::Authenticates {
                from: gw.clone(),
                to: idp.clone(),
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

    #[test]
    fn gateway_lists_the_surface_it_stitches_together() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let gw = node(&g, "envoy");
        let find = |module: &str, kind: ExposedKind| {
            gw.exposes
                .iter()
                .filter(|e| e.module == module && e.kind == kind)
                .collect::<Vec<_>>()
        };
        assert!(
            find("unity-catalog", ExposedKind::Api)
                .iter()
                .any(|e| e.prefix == "/api/2.1/unity-catalog")
        );
        assert!(!find("mlflow", ExposedKind::Api).is_empty());
        assert!(!find("mlflow", ExposedKind::Ui).is_empty());
        // The object store is exposed whole, on its own listener.
        let s3 = find("rustfs", ExposedKind::Service);
        assert_eq!(s3.len(), 1, "{:?}", gw.exposes);
        let shared = find("mlflow", ExposedKind::Api)[0].host_port;
        assert_ne!(s3[0].host_port, shared);
        // Every backend in the surface gets a route edge; postgres is internal-only.
        for backend in ["unity-catalog", "mlflow", "jaeger", "rustfs"] {
            assert!(g.edges.iter().any(|e| matches!(
                e, TopologyEdgeDto::Route { to, .. } if to == backend
            )));
        }
        assert!(!gw.exposes.iter().any(|e| e.module == "postgres"));
    }

    #[test]
    fn identity_provider_is_a_component_the_gateway_delegates_to() {
        let g = graph_dto(&plan(&lakehouse(true)));
        assert_eq!(node(&g, "authelia").role.as_deref(), Some("auth"));
        assert!(g.edges.iter().any(|e| matches!(
            e,
            TopologyEdgeDto::Authenticates { from, to, .. } if from == "envoy" && to == "authelia"
        )));
        let gw = node(&g, "envoy");
        // The login portal is exposed and open; API/UI routes are gated; the S3 listener isn't.
        let portal = gw.exposes.iter().find(|e| e.module == "authelia").unwrap();
        assert!(!portal.gated);
        for e in &gw.exposes {
            match (e.module.as_str(), e.kind) {
                ("authelia", _) => {}
                (_, ExposedKind::Service) => assert!(!e.gated, "{e:?}"),
                _ => assert!(e.gated, "{e:?}"),
            }
        }
        // No separate route edge to the IdP: the portal folds into `Authenticates`.
        assert!(!g.edges.iter().any(|e| matches!(
            e, TopologyEdgeDto::Route { to, .. } if to == "authelia"
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
