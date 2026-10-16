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
//!   Every endpoint comes from the [`RoutePlan`](crate::RoutePlan), attributed to
//!   the backend it reaches, and flagged when forward-auth gates it.
//! - **Edges** — clients reaching each surface, the gateway routing to each backend, the gateway
//!   delegating authentication to the identity provider and authorization to the policy engine,
//!   and each component *using* the capability it demanded (a relational DB, an object store)
//!   from the provider the planner chose.

use std::collections::BTreeMap;

use crate::{
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
    /// A component the plan relies on but does not run on the deploy target: host-bound,
    /// in-process, or a hosted provider in a cloud (see [`GraphNodeDto::cloud`]).
    External,
}

/// One functional component (a module), or the clients node.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNodeDto {
    /// Module id, or [`CLIENTS_NODE_ID`].
    pub id: String,
    /// The implementation the component is, as an architecture-model element id (see
    /// [`Module::model`]). `None` for the clients node and for a module without one.
    pub model: Option<String>,
    /// What the node stands for.
    pub kind: NodeKind,
    /// The implementation's human-readable name (e.g. `"RustFS"`).
    pub display_name: Option<String>,
    /// One-line summary, for a tooltip.
    pub summary: Option<String>,
    /// Wizard category, if any.
    pub category: Option<String>,
    /// The function the component fills (its primary service's role, e.g. `"object_store"`,
    /// `"auth"`). Drives the node's icon, accent and headline.
    pub role: Option<String>,
    /// Where the component runs (`in_process` / `host` / `container:<service>` /
    /// `external:<origin>`).
    pub placement: Option<String>,
    /// For a hosted provider (see
    /// [`Plan::hosted`](crate::Plan::hosted)): its cloud environment's label
    /// (e.g. `"aws"`), or `""` when it is unnamed. The UI groups these nodes by it. `None`
    /// for everything the stack runs.
    pub cloud: Option<String>,
    /// For the gateway: what it exposes, one surface per listener — the platform surface
    /// first, then each service surface.
    pub surfaces: Vec<SurfaceDto>,
    /// For a provider: the APIs it offers its consumers (e.g. `"PostgreSQL"`, `"S3 + STS"`).
    pub offers: Vec<String>,
    /// For a provider: every resource it provisions (databases, buckets), in the order its
    /// init job creates them — consumers' demands first, then the selection's extra resources.
    pub provisions: Vec<ProvisionedDto>,
    /// For a provider: what it calls each kind of resource it provisions.
    pub resource_nouns: Vec<ResourceNounDto>,
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

/// What a provider calls the resources of one kind it provisions — a database, a bucket, an
/// Azure container — so the UI can label them in the provider's own terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceNounDto {
    /// The resource kind (e.g. `"object_store"`).
    pub kind: String,
    /// One resource (e.g. `"container"`).
    pub singular: String,
    /// Several resources (e.g. `"containers"`).
    pub plural: String,
}

/// The [`ResourceNounDto`] for every resource kind `module` provisions, named after the
/// connection it hands out (an Azure Blob store provisions containers, an S3 store buckets).
pub fn resource_nouns(module: &dyn Module) -> Vec<ResourceNounDto> {
    module
        .provides()
        .resource_kinds
        .iter()
        .map(|(kind, template)| {
            let (singular, plural) = match &template.0 {
                Connection::ObjectStore {
                    credential: ObjectStoreCredential::AzureBlob { .. },
                    ..
                } => ("container", "containers"),
                Connection::ObjectStore { .. } => ("bucket", "buckets"),
                Connection::RelationalDb { .. } => ("database", "databases"),
            };
            ResourceNounDto {
                kind: kind.clone(),
                singular: singular.to_string(),
                plural: plural.to_string(),
            }
        })
        .collect()
}

/// A resource a provider provisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisionedDto {
    /// The resource kind (the demanded role, e.g. `"relational_db"`, `"object_store"`).
    pub resource: String,
    /// The resource's name (a database, a bucket).
    pub name: String,
    /// The components that use it, in dependency order. Empty for an environment-level extra
    /// resource, which is provisioned for something outside the stack.
    pub consumers: Vec<String>,
}

