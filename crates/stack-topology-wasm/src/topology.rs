//! The runtime topology projection behind the editor's "markitecture" diagram.
//!
//! The planner's [`ResolvedGraph`](olai_stack_topology::ResolvedGraph) is a *selection* graph:
//! its edges are `requires` + demand edges, used to pull modules in and order them. It is not
//! the layout that actually runs. This module instead reads what the plan **emits** — the
//! rendered compose fragments (`plan.head.includes`) and the structured
//! [`GatewayConfig`] — so the diagram cannot drift from `compose.yaml`:
//!
//! - **Nodes** are the long-running compose services, owned by the module whose fragment
//!   defines them. One-shot jobs (init/migrate/cert-mint) are dropped, and their startup gates
//!   are collapsed onto what the job itself waits on. Services a module places outside compose
//!   ([`Placement::Host`] / [`Placement::InProcess`]) appear as `external` nodes, plus a
//!   synthetic [`HOST_NODE_ID`] node for the host/browser.
//! - **Edges** point in request direction (caller → callee) and are typed: compose
//!   `depends_on` gates, gateway routes, the forward-auth check, the emulated-AWS TLS path, and
//!   host-published ports.

use std::collections::{BTreeMap, BTreeSet};

use olai_stack_topology::{GatewayConfig, Module, Placement, Plan};
use serde::{Deserialize, Serialize};

/// The id of the synthetic node standing for the host / browser (the source of ingress edges).
/// `@` cannot appear in a compose service name, so it never collides with a real service.
pub const HOST_NODE_ID: &str = "@host";

/// The diagram: modules (for grouping and selection), their runtime services, and typed edges
/// between services.
#[derive(Debug, Clone, Serialize)]
pub struct GraphDto {
    /// The modules in the plan, topologically ordered (dependencies first). A service's
    /// [`module`](ServiceNodeDto::module) points here.
    pub nodes: Vec<GraphNodeDto>,
    /// The runtime services: long-running compose services, external services, and the
    /// [`HOST_NODE_ID`] ingress node.
    pub services: Vec<ServiceNodeDto>,
    /// Typed, request-direction edges between [`services`](Self::services).
    pub edges: Vec<TopologyEdgeDto>,
}

/// One module: enriched with the role/placement of its primary service, so the UI can color and
/// icon it without a second lookup.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNodeDto {
    /// Module id (matches [`ServiceNodeDto::module`]).
    pub id: String,
    /// Human-readable label; falls back to the id when unset.
    pub display_name: Option<String>,
    /// One-line summary, for a tooltip.
    pub summary: Option<String>,
    /// Wizard category, if any.
    pub category: Option<String>,
    /// The role of the module's primary service (e.g. `"object_store"`, `"gateway"`). `None`
    /// for a module that contributes no service.
    pub role: Option<String>,
    /// Where the module's primary service runs (`in_process` / `host` / `container:<service>`),
    /// as a display string. `None` when it contributes no service.
    pub placement: Option<String>,
}

/// What a [`ServiceNodeDto`] stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// A long-running compose service.
    Container,
    /// A service the plan references but does not run in compose (host-bound or in-process).
    External,
    /// The synthetic host / browser node ([`HOST_NODE_ID`]).
    Host,
}

/// One runtime service in the diagram.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceNodeDto {
    /// The compose service name (its DNS name on the network), or [`HOST_NODE_ID`].
    pub id: String,
    /// The owning module's id; `None` only for the host node.
    pub module: Option<String>,
    /// What this node stands for.
    pub kind: ServiceKind,
    /// The service's declared role when it is one of the module's `ServiceSpec`s; `None` for a
    /// sidecar the fragment adds on its own (e.g. `sts-shim`).
    pub role: Option<String>,
    /// The container image, as rendered.
    pub image: Option<String>,
    /// Host-published ports (`ports:`).
    pub published: Vec<PortDto>,
    /// In-network ports (`expose:`).
    pub exposed: Vec<u16>,
}

/// A host-published port mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PortDto {
    /// The host-side port.
    pub host: u16,
    /// The container-side port.
    pub container: u16,
}

/// A gateway route aggregated onto a [`TopologyEdgeDto::Route`] edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteRefDto {
    /// The client-facing prefix matched.
    pub prefix: String,
    /// The host port of the listener carrying the route.
    pub host_port: u16,
    /// The upstream rewrite, if the path is changed before forwarding.
    pub rewrite: Option<String>,
    /// Whether the route sits behind the forward-auth check.
    pub gated: bool,
}

