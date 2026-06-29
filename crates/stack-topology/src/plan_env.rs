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

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Catalog;
use crate::catalog::baseline::{
    API_PREFIX_EXTRA, BASE_PATH_EXTRA, DATA_ROOT_DEFAULT, DATA_ROOT_VAR, DEP_GATE_EXTRA,
    REWRITE_OVERRIDE_PREFIX,
};
use crate::connection::Connection;
use crate::endpoint::{Endpoint, RouteIntent};
use crate::module::{Module, ModuleId};
use crate::placement::Placement;
use crate::plan::{AssignedRoute, Listener, RoutePlan};
use crate::render::{InjectedEnv, RenderOutput};
use crate::resolve_graph::{ResolveError, ResolvedGraph, resolve};
use crate::role::{Role, ServiceSpec};

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
    /// Per-module knob overrides: module id → (knob `key` → value). A value present
    /// here wins over the knob's declared [`default`](crate::Knob::default); a knob
    /// absent from this map falls back to its default. The value lands in the module's
    /// [`InjectedEnv`] under the knob's `key`, exactly like any other planner-injected
    /// variable, so the module's template reads it as `{{ env.KEY }}`.
    ///
    /// This is the channel a config UI (hydrofoil / Transler) feeds: it surfaces a
    /// module's knobs from the catalog, lets the user tune them, and hands the chosen
    /// values back here.
    pub knob_overrides: BTreeMap<ModuleId, BTreeMap<String, String>>,
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
            knob_overrides: BTreeMap::new(),
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
    /// The gateway's Envoy admin port, on both sides (`address:0.0.0.0:<port>` in the
    /// bootstrap and the `<port>:<port>` host publish). Configurable so two stacks rendered
    /// on the same host don't collide on the admin endpoint. Defaults to `9901`.
    pub gateway_admin_port: u16,
    /// Explicit host ports to hand out, in order, to endpoints that need their own
    /// dedicated listener ([`UiFixed`](RouteIntent::UiFixed) /
    /// [`Gatewayed`](RouteIntent::Gatewayed)). Consumed first; once exhausted the planner
    /// auto-allocates from [`dedicated_listener_port_base`](Self::dedicated_listener_port_base),
    /// so "every object store is exposed" works with no explicit list.
    #[allow(clippy::struct_field_names)]
    pub dedicated_listener_ports: Vec<u16>,
    /// The base host port the planner auto-allocates dedicated listeners from when
    /// [`dedicated_listener_ports`](Self::dedicated_listener_ports) is exhausted: the first
    /// auto listener binds this port, the next `+1`, and so on. Defaults to `9100` (clear of
    /// the gateway's `9080`).
    #[allow(clippy::struct_field_names)]
    pub dedicated_listener_port_base: u16,
    /// Ordered provider preference per resource role — the environment's say in which
    /// implementation satisfies an abstract demand (e.g. `object_store` →
    /// `["azurite", "seaweedfs"]` for a hydrofoil-style env
    /// that prefers Azurite). The planner picks the first preferred provider present in
    /// the catalog; an empty/absent entry falls back to uniqueness then the catalog
    /// default. A demand's own `provider` pin still wins over this.
    pub provider_preference: BTreeMap<String, Vec<ModuleId>>,
    /// The stack's root data directory, injected into every module's render env as
    /// [`DATA_ROOT`](crate::DATA_ROOT_VAR) and resolved at plan time. A module that persists
    /// state mounts it under `${DATA_ROOT}/<module>` by convention, so relocating all
    /// persistence is this single knob (e.g. an absolute path) rather than an edit per
    /// fragment. Defaults to [`DATA_ROOT_DEFAULT`](crate::DATA_ROOT_DEFAULT) (`./.data`,
    /// relative to the compose file).
    pub data_root: String,
}

impl Default for PlanCtx {
    fn default() -> Self {
        PlanCtx {
            env_name: "lakehouse".into(),
            gateway_service: "envoy".into(),
            gateway_internal_port: 10000,
            gateway_host_port: 9080,
            gateway_admin_port: 9901,
            dedicated_listener_ports: Vec::new(),
            dedicated_listener_port_base: 9100,
            provider_preference: BTreeMap::new(),
            data_root: DATA_ROOT_DEFAULT.into(),
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
    /// A module demands a resource kind no catalog module provisions.
    #[error("no module provides resource kind `{resource}` demanded by `{module}`")]
    UnsatisfiedDemand {
        /// The demanding module.
        module: String,
        /// The unmet resource kind.
        resource: String,
    },
    /// More than one catalog module provisions a demanded resource kind and there is
    /// no tie-break, so the planner will not guess which to deploy.
    #[error(
        "resource kind `{resource}` has multiple providers {providers:?}; selection is ambiguous"
    )]
    AmbiguousProvider {
        /// The demanded resource kind.
        resource: String,
        /// The candidate provider module ids (sorted).
        providers: Vec<ModuleId>,
    },
    /// A demand binds a [`ConnectionField`](crate::ConnectionField) the chosen provider's
    /// connection variant does not carry (e.g. binding `url` against an object store, or an
    /// S3 credential field against an Azure-backed store).
    #[error("connection for `{resource}` has no field `{field:?}` to bind (needed by `{module}`)")]
    UnboundConnectionField {
        /// The demanding module.
        module: String,
        /// The resource role.
        resource: String,
        /// The connection field the variant does not carry.
        field: crate::connection::ConnectionField,
    },
    /// A module's [`Template`](crate::RenderSpec::Template) fragment failed to compile or
    /// render — a malformed template or a reference to a field absent from the render
    /// context. Recoverable because a module can be authored as an external on-disk manifest.
    #[error("module `{module}` failed to render: {source}")]
    Render {
        /// The module whose template failed.
        module: String,
        /// The underlying templating error.
        #[source]
        source: crate::module::RenderError,
    },
    /// Two or more providers of the same resource role are in one environment without an
    /// explicit per-demand pin selecting each. The fix is to pin each demand's provider or
    /// drop one provider from the selection.
    #[error(
        "role `{role}` has multiple unpinned providers {providers:?} in one environment; \
         pin each demand's provider or remove one"
    )]
    ConflictingRoleProviders {
        /// The over-subscribed role.
        role: String,
        /// The provider module ids present for the role (sorted).
        providers: Vec<ModuleId>,
    },
    /// Two modules declared the same compose `configs:` alias. Aliases share one
    /// top-level namespace, so the planner refuses to let one shadow the other; rename
    /// one module's [`RenderFile`](crate::RenderFile) alias.
    #[error(
        "compose config alias `{alias}` is declared by both `{first}` and `{second}`; \
         rename one"
    )]
    ConfigAliasCollision {
        /// The colliding alias.
        alias: String,
        /// The module that declared it first.
        first: ModuleId,
        /// The module that collided with it.
        second: ModuleId,
    },
    /// A module declares a [`required`](crate::Knob::required) knob with no
    /// [`default`](crate::Knob::default), and the [`Selection`] supplied no override for
    /// it. The fix is to set the knob in
    /// [`Selection::knob_overrides`](Selection::knob_overrides) (or give the knob a
    /// default in the catalog).
    #[error("module `{module}` requires a value for knob `{key}` (no default, none supplied)")]
    MissingRequiredKnob {
        /// The module that owns the knob.
        module: ModuleId,
        /// The knob's `key`.
        key: String,
    },
    /// A knob value (an override or the declared default) is not a valid member of the
    /// knob's [`KnobKind`](crate::KnobKind) — e.g. a non-boolean for a `Bool`, an
    /// off-list choice for an `Enum`, or an out-of-range/non-numeric `Integer`/`Port`.
    /// Caught at plan time so a malformed knob can never reach a rendered config and
    /// surface only as a container startup failure.
    #[error("knob `{key}` on module `{module}` rejected value `{value}` for kind {kind:?}")]
    InvalidKnobValue {
        /// The module that owns the knob.
        module: ModuleId,
        /// The knob's `key`.
        key: String,
        /// The offending value.
        value: String,
        /// The kind the value failed to satisfy.
        kind: crate::module::KnobKind,
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
    /// The Envoy admin port (bound on both sides). Set from
    /// [`PlanCtx::gateway_admin_port`]; the `Default` of `0` is only ever a transient
    /// accumulator value the planner overwrites.
    pub admin_port: u16,
}