/// A typed, request-direction edge (`from` calls `to`).
///
/// Internally tagged, with the endpoints on every variant, so it serializes to a flat JS object
/// (`{ kind, key, from, to, … }`) through `serde-wasm-bindgen` without needing map support.
///
/// Every variant carries a `key` that is unique within its graph and stable across re-plans of
/// the same input (see [`TopologyEdgeDto::derive_key`]), so a consumer can address an edge;
/// [`graph_dto`] assigns it once every edge is built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopologyEdgeDto {
    /// Clients reach one of the gateway's surfaces.
    Access {
        /// The edge's stable key (`kind:from→to:qualifier`).
        key: String,
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
        /// The edge's stable key (`kind:from→to:qualifier`).
        key: String,
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
        /// The edge's stable key (`kind:from→to:qualifier`).
        key: String,
        /// The gateway component.
        from: String,
        /// The identity provider.
        to: String,
        /// The surface carrying the provider's login portal (its listener's host port).
        host_port: u16,
        /// Where the provider's login portal is exposed (never gated).
        portal_prefix: String,
    },
    /// The gateway asks the policy engine to authorize each gated request, after the identity
    /// provider has authenticated it.
    Authorizes {
        /// The edge's stable key (`kind:from→to:qualifier`).
        key: String,
        /// The gateway component.
        from: String,
        /// The policy engine.
        to: String,
        /// The surface whose requests are checked (its listener's host port).
        host_port: u16,
    },
    /// A component uses a capability another component provides.
    Uses {
        /// The edge's stable key (`kind:from→to:qualifier`).
        key: String,
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
            | Self::Authorizes { from, to, .. }
            | Self::Uses { from, to, .. } => (from, to),
        }
    }

    /// The key this edge's other fields determine: `kind:from→to:qualifier`, where the
    /// qualifier is the capability for a `uses` edge (one per consumer, provider, and
    /// capability) and the surface's host port for the gateway's edges (one per surface and
    /// backend).
    pub fn derive_key(&self) -> String {
        let (kind, qualifier) = match self {
            Self::Access { host_port, .. } => ("access", host_port.to_string()),
            Self::Route { host_port, .. } => ("route", host_port.to_string()),
            Self::Authenticates { host_port, .. } => ("authenticates", host_port.to_string()),
            Self::Authorizes { host_port, .. } => ("authorizes", host_port.to_string()),
            Self::Uses { role, .. } => ("uses", role.clone()),
        };
        let (from, to) = self.endpoints();
        format!("{kind}:{from}→{to}:{qualifier}")
    }

    /// The edge's key (see [`derive_key`](Self::derive_key)).
    pub fn key(&self) -> &str {
        match self {
            Self::Access { key, .. }
            | Self::Route { key, .. }
            | Self::Authenticates { key, .. }
            | Self::Authorizes { key, .. }
            | Self::Uses { key, .. } => key,
        }
    }

    fn assign_key(&mut self) {
        let derived = self.derive_key();
        match self {
            Self::Access { key, .. }
            | Self::Route { key, .. }
            | Self::Authenticates { key, .. }
            | Self::Authorizes { key, .. }
            | Self::Uses { key, .. } => *key = derived,
        }
    }
}