/// A typed, request-direction edge (`from` calls / waits on `to`).
///
/// Internally tagged, with the endpoints on every variant, so it serializes to a flat JS object
/// (`{ kind, from, to, … }`) through `serde-wasm-bindgen` without needing map support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopologyEdgeDto {
    /// A compose `depends_on` gate. Gates through a dropped one-shot job are collapsed onto the
    /// job's own dependencies.
    Startup {
        /// The waiting service.
        from: String,
        /// The service waited on.
        to: String,
        /// The compose condition (`service_started` / `service_healthy` / …).
        condition: String,
    },
    /// The gateway forwards these routes to an upstream service.
    Route {
        /// The gateway service.
        from: String,
        /// The upstream service.
        to: String,
        /// Every route the gateway forwards to `to`, most-specific-first per listener.
        routes: Vec<RouteRefDto>,
    },
    /// The gateway checks every gated request with the auth provider (`ext_authz`).
    Authz {
        /// The gateway service.
        from: String,
        /// The auth provider service.
        to: String,
        /// Where the provider's login portal is routed (open, not gated).
        portal_prefix: String,
    },
    /// Emulated public hostnames: a client calls real-cloud hostnames (e.g. AWS S3/STS), which
    /// resolve to the gateway's in-network TLS listener; the gateway forwards them to the local
    /// stand-in.
    Emulated {
        /// The caller: a client trusting the gateway's CA, or the gateway itself.
        from: String,
        /// The gateway (for a client), or the upstream answering the hostnames.
        to: String,
        /// The hostnames carried on this hop.
        hosts: Vec<String>,
        /// The TLS listener's in-network port.
        port: u16,
    },
    /// Host-published ports reaching a service from outside compose.
    Ingress {
        /// Always [`HOST_NODE_ID`].
        from: String,
        /// The publishing service.
        to: String,
        /// The host ports published by `to`.
        host_ports: Vec<u16>,
    },
}

impl TopologyEdgeDto {
    /// The edge's endpoints.
    pub fn endpoints(&self) -> (&str, &str) {
        match self {
            Self::Startup { from, to, .. }
            | Self::Route { from, to, .. }
            | Self::Authz { from, to, .. }
            | Self::Emulated { from, to, .. }
            | Self::Ingress { from, to, .. } => (from, to),
        }
    }
}

// --- compose fragment parsing --------------------------------------------------------------

/// The slice of a compose fragment the diagram reads. Unknown keys are ignored.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Fragment {
    #[serde(default)]
    pub(crate) services: BTreeMap<String, FragmentService>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FragmentService {
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    restart: Option<String>,
    #[serde(default)]
    pub(crate) depends_on: DependsOn,
    #[serde(default)]
    ports: Vec<serde_yaml::Value>,
    #[serde(default)]
    expose: Vec<serde_yaml::Value>,
    #[serde(default)]
    volumes: Vec<serde_yaml::Value>,
}

/// Compose `depends_on`, in either its short (list) or long (map) form.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum DependsOn {
    List(Vec<String>),
    Map(BTreeMap<String, DependsEntry>),
}

