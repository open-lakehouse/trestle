//! The **planner** ([`plan`]): a module selection → a fully-assigned environment.
//!
//! This is the producer the rest of the crate was built to feed. A module declares
//! only intent ([`RouteIntent`] on its endpoints) and ingredients ([`Provides`]); the
//! planner is the one vantage that sees *every* selected module at once, so it is the
//! only place gateway prefixes can be assigned without colliding. Given a
//! [`Selection`] and a [`Catalog`], it:
//!
//! 1. resolves the dependency graph ([`resolve`]) — transitive `requires`, conflicts,
//!    topological order;
//! 2. derives a non-colliding gateway route for every surface endpoint, **erroring
//!    loudly on a real collision** rather than silently shadowing one route with
//!    another (the failure mode hand-authored routes invite);
//! 3. emits a structured [`GatewayConfig`] (listeners + clusters) for the gateway,
//!    and a [`RoutePlan`] the [`address`](crate::address) resolver consumes unchanged;
//! 4. renders every module to a [`RenderOutput`] and assembles a consolidated
//!    [`HeadFile`] (config mounts + injected env at the head, then `include:`s).
//!
//! # How a prefix is derived (the crux)
//!
//! Prefixes are derived from *declared semantic facts*, never invented from names:
//!
//! - An [`Api`](RouteIntent::Api) endpoint's prefix is its declared client mount
//!   (the `api_prefix:<endpoint_id>` extra; the endpoint's own `path` stays empty so
//!   the resolver does not double it). Its upstream rewrite is the service's
//!   `base_path` joined with that prefix — unless the module declares an explicit
//!   `rewrite:<prefix>` override (the rare exception, e.g. an OTel ingest path that
//!   must strip to root).
//! - A [`UiPrefixable`](RouteIntent::UiPrefixable) endpoint's prefix and chosen
//!   `base_path` are the module's declared `base_path` (e.g. `/mlflow`).
//! - A [`UiFixed`](RouteIntent::UiFixed) endpoint is given its own dedicated listener
//!   port (allocated by the consumer via [`PlanCtx`]).
//! - An [`Internal`](RouteIntent::Internal) endpoint gets no route (resolves direct).
//!
//! Two surface endpoints resolving to the same prefix is a
//! [`PlanError::PrefixCollision`]: the planner fails, and the fix is an explicit
//! prefix override on one of them — making the exception visible and local.

use std::collections::BTreeMap;

use crate::catalog::Catalog;
use crate::catalog::baseline::{
    API_PREFIX_EXTRA, BASE_PATH_EXTRA, REWRITE_OVERRIDE_PREFIX, S3_BUCKET_MB_LINES_VAR,
};
use crate::endpoint::{Endpoint, RouteIntent};
use crate::module::{Module, ModuleId};
use crate::placement::Placement;
use crate::plan::{AssignedRoute, Listener, RoutePlan};
use crate::render::{InjectedEnv, RenderOutput};
use crate::resolve_graph::{ResolveError, ResolvedGraph, resolve};
use crate::role::ServiceSpec;

/// What modules an environment should contain.
///
/// Modules can be picked directly (combine technologies) or by capability ("I want
/// experiment tracking"); the planner unions both. Capability names are matched
/// against the catalog's [`provider_of`](crate::Module::provider_of) index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Module ids selected directly.
    pub modules: Vec<ModuleId>,
    /// Capabilities selected; each is mapped to its provider module(s) in the catalog.
    pub capabilities: Vec<String>,
}

impl Selection {
    /// A selection of explicit module ids.
    pub fn modules<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<ModuleId>,
    {
        Selection {
            modules: ids.into_iter().map(Into::into).collect(),
            capabilities: Vec::new(),
        }
    }
}

/// Runtime facts the planner needs but cannot derive from the model — the same
/// posture as [`TopologyCtx`](crate::TopologyCtx). The consumer supplies these once
/// per environment.
#[derive(Clone, Debug)]
pub struct PlanCtx {
    /// The compose project / environment name (used as the head file's `name:`).
    pub env_name: String,
    /// The gateway's compose service / DNS name (e.g. `"envoy"`).
    pub gateway_service: String,
    /// The gateway's listening port inside the compose network (e.g. `10000`).
    pub gateway_internal_port: u16,
    /// The gateway's host-published port (e.g. `9080`).
    pub gateway_host_port: u16,
    /// Host ports to hand out, in order, to each [`UiFixed`](RouteIntent::UiFixed)
    /// endpoint that needs its own dedicated listener. The planner never allocates
    /// ports itself.
    #[allow(clippy::struct_field_names)]
    pub dedicated_listener_ports: Vec<u16>,
}