/// A compose `include:` entry contributed by a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposeInclude {
    /// The module that contributed this fragment.
    pub module: ModuleId,
    /// The fragment text (the module's rendered compose `services:` snippet).
    pub fragment: String,
}

/// A top-level compose `configs:` declaration: an alias and the host file it maps to.
///
/// The generated compose emits `configs: <alias>: { file: <path> }`, and a service
/// fragment mounts it by `configs: - source: <alias>`. Each entry comes from a module's
/// [`RenderFile`](crate::RenderFile) that declared an `alias` (the host `path` is the
/// per-module-rooted file path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDecl {
    /// The compose config alias (referenced by `configs: - source: <alias>`).
    pub alias: String,
    /// The host file path the alias maps to (e.g. `modules/envoy/envoy.yaml`).
    pub path: String,
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
    /// The top-level `configs:` declarations (alias → host file), sorted by alias for
    /// deterministic output. Aggregated from every module's aliased
    /// [`RenderFile`](crate::RenderFile)s plus any synthetic entry the planner adds for a
    /// dedicated-renderer artifact (e.g. the gateway's envoy bootstrap).
    pub configs: Vec<ConfigDecl>,
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
    /// The typed [`Connection`] resolved for each demand, keyed by `(demanding module id,
    /// demand index)`. The same handshake the planner flattens into [`injected`](Self::injected)
    /// today, exposed in typed form so a consumer can make explicit per-flavour rendering
    /// decisions (e.g. branch on S3 vs Azure credentials) without re-parsing strings.
    pub connections: BTreeMap<(ModuleId, usize), Connection>,
    /// Each module's rendered output (fragment + files).
    pub renders: Vec<(ModuleId, RenderOutput)>,
    /// The consolidated head/compose plan.
    pub head: HeadFile,
    /// The structured gateway config.
    pub gateway: GatewayConfig,
    /// Postgres databases the selected modules need, deduplicated in dependency order. These
    /// also reach the postgres provider's render as `RenderCtx.objects`, which its init-script
    /// `RenderFile` iterates; this field is the same list exposed for consumers.
    pub postgres_databases: Vec<String>,
    /// Object-store buckets provisioned on SeaweedFS (when it is the chosen object-store
    /// provider), deduplicated in dependency order.
    pub s3_buckets: Vec<String>,
    /// Object-store containers provisioned on Azurite (when it is the chosen object-store
    /// provider), deduplicated in dependency order.
    pub azure_containers: Vec<String>,
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
    // Resolve the selection, auto-provisioning a provider for every resource demand
    // (a demanded relational store, object store, …) before anything else runs.
    let graph = resolve_with_demands(selection, catalog, ctx)?;

    // Resolve each demand's provider and its typed connection exactly once, up front:
    // `choose_provider` is a pure function of `(ctx, catalog, demand)`, so the downstream
    // passes (role exclusivity, provisioning, binding) all read the same chosen provider and
    // resolved connection from here instead of re-running the selection (and its per-call
    // catalog scan) at each site. A connection variant's fields are mandatory, so provider
    // completeness is a compile-time guarantee — no runtime contract check is needed.
    let mut chosen = resolve_demand_providers(&graph, catalog, ctx)?;

    // At most one provider per resource role may end up in an environment — unless a
    // consumer explicitly pinned each (the sanctioned multi-store case).
    check_role_exclusivity(&graph)?;

    // Resources to provision, grouped by the *chosen provider* (deduped, in dependency
    // order). Grouping by provider — not by abstract role — is what lets one object-store
    // demand land on SeaweedFS and another on Azurite, each provisioning on its own init.
    // Walk modules in graph (dependency) order so the grouped names stay deterministic.
    let mut provisioned_by: BTreeMap<ModuleId, Vec<String>> = BTreeMap::new();
    for module in &graph.nodes {
        for (idx, demand) in module.needs.iter().enumerate() {
            let provider = chosen[&(module.id.clone(), idx)].provider.clone();
            let names = provisioned_by.entry(provider).or_default();
            if !names.contains(&demand.name) {
                names.push(demand.name.clone());
            }
        }
    }
    let provisioned_for = |id: &str| -> Vec<String> {
        provisioned_by
            .get(&ModuleId::from(id))
            .cloned()
            .unwrap_or_default()
    };
    // Convenience views for the artifact renderers (single-provider roles today).
    let postgres_databases = provisioned_for("postgres");
    let s3_buckets = provisioned_for("seaweedfs");
    let azure_containers = provisioned_for("azurite");

    // Allocate dedicated listener host ports up front, in graph order, for every endpoint
    // that needs its own listener ([`UiFixed`] / [`Gatewayed`]). Doing this before the main
    // routing loop lets us rewrite a gatewayed object store's connection `endpoint` to the
    // gateway *before* consumers bind it (a provider is ordered before its consumers, but we
    // bind a consumer's connection earlier in its own iteration than we'd reach the
    // provider's route). Host ports come from the explicit `dedicated_listener_ports` first,
    // then auto-allocated from `dedicated_listener_port_base`.
    let mut dedicated_ports: BTreeMap<(String, String), u16> = BTreeMap::new();
    {
        let mut explicit = ctx.dedicated_listener_ports.iter().copied();
        let mut next_auto = ctx.dedicated_listener_port_base;
        let mut alloc = || {
            explicit.next().unwrap_or_else(|| {
                let p = next_auto;
                next_auto = next_auto.saturating_add(1);
                p
            })
        };
        for module in &graph.nodes {
            for service in &module.services {
                for endpoint in &service.endpoints {
                    if matches!(
                        endpoint.intent,
                        RouteIntent::UiFixed | RouteIntent::Gatewayed
                    ) {
                        dedicated_ports
                            .insert((service.name.clone(), endpoint.id.clone()), alloc());
                    }
                }
            }
        }
    }

    // Rewrite each gatewayed object store's resolved `endpoint` (and the `BlobEndpoint=` inside
    // an Azure connection string) to the gateway origin, so consumers reach the store through
    // Envoy on its dedicated listener rather than the provider's in-network address. The
    // provider's own self-connection (used to render its fragment) is rewritten in the same
    // pass via `chosen`.
    rewrite_gatewayed_object_store_endpoints(&mut chosen, &graph, ctx, &dedicated_ports);

    let mut routes = RoutePlan::new();
    let mut gateway = GatewayConfig {
        admin_port: ctx.gateway_admin_port,
        ..GatewayConfig::default()
    };
    let mut injected: BTreeMap<ModuleId, InjectedEnv> = BTreeMap::new();

    let mut shared_routes: Vec<GatewayRoute> = Vec::new();
    let mut dedicated: Vec<ListenerConfig> = Vec::new();

    // Track claimed prefixes (per listener) to detect collisions. The shared listener
    // is keyed by its port; dedicated listeners never collide (own port).
    let mut claimed: BTreeMap<String, String> = BTreeMap::new();

    // Walk modules in dependency order so emitted routes/clusters are deterministic.
    for module in &graph.nodes {
        let mut module_env = InjectedEnv::new();
        // The stack's root data directory, available to every fragment as a render-only value
        // (baked at plan time, like `BASE_PATH`). A module that persists state mounts it under
        // `{{ env.DATA_ROOT }}/<module>` rather than hard-coding a `./.data/...` path, so the
        // whole stack's persistence relocates via the one `PlanCtx::data_root` knob.
        module_env.set(DATA_ROOT_VAR, &ctx.data_root);
        // Seed each module's render env with its declared env vars.
        for (k, v) in module.provides.env_vars.iter() {
            module_env.set(k, v);
        }
        // Inject the module's user-tunable knob values: an override from the selection,
        // else the knob's default. Each lands under the knob's `key`, so a fragment or a
        // mounted config file reads it as `{{ env.KEY }}` — the same injection point as
        // `DATA_ROOT` and `BASE_PATH`.
        let module_overrides = selection.knob_overrides.get(&module.id);
        for knob in &module.knobs {
            if let Some(value) = resolve_knob(&module.id, knob, module_overrides)? {
                module_env.set(&knob.key, value);
            }
        }
        // Bind each demand's resolved connection back into the consuming module's env, by
        // typed field. The connection was resolved once up front (in `chosen`).
        for (idx, demand) in module.needs.iter().enumerate() {
            let connection = &chosen[&(module.id.clone(), idx)].connection;
            bind_connection(&mut module_env, &module.id, demand, connection)?;
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
                    // A fixed-path UI and a gatewayed backend (e.g. an object store) share the
                    // same routing: their own dedicated listener serving `/`, no rewrite, so
                    // the client reaches the service at the listener's origin and constructs
                    // its own URLs unprefixed.
                    RouteIntent::UiFixed | RouteIntent::Gatewayed => {
                        let port = dedicated_ports[&(service.name.clone(), endpoint.id.clone())];
                        ensure_cluster(&mut gateway, service, endpoint)?;
                        // The dedicated listener binds the allocated port on *both* sides:
                        // Envoy listens on it inside the compose network (so an in-network
                        // consumer reaches `envoy:<port>`) and compose publishes it 1:1 to the
                        // host. It forwards to the service-named cluster, which already targets
                        // the upstream's own port (`endpoint.internal_port`) via `ensure_cluster`.
                        dedicated.push(ListenerConfig {
                            host_port: port,
                            internal_port: port,
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

    // Aggregate the stack's env vars for `.env`, last-writer-wins in dependency order:
    // each module's *declared* env vars, plus the coordinates injected to satisfy its
    // demands (a fragment reads those as `${VAR}`, so compose must resolve them from
    // `.env` at run time). Render-only injections (`BASE_PATH`, `DATA_ROOT`) stay out of
    // `.env` — they are rendered into the fragment at plan time.
    let mut env = InjectedEnv::new();
    for module in &graph.nodes {
        for (k, v) in module.provides.env_vars.iter() {
            env.set(k, v);
        }
        // A provider contributes its connections' conventional SDK env vars (an object
        // store's `AWS_*` / `AZURE_STORAGE_CONNECTION_STRING`), derived from the typed
        // credential so it is stated once. Only an in-graph (i.e. chosen) provider reaches
        // here, so an unselected backend's credentials never leak into `.env`. The values
        // are name-independent, so the template's credential is read directly.
        for template in module.provides.resource_kinds.values() {
            for (k, v) in template.0.standard_env() {
                env.set(k, v);
            }
        }
        for (idx, demand) in module.needs.iter().enumerate() {
            let connection = &chosen[&(module.id.clone(), idx)].connection;
            bind_connection(&mut env, &module.id, demand, connection)?;
        }
    }

    // Render every module from its decided env and resolved connections, in dependency
    // order. A module with an empty fragment (e.g. the env-only contract module)
    // contributes no compose include.
    let mut renders = Vec::with_capacity(graph.nodes.len());
    let mut includes = Vec::new();
    // Aggregated `configs:` declarations and the module each alias was first seen on, so a
    // second module reusing an alias is rejected rather than silently shadowing.
    let mut configs: Vec<ConfigDecl> = Vec::new();
    let mut alias_owner: BTreeMap<String, ModuleId> = BTreeMap::new();
    for module in &graph.nodes {
        let module_env = injected.get(&module.id).cloned().unwrap_or_default();
        // The typed connections resolved for this module's demands, grouped by role, so a
        // `Template` fragment can branch on the chosen credential flavour. Alongside them,
        // the resolved `depends_on` gates — one per demand whose chosen provider advertises a
        // startup gate — so a fragment iterates them instead of hard-coding which backend's
        // service it waits on.
        let mut connections: BTreeMap<String, Vec<Connection>> = BTreeMap::new();
        let mut dependencies: Vec<crate::module::DepGate> = Vec::new();
        for (idx, demand) in module.needs.iter().enumerate() {
            let provider = &chosen[&(module.id.clone(), idx)].provider;
            connections
                .entry(demand.resource.clone())
                .or_default()
                .push(chosen[&(module.id.clone(), idx)].connection.clone());
            if let Some(gate) = provider_dep_gate(catalog, provider) {
                dependencies.push(gate);
            }
        }
        // A *provider* renders against its own role too: the names it provisions (`objects`,
        // for an init block to iterate) plus its own connection resolved for each — so its
        // fragment reads e.g. `connections.object_store.0.credential` rather than a `${VAR}`.
        let objects = provisioned_by.get(&module.id).cloned().unwrap_or_default();
        for (role, template) in module.provides.resource_kinds.iter() {
            let role_conns = connections.entry(role.clone()).or_default();
            for name in &objects {
                role_conns.push(template.resolve(name));
            }
        }
        // The gateway module publishes every listener's host port: its fragment iterates
        // `published_ports` to render compose `ports:`, so dedicated listeners (object stores)
        // are reachable from the host without the fragment hard-coding a port list.
        let published_ports = if module.services.iter().any(|s| s.role == Role::gateway()) {
            gateway
                .listeners
                .iter()
                .map(|l| crate::module::PortMapping {
                    host: l.host_port,
                    container: l.internal_port,
                })
                // The Envoy admin endpoint is not a routing listener, so it isn't in
                // `gateway.listeners`; publish it explicitly (1:1) alongside them.
                .chain(std::iter::once(crate::module::PortMapping {
                    host: ctx.gateway_admin_port,
                    container: ctx.gateway_admin_port,
                }))
                .collect()
        } else {
            Vec::new()
        };
        let render_ctx = crate::module::RenderCtx {
            env: &module_env,
            connections,
            dependencies,
            objects,
            published_ports,
        };
        let mut out = module
            .render
            .render(&render_ctx)
            .map_err(|source| PlanError::Render {
                module: module.id.0.clone(),
                source,
            })?;
        // Root each emitted file under the module's own directory (`modules/<id>/<path>`),
        // so a module never hard-codes the global layout and its files sit beside its
        // fragment. The rewritten path is the single source of truth the consumer writes to
        // and the compose `configs: file:` references.
        for f in &mut out.files {
            f.path = format!("modules/{}/{}", module.id.as_str(), f.path);
            if let Some(alias) = &f.alias {
                if let Some(first) = alias_owner.get(alias) {
                    return Err(PlanError::ConfigAliasCollision {
                        alias: alias.clone(),
                        first: first.clone(),
                        second: module.id.clone(),
                    });
                }
                alias_owner.insert(alias.clone(), module.id.clone());
                configs.push(ConfigDecl {
                    alias: alias.clone(),
                    path: f.path.clone(),
                });
            }
        }
        if !out.fragment.trim().is_empty() {
            includes.push(ComposeInclude {
                module: module.id.clone(),
                fragment: out.fragment.clone(),
            });
        }
        renders.push((module.id.clone(), out));
    }

    // Deterministic `configs:` order regardless of module/graph order.
    configs.sort_by(|a, b| a.alias.cmp(&b.alias));

    let head = HeadFile {
        name: ctx.env_name.clone(),
        env: env.clone(),
        includes,
        configs,
    };

    // The typed connections, exposed for downstream consumers (deterministic: built from
    // `chosen`, which is populated walking the graph in dependency order).
    let connections = chosen
        .into_iter()
        .map(|(key, c)| (key, c.connection))
        .collect();

    Ok(EnvironmentPlan {
        graph,
        routes,
        injected,
        connections,
        renders,
        head,
        gateway,
        postgres_databases,
        s3_buckets,
        azure_containers,
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

/// Resolve a selection, growing it until every resource demand is satisfied.
///
/// A module's [`ResourceDemand`] names a resource *kind*; the planner finds the catalog
/// module that provisions it and adds it to the selection if absent. Because a
/// just-added provider may itself `require` or `need` more, this is a fixed point:
/// resolve → scan demands → add missing providers → re-resolve, until a pass adds
/// nothing. The `requires` closure / topo-sort / cycle detection all live in
/// [`resolve`]; this only decides *which* modules to feed it.
fn resolve_with_demands(
    selection: &Selection,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<ResolvedGraph, PlanError> {
    // Each consuming module's demanded providers, recorded as it is discovered, so the
    // final resolve can treat "needs a resource from X" as a dependency edge on X
    // (provider ordered before consumer, like a `requires`).
    let mut demand_edges: BTreeMap<ModuleId, Vec<ModuleId>> = BTreeMap::new();
    let mut selected = resolve_selection(selection, catalog)?;

    // Fixed point: resolve → scan demands → add missing providers → repeat. Bounded by
    // the catalog size (each iteration adds ≥1 module).
    for _ in 0..=catalog.modules().len() {
        let graph = resolve(&selected, catalog.modules())?;
        let mut added = false;
        for module in &graph.nodes {
            for demand in &module.needs {
                let provider = choose_provider(ctx, catalog, demand, Some(&module.id))?;
                let edges = demand_edges.entry(module.id.clone()).or_default();
                if !edges.contains(&provider) {
                    edges.push(provider.clone());
                }
                if !selected.contains(&provider) {
                    selected.push(provider);
                    added = true;
                }
            }
        }
        if !added {
            // Final resolve against an augmented catalog where each consumer also
            // `requires` its demanded providers, so providers start before consumers.
            let augmented = augment_requires(catalog.modules(), &demand_edges);
            return resolve(&selected, &augmented).map_err(Into::into);
        }
    }
    // Unreachable in practice (the loop bound exceeds the max modules addable).
    let augmented = augment_requires(catalog.modules(), &demand_edges);
    resolve(&selected, &augmented).map_err(Into::into)
}

/// Choose the provider module that satisfies a demand, in priority order:
/// 1. the demand's explicit [`provider`](crate::ResourceDemand::provider) pin;
/// 2. the first [`PlanCtx::provider_preference`] entry for the role present in the catalog;
/// 3. the sole provider, if the role has exactly one;
/// 4. the catalog's declared default provider for the role.
///
/// Errors with [`PlanError::UnsatisfiedDemand`] if no module provides the role, or
/// [`PlanError::AmbiguousProvider`] if several do and none of the above selects one.
/// `consumer` (when known) labels an `UnsatisfiedDemand`.
fn choose_provider(
    ctx: &PlanCtx,
    catalog: &Catalog,
    demand: &crate::module::ResourceDemand,
    consumer: Option<&ModuleId>,
) -> Result<ModuleId, PlanError> {
    let role = &demand.resource;
    let candidates = catalog.providers_for(role);
    if candidates.is_empty() {
        return Err(PlanError::UnsatisfiedDemand {
            module: consumer.map(|m| m.0.clone()).unwrap_or_default(),
            resource: role.clone(),
        });
    }
    let provides_role = |id: &ModuleId| candidates.contains(&id);

    // 1. Explicit pin.
    if let Some(pin) = &demand.provider {
        if provides_role(pin) {
            return Ok(pin.clone());
        }
        // A pin that doesn't provide the role is unsatisfiable for this demand.
        return Err(PlanError::UnsatisfiedDemand {
            module: consumer.map(|m| m.0.clone()).unwrap_or_default(),
            resource: role.clone(),
        });
    }
    // 2. Environment preference.
    if let Some(order) = ctx.provider_preference.get(role)
        && let Some(pref) = order.iter().find(|id| provides_role(id))
    {
        return Ok(pref.clone());
    }
    // 3. Unique provider.
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    // 4. Catalog default.
    if let Some(def) = catalog.default_provider_for(role)
        && provides_role(def)
    {
        return Ok(def.clone());
    }
    // Otherwise genuinely ambiguous.
    let mut providers: Vec<ModuleId> = candidates.into_iter().cloned().collect();
    providers.sort();
    Err(PlanError::AmbiguousProvider {
        resource: role.clone(),
        providers,
    })
}

/// Clone the catalog modules, extending each module's `requires` with the providers it
/// demands, so the resolver orders a provider before the module that needs its resource.
fn augment_requires(
    modules: &[Module],
    demand_edges: &BTreeMap<ModuleId, Vec<ModuleId>>,
) -> Vec<Module> {
    modules
        .iter()
        .map(|m| {
            let mut m = m.clone();
            if let Some(providers) = demand_edges.get(&m.id) {
                for p in providers {
                    if !m.requires.contains(p) {
                        m.requires.push(p.clone());
                    }
                }
            }
            m
        })
        .collect()
}

/// Bind a demand's resolved [`Connection`] into `env` by typed field, per the demand's
/// [`ConnectionBinding`](crate::ConnectionBinding).
///
/// Each `(field, key)` pair sets `key` to the connection's value for `field`. A field the
/// connection variant does not carry is a [`PlanError::UnboundConnectionField`].
fn bind_connection(
    env: &mut InjectedEnv,
    consumer: &ModuleId,
    demand: &crate::module::ResourceDemand,
    connection: &Connection,
) -> Result<(), PlanError> {
    for (field, key) in &demand.bind.bind {
        let value = connection
            .field(*field)
            .ok_or_else(|| PlanError::UnboundConnectionField {
                module: consumer.0.clone(),
                resource: demand.resource.clone(),
                field: *field,
            })?;
        env.set(key, value);
    }
    Ok(())
}

/// Resolve a knob to the value the planner should inject, **validated and coerced against
/// its [`KnobKind`]**: an override (from [`Selection::knob_overrides`]) wins, then the
/// knob's declared [`default`](crate::Knob::default); a [`required`](crate::Knob::required)
/// knob with neither is a [`PlanError::MissingRequiredKnob`]. A non-required knob with no
/// value resolves to `None` (nothing injected — the module's template supplies its own).
///
/// The chosen value is checked against the knob's `kind` and canonicalized so what lands in
/// the [`InjectedEnv`] is always a valid literal for the template's target format (e.g. a
/// `Bool` renders as the bare TOML/JSON `true`/`false`, never `"True"` or `"yes"`). A value
/// the kind cannot accept is a plan-time [`PlanError::InvalidKnobValue`] rather than a
/// malformed config that fails only when the container starts.
fn resolve_knob(
    module: &ModuleId,
    knob: &crate::module::Knob,
    overrides: Option<&BTreeMap<String, String>>,
) -> Result<Option<String>, PlanError> {
    let raw = overrides
        .and_then(|o| o.get(&knob.key))
        .map(String::as_str)
        .or(knob.default.as_deref());
    let Some(raw) = raw else {
        if knob.required {
            return Err(PlanError::MissingRequiredKnob {
                module: module.clone(),
                key: knob.key.clone(),
            });
        }
        return Ok(None);
    };
    coerce_knob(raw, &knob.kind)
        .map(Some)
        .ok_or_else(|| PlanError::InvalidKnobValue {
            module: module.clone(),
            key: knob.key.clone(),
            value: raw.to_string(),
            kind: knob.kind.clone(),
        })
}

/// Validate `raw` against `kind` and return its canonical injected form, or `None` if the
/// value is not a valid member of the kind (the caller turns that into a
/// [`PlanError::InvalidKnobValue`]).
///
/// Coercions are deliberately liberal on input, strict on output: a `Bool` accepts the usual
/// truthy/falsey spellings but always emits the bare `true`/`false`; an `Integer`/`Port`
/// parses and range-checks but re-emits the canonical decimal; a `String` passes through; an
/// `Enum` must match one of its options exactly.
fn coerce_knob(raw: &str, kind: &crate::module::KnobKind) -> Option<String> {
    use crate::module::KnobKind;
    match kind {
        KnobKind::String => Some(raw.to_string()),
        KnobKind::Bool => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some("true".to_string()),
            "false" | "0" | "no" | "off" => Some("false".to_string()),
            _ => None,
        },
        KnobKind::Enum { options } => options.iter().find(|o| *o == raw).cloned(),
        KnobKind::Integer { min, max } => {
            let n: i64 = raw.trim().parse().ok()?;
            if min.is_some_and(|lo| n < lo) || max.is_some_and(|hi| n > hi) {
                return None;
            }
            Some(n.to_string())
        }
        KnobKind::Port => {
            let n: u16 = raw.trim().parse().ok()?;
            (n != 0).then(|| n.to_string())
        }
    }
}

/// The provider chosen for a demand and the typed connection it resolves to.
#[derive(Clone)]
struct ResolvedDemand {
    /// The provider module chosen to satisfy the demand.
    provider: ModuleId,
    /// The provider's connection template, resolved for the demanded resource name.
    connection: Connection,
}

/// The resolved provider + connection for each demand, keyed by `(demanding module id,
/// demand index)`.
///
/// Computed once by [`resolve_demand_providers`] and shared by the planner's passes so each
/// demand's provider is selected — and its connection resolved — a single time (rather than
/// re-running [`choose_provider`] and re-substituting `{name}` at every consumer).
type ChosenProviders = BTreeMap<(ModuleId, usize), ResolvedDemand>;

/// Resolve the provider and typed connection for every demand in the graph, once.
///
/// A connection variant's fields are all mandatory, so a chosen provider cannot render an
/// incomplete connection — completeness is a compile-time guarantee, replacing the old
/// runtime role-contract validation.
fn resolve_demand_providers(
    graph: &ResolvedGraph,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<ChosenProviders, PlanError> {
    let mut chosen = ChosenProviders::new();
    for module in &graph.nodes {
        for (idx, demand) in module.needs.iter().enumerate() {
            let provider_id = choose_provider(ctx, catalog, demand, Some(&module.id))?;
            let provider = catalog
                .get(&provider_id)
                .expect("provider id is in catalog");
            let template = provider
                .provides
                .resource_kinds
                .get(&demand.resource)
                .expect("chosen provider provisions the demanded role");
            let connection = template.resolve(&demand.name);
            chosen.insert(
                (module.id.clone(), idx),
                ResolvedDemand {
                    provider: provider_id,
                    connection,
                },
            );
        }
    }
    Ok(chosen)
}

/// Reject an environment that contains two or more providers of the same resource role,
/// unless every extra provider was reached by an explicit per-demand
/// [`provider`](crate::ResourceDemand::provider) pin.
///
/// `choose_provider` already selects exactly one provider per demand, so a same-role clash
/// only arises when a *second* provider is in the graph for an unrelated reason — directly
/// selected, or pulled in by `requires`. That is the case this rejects (with
/// [`PlanError::ConflictingRoleProviders`]): selecting both SeaweedFS and Azurite with no
/// pins is ambiguous, and the planner will not silently drop one. The exception is the
/// deliberate multi-store shape — a consumer that pins each demand's provider — where every
/// provider beyond the first is accounted for by a pin.
fn check_role_exclusivity(graph: &ResolvedGraph) -> Result<(), PlanError> {
    // The (role, provider) pairs a demand explicitly pinned — these are sanctioned.
    let mut pinned: BTreeSet<(String, ModuleId)> = BTreeSet::new();
    for module in &graph.nodes {
        for demand in &module.needs {
            if let Some(p) = &demand.provider {
                pinned.insert((demand.resource.clone(), p.clone()));
            }
        }
    }

    // Providers of each role present in the resolved graph (a node provides a role if it
    // declares it under `resource_kinds`, or its `provider_of` capability names it).
    let mut by_role: BTreeMap<String, Vec<ModuleId>> = BTreeMap::new();
    for module in &graph.nodes {
        let mut roles: BTreeSet<&str> = module
            .provides
            .resource_kinds
            .keys()
            .map(String::as_str)
            .collect();
        if let Some(cap) = &module.provider_of {
            roles.insert(cap.as_str());
        }
        for role in roles {
            by_role
                .entry(role.to_string())
                .or_default()
                .push(module.id.clone());
        }
    }

    for (role, mut providers) in by_role {
        if providers.len() < 2 {
            continue;
        }
        // Allow it only if at most one provider is unpinned: every other was a deliberate pin.
        let unpinned = providers
            .iter()
            .filter(|id| !pinned.contains(&(role.clone(), (*id).clone())))
            .count();
        if unpinned > 1 {
            // Ignore a role that no demand actually targets (e.g. a `provider_of` capability
            // with no matching `resource_kinds` demand): it is not a provisioning clash.
            let demanded = graph
                .nodes
                .iter()
                .flat_map(|m| &m.needs)
                .any(|d| d.resource == role);
            if !demanded {
                continue;
            }
            providers.sort();
            return Err(PlanError::ConflictingRoleProviders { role, providers });
        }
    }
    Ok(())
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

/// Rewrite every gatewayed object store's resolved connection so its `endpoint` (and the
/// `BlobEndpoint=` inside an Azure connection string) points at the gateway's dedicated
/// listener instead of the provider's in-network address.
///
/// An object-store provider advertises a [`Gatewayed`](RouteIntent::Gatewayed) endpoint;
/// `dedicated_ports` carries the listener host port assigned to it. For each such provider we
/// derive the in-network gateway origin `http://<gateway_service>:<port>` and swap it in for
/// the original origin in every `chosen` connection vended by that provider — keeping any path
/// the original endpoint carried (e.g. Azure's `/devstoreaccount1`). This is what makes
/// consumers (and the provider's own fragment) reach the store through Envoy.
fn rewrite_gatewayed_object_store_endpoints(
    chosen: &mut ChosenProviders,
    graph: &ResolvedGraph,
    ctx: &PlanCtx,
    dedicated_ports: &BTreeMap<(String, String), u16>,
) {
    // provider module id → the in-network gateway origin its store is now reached at.
    let mut provider_origin: BTreeMap<ModuleId, String> = BTreeMap::new();
    for module in &graph.nodes {
        for service in &module.services {
            for endpoint in &service.endpoints {
                if endpoint.intent == RouteIntent::Gatewayed
                    && let Some(port) =
                        dedicated_ports.get(&(service.name.clone(), endpoint.id.clone()))
                {
                    provider_origin.insert(
                        module.id.clone(),
                        format!("http://{}:{}", ctx.gateway_service, port),
                    );
                }
            }
        }
    }
    if provider_origin.is_empty() {
        return;
    }
    for resolved in chosen.values_mut() {
        if let Some(new_origin) = provider_origin.get(&resolved.provider) {
            rewrite_object_store_origin(&mut resolved.connection, new_origin);
        }
    }
}

/// Swap the origin (`scheme://host:port`) of an object-store connection's `endpoint` — and the
/// `BlobEndpoint=` segment of an Azure connection string — to `new_origin`, preserving any
/// path. A no-op for non-object-store connections.
fn rewrite_object_store_origin(connection: &mut Connection, new_origin: &str) {
    let Connection::ObjectStore {
        endpoint,
        credential,
        ..
    } = connection
    else {
        return;
    };
    *endpoint = swap_origin(endpoint, new_origin);
    if let crate::connection::ObjectStoreCredential::AzureBlob { connection_string } = credential {
        // The connection string embeds `BlobEndpoint=<url>;`; rewrite that URL's origin too.
        *connection_string = connection_string
            .split(';')
            .map(|seg| match seg.strip_prefix("BlobEndpoint=") {
                Some(url) => format!("BlobEndpoint={}", swap_origin(url, new_origin)),
                None => seg.to_string(),
            })
            .collect::<Vec<_>>()
            .join(";");
    }
}

/// Replace the `scheme://host[:port]` origin of `url` with `new_origin`, keeping the path.
/// `new_origin` is itself an origin (no trailing path). If `url` has no `://`, it is returned
/// unchanged.
fn swap_origin(url: &str, new_origin: &str) -> String {
    match url.split_once("://") {
        Some((_scheme, rest)) => {
            // The path is everything from the first '/' after the authority.
            match rest.find('/') {
                Some(slash) => format!("{}{}", new_origin, &rest[slash..]),
                None => new_origin.to_string(),
            }
        }
        None => url.to_string(),
    }
}

/// The typed `depends_on` gate a chosen `provider` advertises for its consumers, or `None`
/// if it declares no startup gate (nothing to wait for).
///
/// The provider declares the gate as a single `"<service>:<condition>"`
/// [`DEP_GATE_EXTRA`] value (e.g. `"db:service_healthy"`,
/// `"seaweedfs-init:service_completed_successfully"`); this parses it into a typed
/// [`DepGate`](crate::module::DepGate) the planner hands the consumer's render. An
/// unrecognized or missing condition defaults to
/// [`ServiceStarted`](crate::DependsCondition::ServiceStarted), the weakest compose gate.
fn provider_dep_gate(catalog: &Catalog, provider: &ModuleId) -> Option<crate::module::DepGate> {
    let gate = catalog.get(provider)?.provides.extras.get(DEP_GATE_EXTRA)?;
    let (service, condition) = match gate.split_once(':') {
        Some((s, c)) => (s, crate::module::DependsCondition::parse(c)),
        None => (gate.as_str(), None),
    };
    Some(crate::module::DepGate {
        service: service.to_string(),
        condition: condition.unwrap_or(crate::module::DependsCondition::ServiceStarted),
    })
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
        Selection::modules(["envoy", "postgres", "seaweedfs", "mlflow", "unity-catalog"])
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
            needs: vec![],
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
    fn duplicate_config_alias_across_modules_is_rejected() {
        use crate::module::RenderSpec;
        use crate::render::RenderFile;

        // Two modules each declare a `RenderFile` under the same alias. Aliases share one
        // top-level `configs:` namespace, so the planner must refuse rather than let one
        // silently shadow the other.
        let with_alias = |id: &str| -> Module {
            let mut m = module_with(id, vec![], Default::default());
            m.render = RenderSpec::Template {
                fragment: format!("services:\n  {id}: {{}}\n"),
                files: vec![RenderFile {
                    path: "conf.yaml".into(),
                    contents: "k: v\n".into(),
                    alias: Some("shared_alias".into()),
                }],
            };
            m
        };
        let err = plan(
            &Selection::modules(["a", "b"]),
            &Catalog::from_modules([with_alias("a"), with_alias("b")]),
            &PlanCtx::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            PlanError::ConfigAliasCollision {
                alias: "shared_alias".into(),
                first: ModuleId::from("a"),
                second: ModuleId::from("b"),
            }
        );
    }

    #[test]
    fn config_alias_is_rooted_and_declared_in_the_head() {
        use crate::module::RenderSpec;
        use crate::render::RenderFile;

        // A single aliased file is rooted under its module dir and surfaces as one `configs:`
        // declaration on the head.
        let mut m = module_with("svc", vec![], Default::default());
        m.render = RenderSpec::Template {
            fragment: "services:\n  svc: {}\n".into(),
            files: vec![RenderFile {
                path: "app.toml".into(),
                contents: "x = 1\n".into(),
                alias: Some("svc_config".into()),
            }],
        };
        let p = plan(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([m]),
            &PlanCtx::default(),
        )
        .expect("plan succeeds");
        assert_eq!(
            p.head.configs,
            vec![ConfigDecl {
                alias: "svc_config".into(),
                path: "modules/svc/app.toml".into(),
            }]
        );
        // The render output's file path is rooted to match.
        let (_, out) = p
            .renders
            .iter()
            .find(|(id, _)| id.as_str() == "svc")
            .unwrap();
        assert_eq!(out.files[0].path, "modules/svc/app.toml");
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
    fn depends_on_gates_follow_the_chosen_provider() {
        // MLflow demands relational_db + object_store; the planner resolves each demand's
        // chosen provider's declared gate into the typed `dependencies` its fragment iterates.
        // Asserting on the rendered fragment is the real contract.

        // Default (SeaweedFS): db healthy + seaweedfs-init completed.
        let s3 = plan(
            &Selection::modules(["mlflow"]),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .unwrap();
        let (_, mlflow) = s3
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("mlflow"))
            .unwrap();
        assert!(
            mlflow
                .fragment
                .contains("db:\n        condition: service_healthy")
        );
        assert!(
            mlflow
                .fragment
                .contains("seaweedfs-init:\n        condition: service_completed_successfully")
        );
        assert!(!mlflow.fragment.contains("azurite-init"));

        // Azurite-preferred: the object-store gate switches to azurite-init.
        let mut preference = BTreeMap::new();
        preference.insert(
            "object_store".to_string(),
            vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
        );
        let az = plan(
            &Selection::modules(["mlflow"]),
            &baseline_catalog(),
            &PlanCtx {
                provider_preference: preference,
                ..Default::default()
            },
        )
        .unwrap();
        let (_, mlflow_az) = az
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("mlflow"))
            .unwrap();
        assert!(
            mlflow_az
                .fragment
                .contains("azurite-init:\n        condition: service_completed_successfully")
        );
        assert!(!mlflow_az.fragment.contains("seaweedfs-init"));
    }

    #[test]
    fn ui_base_path_is_injected_for_render() {
        let p = plan(
            &default_selection(),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .unwrap();
        let env = p.injected.get(&"mlflow".into()).unwrap();
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
            ..Default::default()
        };
        let p = plan(&sel, &baseline_catalog(), &PlanCtx::default()).unwrap();
        // mlflow + its transitive requires (postgres, seaweedfs, envoy).
        for id in ["mlflow", "postgres", "seaweedfs", "envoy"] {
            assert!(p.graph.module(&id.into()).is_some(), "missing {id}");
        }
    }

    #[test]
    fn unknown_capability_errors() {
        let sel = Selection {
            modules: vec![],
            capabilities: vec!["telepathy".into()],
            ..Default::default()
        };
        let err = plan(&sel, &baseline_catalog(), &PlanCtx::default()).unwrap_err();
        assert_eq!(err, PlanError::UnknownCapability("telepathy".into()));
    }

    #[test]
    fn plan_is_deterministic_regardless_of_selection_order() {
        let cat = baseline_catalog();
        let a = plan(&default_selection(), &cat, &PlanCtx::default()).unwrap();
        let reversed =
            Selection::modules(["unity-catalog", "mlflow", "seaweedfs", "postgres", "envoy"]);
        let b = plan(&reversed, &cat, &PlanCtx::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_knob_prefers_override_then_default() {
        use crate::module::{Knob, KnobKind};

        let module = ModuleId::from("m");
        let knob = Knob {
            key: "SERVE_UI".into(),
            title: None,
            kind: KnobKind::Bool,
            default: Some("true".into()),
            required: false,
            help: None,
        };

        // No override → the declared default.
        assert_eq!(
            resolve_knob(&module, &knob, None).unwrap().as_deref(),
            Some("true")
        );

        // An override wins over the default.
        let overrides = BTreeMap::from([("SERVE_UI".to_string(), "false".to_string())]);
        assert_eq!(
            resolve_knob(&module, &knob, Some(&overrides))
                .unwrap()
                .as_deref(),
            Some("false")
        );
    }

    #[test]
    fn resolve_knob_coerces_to_canonical_form_per_kind() {
        use crate::module::{Knob, KnobKind};

        let module = ModuleId::from("m");
        let knob = |kind: KnobKind, default: &str| Knob {
            key: "K".into(),
            title: None,
            kind,
            default: Some(default.into()),
            required: false,
            help: None,
        };
        let resolved = |k: &Knob| resolve_knob(&module, k, None).unwrap().unwrap();

        // Bool: liberal input ("True", "yes", "1", "on", whitespace) → bare true/false.
        for truthy in ["true", "True", "YES", " 1 ", "on"] {
            assert_eq!(resolved(&knob(KnobKind::Bool, truthy)), "true", "{truthy}");
        }
        for falsey in ["false", "No", "0", "OFF"] {
            assert_eq!(resolved(&knob(KnobKind::Bool, falsey)), "false", "{falsey}");
        }

        // Integer/Port: parsed and re-emitted canonically (trimmed).
        assert_eq!(
            resolved(&knob(
                KnobKind::Integer {
                    min: Some(1),
                    max: Some(10)
                },
                " 7 "
            )),
            "7"
        );
        assert_eq!(resolved(&knob(KnobKind::Port, "8091")), "8091");

        // Enum: an exact member passes through.
        assert_eq!(
            resolved(&knob(
                KnobKind::Enum {
                    options: vec!["a".into(), "b".into()]
                },
                "b"
            )),
            "b"
        );
    }

    #[test]
    fn resolve_knob_rejects_values_outside_their_kind() {
        use crate::module::{Knob, KnobKind};

        let module = ModuleId::from("m");
        let reject = |kind: KnobKind, value: &str| {
            let k = Knob {
                key: "K".into(),
                title: None,
                kind: kind.clone(),
                default: Some(value.into()),
                required: false,
                help: None,
            };
            assert_eq!(
                resolve_knob(&module, &k, None).unwrap_err(),
                PlanError::InvalidKnobValue {
                    module: module.clone(),
                    key: "K".into(),
                    value: value.into(),
                    kind,
                },
                "expected {value:?} to be rejected"
            );
        };

        reject(KnobKind::Bool, "maybe");
        reject(KnobKind::Bool, "");
        reject(KnobKind::Port, "0"); // 0 is not a usable TCP port
        reject(KnobKind::Port, "99999"); // out of u16 range
        reject(
            KnobKind::Integer {
                min: Some(1),
                max: Some(10),
            },
            "11",
        );
        reject(
            KnobKind::Enum {
                options: vec!["a".into()],
            },
            "z",
        );
    }

    #[test]
    fn resolve_knob_errors_only_when_required_and_unset() {
        use crate::module::{Knob, KnobKind};

        let module = ModuleId::from("m");
        let base = Knob {
            key: "TOKEN".into(),
            title: None,
            kind: KnobKind::String,
            default: None,
            required: false,
            help: None,
        };

        // No value + not required → nothing injected (the template owns its own fallback).
        assert_eq!(resolve_knob(&module, &base, None).unwrap(), None);

        // No value + required → a loud error naming the module and key.
        let required = Knob {
            required: true,
            ..base
        };
        assert_eq!(
            resolve_knob(&module, &required, None).unwrap_err(),
            PlanError::MissingRequiredKnob {
                module,
                key: "TOKEN".into(),
            }
        );
    }
}