impl Default for DependsOn {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DependsEntry {
    #[serde(default)]
    condition: Option<String>,
}

/// The compose default when `depends_on` names a service without a condition.
const SERVICE_STARTED: &str = "service_started";
const SERVICE_COMPLETED: &str = "service_completed_successfully";

impl DependsOn {
    /// `(service, condition)` pairs, in name order.
    pub(crate) fn gates(&self) -> Vec<(String, String)> {
        match self {
            Self::List(names) => names
                .iter()
                .map(|n| (n.clone(), SERVICE_STARTED.to_string()))
                .collect(),
            Self::Map(map) => map
                .iter()
                .map(|(n, e)| {
                    let condition = e.condition.as_deref().unwrap_or(SERVICE_STARTED);
                    (n.clone(), condition.to_string())
                })
                .collect(),
        }
    }
}

/// Parse a rendered fragment. An empty fragment (an env-only module) has no services.
pub(crate) fn parse_fragment(fragment: &str) -> Result<Fragment, serde_yaml::Error> {
    if fragment.lines().all(|l| {
        let l = l.trim();
        l.is_empty() || l.starts_with('#')
    }) {
        return Ok(Fragment::default());
    }
    serde_yaml::from_str(fragment)
}

/// Parse one `ports:` entry: short syntax (`"9080:10000"`, `"127.0.0.1:9080:10000/tcp"`) or the
/// long `{ published, target }` form. A container-only entry (no host side) publishes nothing.
fn parse_port(value: &serde_yaml::Value) -> Option<PortDto> {
    match value {
        serde_yaml::Value::String(s) => {
            let s = s.split('/').next().unwrap_or(s);
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() < 2 {
                return None;
            }
            let container = parts[parts.len() - 1].parse().ok()?;
            let host = parts[parts.len() - 2].parse().ok()?;
            Some(PortDto { host, container })
        }
        serde_yaml::Value::Mapping(m) => {
            let num = |key: &str| -> Option<u16> {
                match m.get(key)? {
                    serde_yaml::Value::Number(n) => n.as_u64()?.try_into().ok(),
                    serde_yaml::Value::String(s) => s.parse().ok(),
                    _ => None,
                }
            };
            Some(PortDto {
                host: num("published")?,
                container: num("target")?,
            })
        }
        _ => None,
    }
}

/// Parse one `expose:` entry (`"9000"`, `9000`, or `"9000/tcp"`).
fn parse_expose(value: &serde_yaml::Value) -> Option<u16> {
    match value {
        serde_yaml::Value::Number(n) => n.as_u64()?.try_into().ok(),
        serde_yaml::Value::String(s) => s.split('/').next()?.parse().ok(),
        _ => None,
    }
}

/// The source of one `volumes:` entry (`"src:dst[:mode]"` or `{ source, … }`).
fn volume_source(value: &serde_yaml::Value) -> Option<&str> {
    match value {
        serde_yaml::Value::String(s) => s.split(':').next(),
        serde_yaml::Value::Mapping(m) => m.get("source")?.as_str(),
        _ => None,
    }
}

impl FragmentService {
    /// A one-shot job runs to completion rather than staying up: compose's default restart
    /// policy (`no`, i.e. the key is absent) or `on-failure`. Long-running services in the
    /// templates all declare `unless-stopped` / `always`.
    fn restart_says_job(&self) -> bool {
        matches!(
            self.restart.as_deref(),
            None | Some("no") | Some("on-failure")
        )
    }
}

// --- projection ------------------------------------------------------------------------------

/// Render a [`Placement`] to the display string the UI shows on a node.
pub(crate) fn placement_str(placement: &Placement) -> String {
    match placement {
        Placement::InProcess => "in_process".to_string(),
        Placement::Host => "host".to_string(),
        Placement::Container { service } => format!("container:{service}"),
    }
}

/// Project a settled [`Plan`] into the diagram [`GraphDto`].
///
/// A fragment that fails to parse contributes no services rather than failing the plan — the
/// planner's own output is valid compose (the golden tests check it), and the diagram should
/// degrade, not block the editor. The unit tests assert every baseline fragment parses.
pub fn graph_dto(plan: &Plan) -> GraphDto {
    let nodes = module_nodes(plan);

    // Every compose service, keyed by name, with its owning module.
    let mut compose: BTreeMap<String, (String, FragmentService)> = BTreeMap::new();
    for include in &plan.head.includes {
        let Ok(fragment) = parse_fragment(&include.fragment) else {
            continue;
        };
        for (name, svc) in fragment.services {
            compose.insert(name, (include.module.as_str().to_string(), svc));
        }
    }

    let jobs = one_shot_jobs(&compose);

    // Declared roles, by compose service name.
    let mut roles: BTreeMap<String, String> = BTreeMap::new();
    for specs in plan.services.values() {
        for spec in specs {
            if let Placement::Container { service } = &spec.placement {
                roles.insert(service.clone(), spec.role.as_str().to_string());
            }
        }
    }

    let mut services: Vec<ServiceNodeDto> = Vec::new();
    // Container services, in module (dependency) order, then name order within a module.
    for module in &plan.graph.nodes {
        let module_id = module.id().as_str();
        for (name, (owner, svc)) in &compose {
            if owner != module_id || jobs.contains(name) {
                continue;
            }
            services.push(ServiceNodeDto {
                id: name.clone(),
                module: Some(owner.clone()),
                kind: ServiceKind::Container,
                role: roles.get(name).cloned(),
                image: svc.image.clone(),
                published: svc.ports.iter().filter_map(parse_port).collect(),
                exposed: svc.expose.iter().filter_map(parse_expose).collect(),
            });
        }
    }
    // Services a module declares outside compose (host-bound / in-process).
    for (module_id, specs) in &plan.services {
        for spec in specs {
            if matches!(spec.placement, Placement::Container { .. }) {
                continue;
            }
            services.push(ServiceNodeDto {
                id: spec.name.clone(),
                module: Some(module_id.as_str().to_string()),
                kind: ServiceKind::External,
                role: Some(spec.role.as_str().to_string()),
                image: None,
                published: Vec::new(),
                exposed: Vec::new(),
            });
        }
    }

    let mut edges = startup_edges(&compose, &jobs);
    let gateway_service = gateway_service(plan, &compose, &jobs);
    if let Some(gw) = &gateway_service {
        edges.extend(gateway_edges(&plan.gateway, gw, &compose, &jobs));
    }
    let ingress = ingress_edges(&services);
    if !ingress.is_empty() {
        services.insert(
            0,
            ServiceNodeDto {
                id: HOST_NODE_ID.to_string(),
                module: None,
                kind: ServiceKind::Host,
                role: None,
                image: None,
                published: Vec::new(),
                exposed: Vec::new(),
            },
        );
        edges.extend(ingress);
    }

    GraphDto {
        nodes,
        services,
        edges,
    }
}

/// The module list, each enriched with its primary service's role and placement.
fn module_nodes(plan: &Plan) -> Vec<GraphNodeDto> {
    plan.graph
        .nodes
        .iter()
        .map(|m: &std::sync::Arc<dyn Module>| {
            let id = m.id().as_str().to_string();
            let svc = plan
                .services
                .iter()
                .find(|(mid, _)| mid.as_str() == id)
                .and_then(|(_, specs)| specs.first());
            GraphNodeDto {
                display_name: m.display_name().map(str::to_string),
                summary: m.summary().map(str::to_string),
                category: m.category().map(str::to_string),
                role: svc.map(|s| s.role.as_str().to_string()),
                placement: svc.map(|s| placement_str(&s.placement)),
                id,
            }
        })
        .collect()
}

/// The compose services that are one-shot jobs: those whose restart policy says so, plus any
/// another service waits on with `service_completed_successfully` (only a job can complete).
fn one_shot_jobs(compose: &BTreeMap<String, (String, FragmentService)>) -> BTreeSet<String> {
    let mut jobs: BTreeSet<String> = compose
        .iter()
        .filter(|(_, (_, svc))| svc.restart_says_job())
        .map(|(name, _)| name.clone())
        .collect();
    for (_, svc) in compose.values() {
        for (dep, condition) in svc.depends_on.gates() {
            if condition == SERVICE_COMPLETED && compose.contains_key(&dep) {
                jobs.insert(dep);
            }
        }
    }
    jobs
}

/// The effective startup gates of a service: its own `depends_on`, with any gate on a dropped
/// job replaced by the job's own (recursively) gates. Gates on services missing from compose
/// are dropped.
fn effective_gates(
    name: &str,
    compose: &BTreeMap<String, (String, FragmentService)>,
    jobs: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<(String, String)> = compose
        .get(name)
        .map(|(_, svc)| svc.depends_on.gates())
        .unwrap_or_default();
    stack.reverse();
    // Direct gates take precedence over one collapsed through a job onto the same service.
    let direct: BTreeSet<String> = stack
        .iter()
        .filter(|(d, _)| !jobs.contains(d))
        .map(|(d, _)| d.clone())
        .collect();
    while let Some((dep, condition)) = stack.pop() {
        if !seen.insert(dep.clone()) || dep == name {
            continue;
        }
        if jobs.contains(&dep) {
            if let Some((_, job)) = compose.get(&dep) {
                let mut inner = job.depends_on.gates();
                inner.reverse();
                stack.extend(inner.into_iter().filter(|(d, _)| !direct.contains(d)));
            }
        } else if compose.contains_key(&dep) {
            out.push((dep, condition));
        }
    }
    out
}

/// One [`TopologyEdgeDto::Startup`] edge per effective gate of every long-running service.
fn startup_edges(
    compose: &BTreeMap<String, (String, FragmentService)>,
    jobs: &BTreeSet<String>,
) -> Vec<TopologyEdgeDto> {
    let mut edges = Vec::new();
    for name in compose.keys() {
        if jobs.contains(name) {
            continue;
        }
        for (to, condition) in effective_gates(name, compose, jobs) {
            edges.push(TopologyEdgeDto::Startup {
                from: name.clone(),
                to,
                condition,
            });
        }
    }
    edges
}

/// The compose service filling the `gateway` role, if any.
fn gateway_service(
    plan: &Plan,
    compose: &BTreeMap<String, (String, FragmentService)>,
    jobs: &BTreeSet<String>,
) -> Option<String> {
    plan.services
        .values()
        .flatten()
        .filter(|s| s.role == olai_stack_topology::Role::gateway())
        .find_map(|s| match &s.placement {
            Placement::Container { service }
                if compose.contains_key(service) && !jobs.contains(service) =>
            {
                Some(service.clone())
            }
            _ => None,
        })
}

/// Route, auth and emulated-hostname edges out of the gateway, from the structured
/// [`GatewayConfig`]. A cluster's `host` is the upstream's compose DNS name, i.e. its service.
fn gateway_edges(
    gateway: &GatewayConfig,
    gw: &str,
    compose: &BTreeMap<String, (String, FragmentService)>,
    jobs: &BTreeSet<String>,
) -> Vec<TopologyEdgeDto> {
    let live = |svc: &str| compose.contains_key(svc) && !jobs.contains(svc);
    let host_of = |cluster: &str| -> Option<&str> {
        gateway
            .clusters
            .iter()
            .find(|c| c.name == cluster)
            .map(|c| c.host.as_str())
    };
    let auth_host = gateway.auth.as_ref().and_then(|a| host_of(&a.cluster));

    let mut edges = Vec::new();

    // Routes, aggregated per upstream service (first-seen order). The auth provider's portal
    // route is folded into the `Authz` edge instead.
    let mut routes: Vec<(String, Vec<RouteRefDto>)> = Vec::new();
    for (i, listener) in gateway.listeners.iter().enumerate() {
        for route in &listener.routes {
            let Some(host) = host_of(&route.cluster) else {
                continue;
            };
            if !live(host) || Some(host) == auth_host {
                continue;
            }
            let r = RouteRefDto {
                prefix: route.prefix.clone(),
                host_port: listener.host_port,
                rewrite: route.rewrite.clone(),
                // Mirrors the Envoy renderer: only the shared (first) listener is gated.
                gated: i == 0 && gateway.auth.is_some(),
            };
            match routes.iter_mut().find(|(h, _)| h == host) {
                Some((_, rs)) => rs.push(r),
                None => routes.push((host.to_string(), vec![r])),
            }
        }
    }
    for (to, routes) in routes {
        edges.push(TopologyEdgeDto::Route {
            from: gw.to_string(),
            to,
            routes,
        });
    }

    if let (Some(auth), Some(host)) = (&gateway.auth, auth_host)
        && live(host)
    {
        edges.push(TopologyEdgeDto::Authz {
            from: gw.to_string(),
            to: host.to_string(),
            portal_prefix: auth.portal_prefix.clone(),
        });
    }

    if let Some(tls) = &gateway.tls {
        // Clients: every service mounting the gateway's CA volume calls the emulated hostnames.
        // (The gateway mounts it too, to serve the certificate — it is not its own client.)
        for (name, (_, svc)) in compose {
            if name == gw || jobs.contains(name) {
                continue;
            }
            if svc
                .volumes
                .iter()
                .any(|v| volume_source(v) == Some(tls.trust.volume.as_str()))
            {
                edges.push(TopologyEdgeDto::Emulated {
                    from: name.clone(),
                    to: gw.to_string(),
                    hosts: tls.hosts.clone(),
                    port: tls.port,
                });
            }
        }
        // Upstreams: one hop per virtual host.
        for vhost in &tls.virtual_hosts {
            let Some(host) = host_of(&vhost.cluster) else {
                continue;
            };
            if live(host) {
                edges.push(TopologyEdgeDto::Emulated {
                    from: gw.to_string(),
                    to: host.to_string(),
                    hosts: vhost.hosts.clone(),
                    port: tls.port,
                });
            }
        }
    }
    edges
}

/// One [`TopologyEdgeDto::Ingress`] edge per service that publishes host ports.
fn ingress_edges(services: &[ServiceNodeDto]) -> Vec<TopologyEdgeDto> {
    services
        .iter()
        .filter(|s| !s.published.is_empty())
        .map(|s| TopologyEdgeDto::Ingress {
            from: HOST_NODE_ID.to_string(),
            to: s.id.clone(),
            host_ports: s.published.iter().map(|p| p.host).collect(),
        })
        .collect()
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

    fn service_ids(g: &GraphDto) -> Vec<&str> {
        g.services.iter().map(|s| s.id.as_str()).collect()
    }

    fn has_startup(g: &GraphDto, from: &str, to: &str) -> bool {
        g.edges.iter().any(|e| {
            matches!(e, TopologyEdgeDto::Startup { from: f, to: t, .. } if f == from && t == to)
        })
    }

    #[test]
    fn every_baseline_fragment_parses() {
        for auth in [false, true] {
            let p = plan(&lakehouse(auth));
            for include in &p.head.includes {
                parse_fragment(&include.fragment).unwrap_or_else(|e| {
                    panic!("{} fragment does not parse: {e}", include.module.as_str())
                });
            }
        }
    }

    #[test]
    fn one_shot_jobs_are_hidden_and_collapsed() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let ids = service_ids(&g);
        for job in ["rustfs-init", "headwaters-migrate", "gateway-certs"] {
            assert!(!ids.contains(&job), "job {job} should be hidden: {ids:?}");
        }
        // Long-running sidecars stay, inside their module.
        let shim = g.services.iter().find(|s| s.id == "sts-shim").unwrap();
        assert_eq!(shim.module.as_deref(), Some("rustfs"));
        // headwaters → headwaters-migrate → db collapses to headwaters → db.
        assert!(has_startup(&g, "headwaters", "db"), "{:#?}", g.edges);
        // mlflow → rustfs-init → rustfs collapses to mlflow → rustfs.
        assert!(has_startup(&g, "mlflow", "rustfs"), "{:#?}", g.edges);
        assert!(has_startup(&g, "sts-shim", "rustfs"));
        // No edge ever references a hidden node.
        assert_edges_reference_known_services(&g);
    }