/// Render a [`Placement`] to the display string the UI shows on a node.
fn placement_str(placement: &Placement) -> String {
    match placement {
        Placement::InProcess => "in_process".to_string(),
        Placement::Host => "host".to_string(),
        Placement::Container { service } => format!("container:{service}"),
        Placement::External { origin } => format!("external:{origin}"),
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
        },
        Connection::RelationalDb { url, .. } => match url.split_once("://") {
            Some(("postgres" | "postgresql", _)) => "PostgreSQL".to_string(),
            Some((scheme, _)) => scheme.to_string(),
            None => "SQL".to_string(),
        },
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

    // The module behind a gateway upstream cluster (the cluster's host is a service name).
    let module_of_cluster = |cluster: &str| -> Option<String> {
        let host = plan
            .gateway
            .clusters
            .iter()
            .find(|c| c.name == cluster)
            .map_or(cluster, |c| c.host.as_str());
        module_of_service(host)
    };

    let auth = plan.gateway.auth.as_ref();
    let auth_module = auth.and_then(|a| module_of_cluster(&a.cluster));
    let policy_module = plan
        .gateway
        .policy
        .as_ref()
        .and_then(|p| module_of_cluster(&p.cluster));
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
    // (provider, resource name) → (kind, consumers), from the demands.
    let mut consumers_of: BTreeMap<(String, String), (String, Vec<String>)> = BTreeMap::new();
    for module in &plan.graph.nodes {
        for (idx, demand) in plan.graph.needs(module.id()).iter().enumerate() {
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
            consumers_of
                .entry((to.to_string(), demand.name.clone()))
                .or_insert_with(|| (demand.resource.clone(), Vec::new()))
                .1
                .push(from.to_string());
            let existing = uses.iter_mut().find(|e| {
                matches!(e, TopologyEdgeDto::Uses { from: f, to: t, role, .. }
                    if f == from && t == to && *role == demand.resource)
            });
            match existing {
                Some(TopologyEdgeDto::Uses { resources, .. }) => {
                    resources.push(demand.name.clone());
                }
                _ => uses.push(TopologyEdgeDto::Uses {
                    key: String::new(),
                    from: from.to_string(),
                    to: to.to_string(),
                    role: demand.resource.clone(),
                    protocol: protocol(connection, false),
                    resources: vec![demand.name.clone()],
                }),
            }
        }
    }

    // What each provider provisions, in its init job's order. A name no demand accounts for is
    // an extra resource: it has no consumers, and its kind is the provider's resource kind.
    let mut provisions: BTreeMap<String, Vec<ProvisionedDto>> = BTreeMap::new();
    for (provider, names) in &plan.provisioned {
        let Some(module) = plan.graph.module(provider) else {
            continue;
        };
        let own_kind = module.provides().resource_kinds.keys().next().cloned();
        let list = provisions.entry(provider.as_str().to_string()).or_default();
        for name in names {
            let (resource, consumers) =
                match consumers_of.get(&(provider.as_str().to_string(), name.clone())) {
                    Some((kind, consumers)) => (kind.clone(), consumers.clone()),
                    None => (own_kind.clone().unwrap_or_default(), Vec::new()),
                };
            list.push(ProvisionedDto {
                resource,
                name: name.clone(),
                consumers,
            });
        }
    }

    // What each component uses by hard requirement rather than by demand (e.g. an agent
    // platform that gets its models from the experiment tracker's AI gateway): one edge per
    // required module, labelled with the capability that module provides. The gateway and its
    // auth/policy providers are skipped — their own edges already connect them.
    let infrastructure = [&gateway_module, &auth_module, &policy_module];
    for module in &plan.graph.nodes {
        let from = module.id().as_str();
        for required in module.requires() {
            let to = required.as_str();
            if infrastructure.iter().any(|m| m.as_deref() == Some(to))
                || uses.iter().any(|e| e.endpoints() == (from, to))
            {
                continue;
            }
            let Some(provider) = plan.graph.module(required) else {
                continue;
            };
            let role = provider
                .provider_of()
                .map(str::to_string)
                .or_else(|| primary(to).map(|s| s.role.as_str().to_string()))
                .unwrap_or_default();
            uses.push(TopologyEdgeDto::Uses {
                key: String::new(),
                from: from.to_string(),
                to: to.to_string(),
                role,
                protocol: "HTTP".to_string(),
                resources: Vec::new(),
            });
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
            model: m.model().map(str::to_string),
            kind,
            display_name: m.display_name().map(str::to_string),
            summary: m.summary().map(str::to_string),
            category: m.category().map(str::to_string),
            role: Some(svc.role.as_str().to_string()),
            placement: Some(placement_str(&svc.placement)),
            cloud: plan
                .hosted
                .get(m.id())
                .map(|e| e.cloud.clone().unwrap_or_default()),
            offers: offers_of(id),
            surfaces: if kind == NodeKind::Gateway {
                surfaces.clone()
            } else {
                Vec::new()
            },
            provisions: provisions.remove(id).unwrap_or_default(),
            resource_nouns: resource_nouns(m.as_ref()),
        });
    }

    let mut edges: Vec<TopologyEdgeDto> = Vec::new();
    if let Some(gw) = &gateway_module {
        if !surfaces.is_empty() {
            nodes.insert(
                0,
                GraphNodeDto {
                    id: CLIENTS_NODE_ID.to_string(),
                    model: None,
                    kind: NodeKind::Clients,
                    display_name: Some("Clients".to_string()),
                    summary: Some("Browsers, SDKs and CLIs on the host.".to_string()),
                    category: None,
                    role: None,
                    placement: None,
                    cloud: None,
                    offers: Vec::new(),
                    surfaces: Vec::new(),
                    provisions: Vec::new(),
                    resource_nouns: Vec::new(),
                },
            );
            for surface in &surfaces {
                edges.push(TopologyEdgeDto::Access {
                    key: String::new(),
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
                    key: String::new(),
                    from: gw.clone(),
                    to: to.to_string(),
                    host_port: surface.host_port,
                    gated,
                });
            }
        }
        if let (Some(a), Some(idp)) = (auth, &auth_module) {
            edges.push(TopologyEdgeDto::Authenticates {
                key: String::new(),
                from: gw.clone(),
                to: idp.clone(),
                host_port: shared_port.unwrap_or_default(),
                portal_prefix: a.portal_prefix.clone(),
            });
        }
        // The policy check runs on the shared listener, after forward-auth.
        if let Some(engine) = &policy_module {
            edges.push(TopologyEdgeDto::Authorizes {
                key: String::new(),
                from: gw.clone(),
                to: engine.clone(),
                host_port: shared_port.unwrap_or_default(),
            });
        }
    }
    edges.extend(uses);
    for edge in &mut edges {
        edge.assign_key();
    }

    GraphDto { nodes, edges }
}