impl Default for PlanCtx {
    fn default() -> Self {
        PlanCtx {
            env_name: "lakehouse".into(),
            gateway_service: "envoy".into(),
            gateway_internal_port: 10000,
            gateway_host_port: 9080,
            dedicated_listener_ports: Vec::new(),
        }
    }
}

/// What can go wrong planning an environment.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    /// Resolving the module graph failed (unknown id, conflict, or cycle).
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// A requested capability has no provider module in the catalog.
    #[error("no module in the catalog provides capability `{0}`")]
    UnknownCapability(String),
    /// Two surface endpoints derived the same gateway prefix. The fix is an explicit
    /// prefix override on one of them.
    #[error(
        "prefix `{prefix}` is claimed by both `{first}` and `{second}`; \
         give one an explicit prefix override"
    )]
    PrefixCollision {
        /// The colliding prefix.
        prefix: String,
        /// `service.endpoint` that claimed it first.
        first: String,
        /// `service.endpoint` that collided with it.
        second: String,
    },
    /// A [`UiFixed`](RouteIntent::UiFixed) endpoint needs a dedicated listener port,
    /// but [`PlanCtx::dedicated_listener_ports`] ran out.
    #[error("ran out of dedicated listener ports for fixed-path UI `{0}`")]
    NoDedicatedPort(String),
    /// A surface endpoint derived an empty mount prefix, which would shadow every
    /// other route as a catch-all. An API needs an `api_prefix:<id>` extra; a
    /// prefixable UI needs a non-empty `base_path`.
    #[error("surface endpoint `{0}` has no mount prefix (would be a catch-all `/`)")]
    MissingPrefix(String),
    /// Two surface endpoints on one service declare different upstream ports, so a
    /// single service-named gateway cluster cannot serve both.
    #[error("service `{service}` has surface endpoints on conflicting ports {first} and {second}")]
    ClusterPortConflict {
        /// The service with conflicting endpoint ports.
        service: String,
        /// The port the first cluster was built on.
        first: u16,
        /// The conflicting port a later endpoint declared.
        second: u16,
    },
}

/// An upstream cluster the gateway forwards to — one per surface service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterConfig {
    /// The cluster's logical name (the service name).
    pub name: String,
    /// The upstream host (the service's compose DNS name).
    pub host: String,
    /// The upstream port (the endpoint's internal port).
    pub port: u16,
}

/// One route on a gateway listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayRoute {
    /// The client-facing prefix the listener matches.
    pub prefix: String,
    /// The upstream cluster name to forward to.
    pub cluster: String,
    /// The upstream rewrite, if the path is changed before forwarding. `None`
    /// forwards the matched path unchanged (no rewrite emitted); `Some(path)`
    /// rewrites the matched prefix to `path`.
    pub rewrite: Option<String>,
}

/// A gateway listener: a host port and the routes it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerConfig {
    /// The host-published port this listener binds.
    pub host_port: u16,
    /// The listener's internal port inside the compose network.
    pub internal_port: u16,
    /// The routes on this listener, most-specific-first (Envoy match priority).
    pub routes: Vec<GatewayRoute>,
}

/// The structured gateway configuration the planner emits — what a consumer turns
/// into an Envoy (or other) config, replacing hand-authored route/cluster lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GatewayConfig {
    /// The listeners (the shared one plus any dedicated for fixed-path UIs).
    pub listeners: Vec<ListenerConfig>,
    /// The upstream clusters, deduplicated by name.
    pub clusters: Vec<ClusterConfig>,
}

/// A compose `include:` entry contributed by a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposeInclude {
    /// The module that contributed this fragment.
    pub module: ModuleId,
    /// The fragment text (the module's rendered compose `services:` snippet).
    pub fragment: String,
}

/// The consolidated top-level compose plan: customization at the head (injected env),
/// then a plain list of module fragments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadFile {
    /// The compose project name.
    pub name: String,
    /// The environment-wide injected variables, gathered at the head.
    pub env: InjectedEnv,
    /// The module fragments to include, in dependency order.
    pub includes: Vec<ComposeInclude>,
}