    #[test]
    fn resolver_requires_edges_are_not_drawn() {
        let g = graph_dto(&plan(&lakehouse(false)));
        // mlflow `requires` envoy, but compose has no such gate: it is a route, gateway → mlflow.
        assert!(!has_startup(&g, "mlflow", "envoy"));
        let route = g
            .edges
            .iter()
            .find_map(|e| match e {
                TopologyEdgeDto::Route { from, to, routes }
                    if from == "envoy" && to == "mlflow" =>
                {
                    Some(routes)
                }
                _ => None,
            })
            .expect("envoy should route to mlflow");
        assert!(
            route
                .iter()
                .any(|r| r.prefix.starts_with("/api/2.0/mlflow"))
        );
        assert!(route.iter().all(|r| !r.gated), "auth is off");
    }

    #[test]
    fn auth_provider_is_wired_to_the_gateway() {
        let g = graph_dto(&plan(&lakehouse(true)));
        assert!(has_startup(&g, "envoy", "authelia"), "{:#?}", g.edges);
        assert!(g.edges.iter().any(|e| matches!(
            e,
            TopologyEdgeDto::Authz { from, to, .. } if from == "envoy" && to == "authelia"
        )));
        // API/UI routes on the shared listener are gated; dedicated listeners stay open.
        for e in &g.edges {
            if let TopologyEdgeDto::Route { to, routes, .. } = e {
                assert_ne!(to, "authelia", "the portal route folds into the Authz edge");
                for r in routes {
                    let shared = r.host_port == PlanCtx::default().gateway_host_port;
                    assert_eq!(r.gated, shared, "{to} {r:?}");
                }
            }
        }
    }