#[cfg(test)]
mod tests {
    use crate::{Catalog, ExtraResource, PlanCtx, Selection, baseline_catalog};

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
    fn provisions_name_their_consumers_and_include_extras() {
        let mut selection = lakehouse(false);
        selection.extra_resources = vec![
            ExtraResource {
                resource: "relational_db".into(),
                name: "analytics".into(),
                provider: Some("postgres".into()),
            },
            ExtraResource {
                resource: "object_store".into(),
                name: "exports".into(),
                provider: None,
            },
        ];
        let g = graph_dto(&plan(&selection));
        let postgres = &node(&g, "postgres").provisions;
        let uc_db = postgres
            .iter()
            .find(|p| p.name == "unitycatalog")
            .expect("UC's database");
        assert_eq!(uc_db.resource, "relational_db");
        assert_eq!(uc_db.consumers, ["unity-catalog"]);
        // An extra resource comes last, with the provider's kind and no consumers.
        let extra = postgres.last().expect("provisions");
        assert_eq!(
            (extra.name.as_str(), extra.resource.as_str()),
            ("analytics", "relational_db")
        );
        assert!(extra.consumers.is_empty());
        let bucket = node(&g, "rustfs")
            .provisions
            .iter()
            .find(|p| p.name == "exports")
            .expect("the extra bucket lands on the object store");
        assert_eq!(bucket.resource, "object_store");
        assert!(bucket.consumers.is_empty());
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
    fn a_selected_policy_engine_authorizes_gateway_requests() {
        // The policy engine exposes nothing (its only endpoint is the internal gRPC check), so
        // the gateway's authorization check is its only connection.
        let g = graph_dto(&plan(&Selection::modules([
            "envoy", "authelia", "opa", "mlflow",
        ])));
        assert_eq!(node(&g, "opa").role.as_deref(), Some("policy"));
        let gateway_port = PlanCtx::default().gateway_host_port;
        assert!(g.edges.iter().any(|e| matches!(
            e,
            TopologyEdgeDto::Authorizes { from, to, host_port, .. }
                if from == "envoy" && to == "opa" && *host_port == gateway_port
        )));
        // Authorization comes on top of sign-in, not instead of it.
        assert!(g.edges.iter().any(|e| matches!(
            e,
            TopologyEdgeDto::Authenticates { to, .. } if to == "authelia"
        )));
        // Without a policy engine there is no authorization check.
        let g = graph_dto(&plan(&lakehouse(true)));
        assert!(
            !g.edges
                .iter()
                .any(|e| matches!(e, TopologyEdgeDto::Authorizes { .. }))
        );
    }

    #[test]
    fn a_hard_requirement_draws_a_uses_edge() {
        // Omnigent requires MLflow (its agents' models come from MLflow's AI Gateway), not by a
        // resource demand: the graph still shows the dependency, labelled with what MLflow
        // provides. Its requirement on the gateway is not drawn twice.
        let g = graph_dto(&plan(&Selection::modules(["envoy", "omnigent"])));
        assert_eq!(node(&g, "omnigent").role.as_deref(), Some("agent_platform"));
        let uses = |from: &str, to: &str| {
            g.edges
                .iter()
                .filter(|e| {
                    matches!(e, TopologyEdgeDto::Uses { from: f, to: t, .. } if f == from && t == to)
                })
                .collect::<Vec<_>>()
        };
        assert!(matches!(
            uses("omnigent", "mlflow").as_slice(),
            [TopologyEdgeDto::Uses { role, protocol, resources, .. }]
                if role == "experiment_tracking" && protocol == "HTTP" && resources.is_empty()
        ));
        assert!(uses("omnigent", "envoy").is_empty());
        // The demanded relational store keeps its demand edge.
        assert!(matches!(
            uses("omnigent", "postgres").as_slice(),
            [TopologyEdgeDto::Uses { role, .. }] if role == "relational_db"
        ));
    }

    #[test]
    fn every_edge_references_a_node() {
        let mut authorized = lakehouse(false);
        authorized.modules.extend(["authelia".into(), "opa".into()]);
        for selection in [lakehouse(false), lakehouse(true), authorized] {
            let g = graph_dto(&plan(&selection));
            for e in &g.edges {
                let (from, to) = e.endpoints();
                for id in [from, to] {
                    assert!(g.nodes.iter().any(|n| n.id == id), "dangling {e:?}");
                }
            }
        }
    }

    /// Selections that between them reach every edge kind: auth + policy, every in-stack
    /// module that coexists (one object store), and the lakehouse with and without auth.
    fn edge_rich_selections() -> Vec<Selection> {
        let mut authorized = lakehouse(false);
        authorized.modules.extend(["authelia".into(), "opa".into()]);
        let everything = Selection::modules([
            "envoy",
            "authelia",
            "opa",
            "postgres",
            "rustfs",
            "mlflow",
            "unity-catalog",
            "jaeger",
            "headwaters",
            "omnigent",
            "marimo",
            "puppygraph",
            "rivers",
        ]);
        vec![lakehouse(false), lakehouse(true), authorized, everything]
    }

    #[test]
    fn edge_keys_are_unique_and_derived_from_the_edge() {
        let mut kinds = std::collections::BTreeSet::new();
        for selection in edge_rich_selections() {
            let g = graph_dto(&plan(&selection));
            let mut seen = std::collections::BTreeSet::new();
            for e in &g.edges {
                assert_eq!(e.key(), e.derive_key(), "unassigned key on {e:?}");
                assert!(
                    seen.insert(e.key().to_string()),
                    "duplicate key {}",
                    e.key()
                );
                kinds.insert(e.key().split(':').next().unwrap().to_string());
            }
        }
        // Every variant was exercised, so the uniqueness check above covers each one.
        assert_eq!(
            kinds.into_iter().collect::<Vec<_>>(),
            ["access", "authenticates", "authorizes", "route", "uses"]
        );
    }

    #[test]
    fn edge_keys_read_kind_endpoints_and_qualifier() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let keys: Vec<&str> = g.edges.iter().map(TopologyEdgeDto::key).collect();
        assert!(
            keys.contains(&"uses:mlflow→postgres:relational_db"),
            "{keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.starts_with("access:@clients→envoy:")),
            "{keys:?}"
        );
    }

    #[test]
    fn component_nodes_carry_their_model_ref() {
        let g = graph_dto(&plan(&lakehouse(false)));
        assert_eq!(node(&g, "postgres").model.as_deref(), Some("postgresql"));
        assert_eq!(
            node(&g, "unity-catalog").model.as_deref(),
            Some("unityCatalogOSS")
        );
        assert_eq!(node(&g, CLIENTS_NODE_ID).model, None);
    }
}