/// A fully-assigned environment: the resolved graph, the routing plan, the rendered
/// modules, and the consolidated head + gateway config. Everything is data; the
/// consumer does the I/O (serialize, write, mount, launch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentPlan {
    /// The resolved, ordered module graph.
    pub graph: ResolvedGraph,
    /// The per-endpoint route assignments the [`address`](crate::address) resolver
    /// consumes.
    pub routes: RoutePlan,
    /// Each module's injected env (the values the planner decided for it).
    pub injected: BTreeMap<ModuleId, InjectedEnv>,
    /// Each module's rendered output (fragment + files).
    pub renders: Vec<(ModuleId, RenderOutput)>,
    /// The consolidated head/compose plan.
    pub head: HeadFile,
    /// The structured gateway config.
    pub gateway: GatewayConfig,
    /// Postgres databases the selected modules need, deduplicated in dependency order
    /// (drives the Postgres init artifact).
    pub postgres_databases: Vec<String>,
    /// Object-store buckets the selected modules need, deduplicated in dependency
    /// order (drives the SeaweedFS bucket-init).
    pub s3_buckets: Vec<String>,
    /// The stack's aggregated environment variables (drives the `.env` artifact),
    /// last-writer-wins in dependency order.
    pub env: InjectedEnv,
}

/// Plan an environment from a selection against a catalog.
///
/// See the module docs for the prefix-derivation rules. Returns a fully-materialized
/// [`EnvironmentPlan`], or a [`PlanError`] (notably [`PlanError::PrefixCollision`]).
pub fn plan(
    selection: &Selection,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<EnvironmentPlan, PlanError> {
    let selected = resolve_selection(selection, catalog)?;
    let graph = resolve(&selected, catalog.modules())?;

    let mut routes = RoutePlan::new();
    let mut gateway = GatewayConfig::default();
    let mut injected: BTreeMap<ModuleId, InjectedEnv> = BTreeMap::new();

    // The shared listener is always present; dedicated listeners are appended per
    // fixed-path UI as we encounter them.
    let mut shared_routes: Vec<GatewayRoute> = Vec::new();
    let mut dedicated: Vec<ListenerConfig> = Vec::new();
    let mut next_dedicated = ctx.dedicated_listener_ports.iter().copied();

    // Track claimed prefixes (per listener) to detect collisions. The shared listener
    // is keyed by its port; dedicated listeners never collide (own port).
    let mut claimed: BTreeMap<String, String> = BTreeMap::new();

    // Aggregate set-like resources across modules in dependency order (dedup,
    // order-preserving) for the artifacts the planner renders downstream.
    let mut postgres_databases: Vec<String> = Vec::new();
    let mut s3_buckets: Vec<String> = Vec::new();
    for module in &graph.nodes {
        for db in &module.provides.postgres_databases {
            if !postgres_databases.contains(db) {
                postgres_databases.push(db.clone());
            }
        }
        for b in &module.provides.s3_buckets {
            if !s3_buckets.contains(b) {
                s3_buckets.push(b.clone());
            }
        }
    }

    // Walk modules in dependency order so emitted routes/clusters are deterministic.
    for module in &graph.nodes {
        let mut module_env = InjectedEnv::new();
        // Seed each module's render env with its declared env vars. SeaweedFS also
        // needs the aggregated bucket list folded into its one-shot init block.
        for (k, v) in module.provides.env_vars.iter() {
            module_env.set(k, v);
        }
        if module.id.as_str() == "local-stack-seaweedfs" {
            module_env.set(S3_BUCKET_MB_LINES_VAR, seaweedfs_bucket_lines(&s3_buckets));
        }
        for service in &module.services {
            for endpoint in &service.endpoints {
                match &endpoint.intent {
                    RouteIntent::Internal => {} // no route
                    RouteIntent::Api => {
                        // The mount prefix is declared in extras, not on the
                        // endpoint's `path` (which stays empty so the resolver's
                        // `join(prefix, path)` round-trips to exactly the prefix).
                        let prefix = api_mount_prefix(module, &endpoint.id);
                        require_prefix(&prefix, service, &endpoint.id)?;
                        claim(&mut claimed, &prefix, service, &endpoint.id)?;
                        let rewrite = api_rewrite(module, &prefix);
                        ensure_cluster(&mut gateway, service, endpoint)?;
                        shared_routes.push(GatewayRoute {
                            prefix: prefix.clone(),
                            cluster: service.name.clone(),
                            rewrite: rewrite.clone(),
                        });
                        // One rewrite value feeds both the gateway config and the
                        // route plan, keeping them in lock-step.
                        routes.assign(
                            &service.name,
                            &endpoint.id,
                            AssignedRoute {
                                prefix,
                                rewrite,
                                listener: Listener::Shared,
                                base_path: None,
                            },
                        );
                    }
                    RouteIntent::UiPrefixable => {
                        let base = module_base_path(module);
                        // An empty base path would mount the UI at `/` — a catch-all
                        // that shadows every other route. A prefixable UI must declare
                        // a non-empty base path; if it truly serves at root it is a
                        // `UiFixed` with its own listener instead.
                        require_prefix(&base, service, &endpoint.id)?;
                        claim(&mut claimed, &base, service, &endpoint.id)?;
                        ensure_cluster(&mut gateway, service, endpoint)?;
                        shared_routes.push(GatewayRoute {
                            prefix: base.clone(),
                            cluster: service.name.clone(),
                            rewrite: None,
                        });
                        // Feed the chosen base path back to the module's render.
                        module_env.set("BASE_PATH", &base);
                        routes.assign(
                            &service.name,
                            &endpoint.id,
                            AssignedRoute {
                                prefix: base.clone(),
                                rewrite: None,
                                listener: Listener::Shared,
                                base_path: Some(base),
                            },
                        );
                    }
                    RouteIntent::UiFixed => {
                        let port = next_dedicated
                            .next()
                            .ok_or_else(|| PlanError::NoDedicatedPort(service.name.clone()))?;
                        ensure_cluster(&mut gateway, service, endpoint)?;
                        dedicated.push(ListenerConfig {
                            host_port: port,
                            internal_port: endpoint.internal_port,
                            routes: vec![GatewayRoute {
                                prefix: "/".into(),
                                cluster: service.name.clone(),
                                rewrite: None,
                            }],
                        });
                        routes.assign(
                            &service.name,
                            &endpoint.id,
                            AssignedRoute {
                                prefix: "/".into(),
                                rewrite: None,
                                listener: Listener::Dedicated { port },
                                base_path: None,
                            },
                        );
                    }
                }
            }
        }

        // Routes/env are decided here; rendering happens in a second pass below, once
        // every module's injected env is settled, so renders stay in dependency order.
        injected.insert(module.id.clone(), module_env);
    }

    // Order shared-listener routes most-specific-first (longer prefix wins), stable
    // within equal length by prefix so the output is deterministic.
    shared_routes.sort_by(|a, b| {
        b.prefix
            .len()
            .cmp(&a.prefix.len())
            .then_with(|| a.prefix.cmp(&b.prefix))
    });

    gateway.listeners.push(ListenerConfig {
        host_port: ctx.gateway_host_port,
        internal_port: ctx.gateway_internal_port,
        routes: shared_routes,
    });
    gateway.listeners.extend(dedicated);
    gateway.clusters.sort_by(|a, b| a.name.cmp(&b.name));

    // Aggregate the stack's env vars for `.env` — only modules' *declared* env vars,
    // last-writer-wins in dependency order. (Render-only injections like `BASE_PATH`
    // and `S3_BUCKET_MB_LINES` stay out of `.env`; they belong to a module's fragment.)
    let mut env = InjectedEnv::new();
    for module in &graph.nodes {
        for (k, v) in module.provides.env_vars.iter() {
            env.set(k, v);
        }
    }

    // Render every module from its decided env, in dependency order. A module with an
    // empty fragment (e.g. the env-only contract module) contributes no compose
    // include.
    let mut renders = Vec::with_capacity(graph.nodes.len());
    let mut includes = Vec::new();
    for module in &graph.nodes {
        let module_env = injected.get(&module.id).cloned().unwrap_or_default();
        let out = module.render.render(&module_env);
        if !out.fragment.trim().is_empty() {
            includes.push(ComposeInclude {
                module: module.id.clone(),
                fragment: out.fragment.clone(),
            });
        }
        renders.push((module.id.clone(), out));
    }

    let head = HeadFile {
        name: ctx.env_name.clone(),
        env: env.clone(),
        includes,
    };

    Ok(EnvironmentPlan {
        graph,
        routes,
        injected,
        renders,
        head,
        gateway,
        postgres_databases,
        s3_buckets,
        env,
    })
}

/// Resolve a selection into a flat list of directly-selected module ids (capabilities
/// mapped through the catalog's provider index), preserving order and deduplicating.
fn resolve_selection(selection: &Selection, catalog: &Catalog) -> Result<Vec<ModuleId>, PlanError> {
    let mut out: Vec<ModuleId> = Vec::new();
    let push = |id: ModuleId, out: &mut Vec<ModuleId>| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    for id in &selection.modules {
        push(id.clone(), &mut out);
    }
    for cap in &selection.capabilities {
        let providers = catalog.providers_of(cap);
        if providers.is_empty() {
            return Err(PlanError::UnknownCapability(cap.clone()));
        }
        for id in providers {
            push(id.clone(), &mut out);
        }
    }
    Ok(out)
}

/// Claim `prefix` for `service.endpoint`, erroring if another endpoint already did.
fn claim(
    claimed: &mut BTreeMap<String, String>,
    prefix: &str,
    service: &ServiceSpec,
    endpoint_id: &str,
) -> Result<(), PlanError> {
    let who = format!("{}.{}", service.name, endpoint_id);
    if let Some(first) = claimed.get(prefix) {
        return Err(PlanError::PrefixCollision {
            prefix: prefix.to_string(),
            first: first.clone(),
            second: who,
        });
    }
    claimed.insert(prefix.to_string(), who);
    Ok(())
}

/// Reject an empty mount prefix — it would become a `/` catch-all that shadows every
/// other route.
fn require_prefix(prefix: &str, service: &ServiceSpec, endpoint_id: &str) -> Result<(), PlanError> {
    if prefix.is_empty() {
        return Err(PlanError::MissingPrefix(format!(
            "{}.{}",
            service.name, endpoint_id
        )));
    }
    Ok(())
}

/// Ensure the gateway has a cluster (named for `service`) targeting `endpoint`.
///
/// The cluster port comes from the **routed endpoint** — not the service's first
/// endpoint, which may listen elsewhere. Routes reference the cluster by service name,
/// so a service exposes exactly one gateway cluster; if two of its surface endpoints
/// disagree on the upstream port, that is a modeling error the planner reports
/// ([`PlanError::ClusterPortConflict`]) rather than silently forwarding to the wrong
/// one.
fn ensure_cluster(
    gateway: &mut GatewayConfig,
    service: &ServiceSpec,
    endpoint: &Endpoint,
) -> Result<(), PlanError> {
    let port = endpoint.internal_port;
    if let Some(existing) = gateway.clusters.iter().find(|c| c.name == service.name) {
        if existing.port != port {
            return Err(PlanError::ClusterPortConflict {
                service: service.name.clone(),
                first: existing.port,
                second: port,
            });
        }
        return Ok(());
    }
    let host = match &service.placement {
        Placement::Container { service } => service.clone(),
        // Host/in-process services are reached via the host gateway DNS from a
        // container; the gateway cluster targets that.
        _ => "host.docker.internal".to_string(),
    };
    gateway.clusters.push(ClusterConfig {
        name: service.name.clone(),
        host,
        port,
    });
    Ok(())
}

/// The SeaweedFS one-shot bucket-init lines for the aggregated bucket list, one
/// `aws s3 mb` per bucket, indented to sit inside the fragment's `entrypoint` block.
/// Empty when there are no buckets.
fn seaweedfs_bucket_lines(buckets: &[String]) -> String {
    buckets
        .iter()
        .map(|b| {
            format!(
                "        aws --endpoint-url http://seaweedfs:8333 s3 mb s3://{b} 2>&1 || true;\n"
            )
        })
        .collect()
}

/// The base path a module's service serves itself under, from the `base_path` extra
/// (empty string if unset → service serves at root).
fn module_base_path(module: &Module) -> String {
    module
        .provides
        .extras
        .get(BASE_PATH_EXTRA)
        .cloned()
        .unwrap_or_default()
}

/// The client-facing mount prefix declared for an API endpoint, read from the
/// `api_prefix:<endpoint_id>` extra. The endpoint's own `path` stays empty so the
/// resolver's `join(prefix, path)` round-trips to exactly this prefix.
fn api_mount_prefix(module: &Module, endpoint_id: &str) -> String {
    module
        .provides
        .extras
        .get(&format!("{API_PREFIX_EXTRA}{endpoint_id}"))
        .cloned()
        .unwrap_or_default()
}

/// The gateway rewrite for an API route, for the structured [`GatewayConfig`].
///
/// Tri-state: `None` means forward the path unchanged (no rewrite emitted);
/// `Some(path)` means rewrite the matched prefix to `path`.
///
/// Resolution order: an explicit `rewrite:<prefix>` override on the module (the rare
/// exception) wins — an **empty** override value forces passthrough (`None`), a
/// non-empty one rewrites to that value. With no override, the rewrite is the
/// service's `base_path` joined with the client `prefix`; a service serving at root
/// (empty base path) needs no rewrite.
fn api_rewrite(module: &Module, prefix: &str) -> Option<String> {
    if let Some(over) = module
        .provides
        .extras
        .get(&format!("{REWRITE_OVERRIDE_PREFIX}{prefix}"))
    {
        // Empty override == "this route passes through unchanged" (the gateway emits
        // no rewrite block), matching how the trestle templates treat an empty
        // rewrite. A non-empty override is the literal upstream path.
        return if over.is_empty() {
            None
        } else {
            Some(over.clone())
        };
    }
    let base = module_base_path(module);
    if base.is_empty() {
        None
    } else {
        Some(join_path(&base, prefix))
    }
}

/// Join two path segments with exactly one slash at the seam, no trailing slash
/// duplication.
fn join_path(a: &str, b: &str) -> String {
    let a = a.trim_end_matches('/');
    let b = b.trim_start_matches('/');
    if b.is_empty() {
        a.to_string()
    } else if a.is_empty() {
        format!("/{b}")
    } else {
        format!("{a}/{b}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::baseline_catalog;

    fn default_selection() -> Selection {
        Selection::modules([
            "local-stack-envoy",
            "local-stack-postgres",
            "local-stack-seaweedfs",
            "local-stack-mlflow",
            "local-stack-unity-catalog",
        ])
    }

    fn shared_routes(plan: &EnvironmentPlan) -> &[GatewayRoute] {
        &plan.gateway.listeners[0].routes
    }

    fn route_for<'a>(plan: &'a EnvironmentPlan, prefix: &str) -> &'a GatewayRoute {
        shared_routes(plan)
            .iter()
            .find(|r| r.prefix == prefix)
            .unwrap_or_else(|| panic!("no route for {prefix}"))
    }

    /// A one-service module with `endpoints`, for exercising planner guards.
    fn module_with(
        id: &str,
        endpoints: Vec<Endpoint>,
        provides: crate::module::Provides,
    ) -> Module {
        Module {
            id: id.into(),
            display_name: None,
            summary: None,
            category: None,
            provider_of: None,
            requires: vec![],
            conflicts_with: vec![],
            services: vec![ServiceSpec {
                name: id.to_string(),
                role: crate::role::Role::new("svc"),
                placement: Placement::Container {
                    service: id.to_string(),
                },
                endpoints,
                depends_on: vec![],
            }],
            provides,
            knobs: vec![],
            render: Default::default(),
        }
    }

    fn ep(id: &str, port: u16, intent: RouteIntent) -> Endpoint {
        Endpoint {
            id: id.into(),
            scheme: crate::endpoint::Scheme::Http,
            internal_port: port,
            host_port: None,
            intent,
            path: String::new(),
        }
    }

    #[test]
    fn empty_ui_base_path_is_rejected_not_a_catch_all() {
        // A UiPrefixable with no base_path would mount at `/` and shadow everything.
        let m = module_with(
            "svc",
            vec![ep("ui", 8080, RouteIntent::UiPrefixable)],
            Default::default(),
        );
        let err = plan(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([m]),
            &PlanCtx::default(),
        )
        .unwrap_err();
        assert_eq!(err, PlanError::MissingPrefix("svc.ui".into()));
    }

    #[test]
    fn conflicting_endpoint_ports_on_one_service_error() {
        // Two API endpoints on the same service but different ports can't share one
        // service-named cluster.
        let mut provides = crate::module::Provides::default();
        provides
            .extras
            .insert(format!("{API_PREFIX_EXTRA}a"), "/a".into());
        provides
            .extras
            .insert(format!("{API_PREFIX_EXTRA}b"), "/b".into());
        let m = module_with(
            "svc",
            vec![
                ep("a", 8080, RouteIntent::Api),
                ep("b", 9090, RouteIntent::Api),
            ],
            provides,
        );
        let err = plan(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([m]),
            &PlanCtx::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            PlanError::ClusterPortConflict {
                service: "svc".into(),
                first: 8080,
                second: 9090,
            }
        );
    }

    #[test]
    fn default_lakehouse_rederives_working_routes() {
        let p = plan(
            &default_selection(),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .unwrap();

        // MLflow tracking API rewrites under the service base path.
        let mlflow = route_for(&p, "/api/2.0/mlflow");
        assert_eq!(mlflow.cluster, "mlflow");
        assert_eq!(mlflow.rewrite.as_deref(), Some("/mlflow/api/2.0/mlflow"));

        // MLflow OTel route is the override exception: it passes through unchanged
        // (the empty override forces no rewrite), unlike the tracking API.
        let otel = route_for(&p, "/api/2.0/otel");
        assert_eq!(otel.rewrite, None);

        // MLflow UI fronts at its base path, no rewrite.
        let ui = route_for(&p, "/mlflow");
        assert_eq!(ui.cluster, "mlflow");
        assert_eq!(ui.rewrite, None);

        // Unity Catalog REST fronts at the Databricks-shaped path, served at root → no
        // rewrite.
        let uc = route_for(&p, "/api/2.1/unity-catalog");
        assert_eq!(uc.cluster, "unitycatalog");
        assert_eq!(uc.rewrite, None);
        route_for(&p, "/unity-catalog");
    }

    #[test]
    fn ui_base_path_is_injected_for_render() {
        let p = plan(
            &default_selection(),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .unwrap();
        let env = p.injected.get(&"local-stack-mlflow".into()).unwrap();
        assert_eq!(env.get("BASE_PATH"), Some("/mlflow"));
    }

    #[test]
    fn routes_are_most_specific_first() {
        let p = plan(
            &default_selection(),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .unwrap();
        let lens: Vec<usize> = shared_routes(&p).iter().map(|r| r.prefix.len()).collect();
        let mut sorted = lens.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(lens, sorted, "routes must be ordered longest-prefix-first");
    }

    #[test]
    fn clusters_are_derived_from_placement() {
        let p = plan(
            &default_selection(),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .unwrap();
        let mlflow = p
            .gateway
            .clusters
            .iter()
            .find(|c| c.name == "mlflow")
            .unwrap();
        assert_eq!(mlflow.host, "mlflow");
        assert_eq!(mlflow.port, 5000);
        let uc = p
            .gateway
            .clusters
            .iter()
            .find(|c| c.name == "unitycatalog")
            .unwrap();
        assert_eq!(uc.host, "unitycatalog");
        assert_eq!(uc.port, 8080);
    }

    #[test]
    fn capability_selection_pulls_in_provider() {
        let sel = Selection {
            modules: vec![],
            capabilities: vec!["experiment_tracking".into()],
        };
        let p = plan(&sel, &baseline_catalog(), &PlanCtx::default()).unwrap();
        // mlflow + its transitive requires (postgres, seaweedfs, envoy).
        for id in [
            "local-stack-mlflow",
            "local-stack-postgres",
            "local-stack-seaweedfs",
            "local-stack-envoy",
        ] {
            assert!(p.graph.module(&id.into()).is_some(), "missing {id}");
        }
    }

    #[test]
    fn unknown_capability_errors() {
        let sel = Selection {
            modules: vec![],
            capabilities: vec!["telepathy".into()],
        };
        let err = plan(&sel, &baseline_catalog(), &PlanCtx::default()).unwrap_err();
        assert_eq!(err, PlanError::UnknownCapability("telepathy".into()));
    }

    #[test]
    fn plan_is_deterministic_regardless_of_selection_order() {
        let cat = baseline_catalog();
        let a = plan(&default_selection(), &cat, &PlanCtx::default()).unwrap();
        let reversed = Selection::modules([
            "local-stack-unity-catalog",
            "local-stack-mlflow",
            "local-stack-seaweedfs",
            "local-stack-postgres",
            "local-stack-envoy",
        ]);
        let b = plan(&reversed, &cat, &PlanCtx::default()).unwrap();
        assert_eq!(a, b);
    }
}