    #[test]
    fn emulated_aws_path_goes_through_the_gateway() {
        let g = graph_dto(&plan(&lakehouse(false)));
        let emulated: Vec<(&str, &str)> = g
            .edges
            .iter()
            .filter(|e| matches!(e, TopologyEdgeDto::Emulated { .. }))
            .map(TopologyEdgeDto::endpoints)
            .collect();
        // UC mounts the gateway CA and calls the AWS hostnames; MLflow uses an endpoint override.
        assert!(
            emulated.contains(&("unitycatalog", "envoy")),
            "{emulated:?}"
        );
        assert!(
            !emulated.iter().any(|(f, _)| *f == "mlflow"),
            "{emulated:?}"
        );
        assert!(emulated.contains(&("envoy", "sts-shim")), "{emulated:?}");
        assert!(emulated.contains(&("envoy", "rustfs")), "{emulated:?}");
    }

    #[test]
    fn host_ingress_reaches_published_services() {
        let g = graph_dto(&plan(&lakehouse(false)));
        assert_eq!(g.services[0].id, HOST_NODE_ID);
        let ingress_to: Vec<&str> = g
            .edges
            .iter()
            .filter(|e| matches!(e, TopologyEdgeDto::Ingress { .. }))
            .map(|e| e.endpoints().1)
            .collect();
        assert!(ingress_to.contains(&"envoy"), "{ingress_to:?}");
        let envoy = g.services.iter().find(|s| s.id == "envoy").unwrap();
        assert!(
            envoy
                .published
                .iter()
                .any(|p| p.host == PlanCtx::default().gateway_host_port),
            "{envoy:?}"
        );
    }

    /// Drift guard: every compose `depends_on` gate between long-running services is drawn,
    /// and every drawn gate exists in compose (directly or through a job).
    #[test]
    fn startup_edges_match_compose_depends_on() {
        let p = plan(&lakehouse(true));
        let g = graph_dto(&p);
        let mut compose: BTreeMap<String, Fragment> = BTreeMap::new();
        for include in &p.head.includes {
            compose.insert(
                include.module.as_str().to_string(),
                parse_fragment(&include.fragment).unwrap(),
            );
        }
        let all: BTreeMap<&String, &FragmentService> =
            compose.values().flat_map(|f| f.services.iter()).collect();
        let live: BTreeSet<&str> = service_ids(&g).into_iter().collect();
        for (name, svc) in &all {
            if !live.contains(name.as_str()) {
                continue;
            }
            for (dep, _) in svc.depends_on.gates() {
                if live.contains(dep.as_str()) {
                    assert!(has_startup(&g, name, &dep), "missing {name} → {dep}");
                }
            }
        }
        for e in &g.edges {
            if let TopologyEdgeDto::Startup { from, to, .. } = e {
                let direct = all[from].depends_on.gates().iter().any(|(d, _)| d == to);
                let via_job = all[from].depends_on.gates().iter().any(|(d, _)| {
                    !live.contains(d.as_str())
                        && all
                            .get(d)
                            .is_some_and(|j| j.depends_on.gates().iter().any(|(x, _)| x == to))
                });
                assert!(direct || via_job, "drawn {from} → {to} is not in compose");
            }
        }
    }

    fn assert_edges_reference_known_services(g: &GraphDto) {
        let ids: BTreeSet<&str> = service_ids(g).into_iter().collect();
        for e in &g.edges {
            let (from, to) = e.endpoints();
            assert!(
                ids.contains(from) && ids.contains(to),
                "dangling edge {e:?}"
            );
        }
    }

    #[test]
    fn parses_port_and_depends_on_forms() {
        let f = parse_fragment(
            "services:\n  a:\n    restart: unless-stopped\n    ports:\n      - \"127.0.0.1:9080:10000/tcp\"\n      - \"5000\"\n      - published: 81\n        target: 80\n    expose: [9000, \"9001/tcp\"]\n    depends_on: [b]\n  b:\n    restart: always\n    depends_on:\n      c: {}\n",
        )
        .unwrap();
        let a = &f.services["a"];
        let ports: Vec<PortDto> = a.ports.iter().filter_map(parse_port).collect();
        assert_eq!(
            ports,
            vec![
                PortDto {
                    host: 9080,
                    container: 10000
                },
                PortDto {
                    host: 81,
                    container: 80
                }
            ]
        );
        let exposed: Vec<u16> = a.expose.iter().filter_map(parse_expose).collect();
        assert_eq!(exposed, vec![9000, 9001]);
        assert_eq!(
            a.depends_on.gates(),
            vec![("b".into(), SERVICE_STARTED.into())]
        );
        assert_eq!(
            f.services["b"].depends_on.gates(),
            vec![("c".into(), SERVICE_STARTED.into())]
        );
        assert!(
            parse_fragment("# env-only\n\n")
                .unwrap()
                .services
                .is_empty()
        );
    }
}
