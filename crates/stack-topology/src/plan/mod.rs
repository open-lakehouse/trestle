//! The **planner** ([`Catalog::plan`](crate::Catalog::plan)): a module selection → a
//! fully-assigned environment.
//!
//! This is the producer the rest of the crate was built to feed. A module declares
//! only intent ([`RouteIntent`] on its endpoints) and ingredients
//! ([`Provides`](crate::Provides)); the planner is the one vantage that sees *every* selected
//! module at once, so it is the only place gateway prefixes can be assigned without
//! colliding. Given a
//! [`Selection`] and a [`Catalog`], it:
//!
//! 1. resolves the dependency graph ([`resolve`](fn@resolve)) — transitive `requires`,
//!    conflicts, topological order;
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

pub mod resolve;
pub mod routing;

use crate::address::{AddressError, ServiceRef};
use crate::catalog::Catalog;
use crate::catalog::baseline::{DATA_ROOT_DEFAULT, DATA_ROOT_VAR, DEP_GATE_EXTRA};
use crate::catalog::module::{ModuleId, ResolvedKnobs};
use crate::model::connection::Connection;
use crate::model::endpoint::{Endpoint, Rewrite, RouteIntent};
use crate::model::placement::Placement;
use crate::model::role::{Role, ServiceSpec};
use crate::plan::resolve::{ExtraEdges, ResolveError, ResolvedGraph, resolve, resolve_with};
use crate::plan::routing::{AssignedRoute, Listener, RoutePlan};
use crate::render::{InjectedEnv, RenderOutput};

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
    /// This is the channel a config UI feeds: it surfaces a module's knobs from the
    /// catalog, lets the user tune them, and hands the chosen values back here.
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

/// The user's own application, which the gateway fronts with a `/` catch-all so any
/// request not matched by a module route reaches the app. Supplied via
/// [`PlanCtx::app`]; the planner emits its cluster and the trailing catch-all route into
/// the [`GatewayConfig`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppUpstream {
    /// The app's compose service / DNS name (the gateway cluster's upstream host).
    pub service: String,
    /// The app's port.
    pub port: u16,
}

/// Runtime facts the planner needs but cannot derive from the model — the gateway's
/// service name and ports, the project name, port allocations, provider preferences, the
/// data root, and the optional app upstream. The consumer supplies these once per
/// environment; the plan captures the gateway facts for the address resolver.
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
    /// `["azurite", "seaweedfs"]` for an env that prefers Azurite). The planner picks the first
    /// preferred provider present in the catalog; an empty/absent entry falls back to
    /// uniqueness then the catalog default. A demand's own `provider` pin still wins over this.
    pub provider_preference: BTreeMap<String, Vec<ModuleId>>,
    /// The stack's root data directory, injected into every module's render env as
    /// [`DATA_ROOT`](crate::DATA_ROOT_VAR) and resolved at plan time. A module that persists
    /// state mounts it under `${DATA_ROOT}/<module>` by convention, so relocating all
    /// persistence is this single knob (e.g. an absolute path) rather than an edit per
    /// fragment. Defaults to [`DATA_ROOT_DEFAULT`] (`./.data`,
    /// relative to the compose file).
    pub data_root: String,
    /// The user's app, if any: when set, the gateway gets an `app` cluster and a `/`
    /// catch-all route (emitted last, after every module route) forwarding unmatched
    /// requests to it. Defaults to `None`. Part of the plan because the catch-all is a
    /// structural fact about the gateway, not a render-time decoration.
    pub app: Option<AppUpstream>,
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
            app: None,
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
        field: crate::model::connection::ConnectionField,
    },
    /// A module's [`RenderSpec`](crate::RenderSpec) fragment failed to compile or
    /// render — a malformed template or a reference to a field absent from the render
    /// context. Recoverable because a module can be authored as an external on-disk manifest.
    #[error("module `{module}` failed to render: {source}")]
    Render {
        /// The module whose template failed.
        module: String,
        /// The underlying templating error.
        #[source]
        source: crate::catalog::module::RenderError,
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
        kind: crate::catalog::module::KnobKind,
    },
    /// The gateway's `ENVOY_AUTH` knob is on and an `auth`-role provider was selected, but that
    /// provider does not declare what the gateway needs to wire `ext_authz`: a service with an
    /// [`Internal`](RouteIntent::Internal) endpoint (the upstream the filter calls) and the
    /// [`EXT_AUTHZ_PATH_EXTRA`] extra (the endpoint path). The fix is on the auth provider
    /// module, not the selection.
    #[error(
        "auth provider `{module}` is misconfigured for forward-auth: {reason} \
         (needs an Internal endpoint and the `{}` extra)",
        EXT_AUTHZ_PATH_EXTRA
    )]
    AuthProviderMisconfigured {
        /// The auth-role provider module.
        module: ModuleId,
        /// What specifically is missing.
        reason: String,
    },
    /// The app upstream ([`PlanCtx::app`]) collides with a selected module: either a module
    /// already claimed the `/` route the app catch-all needs, or a module contributes a service
    /// named `app` (the cluster name the app upstream reserves). The fix is to drop the app
    /// upstream or rename/re-prefix the conflicting module.
    #[error("app upstream collides with module `{conflict}`: {reason}")]
    AppUpstreamCollision {
        /// The `service.endpoint` (for a route collision) or service name (for a cluster-name
        /// collision) that conflicts with the app upstream.
        conflict: String,
        /// What specifically collides (the `/` route or the `app` cluster name).
        reason: String,
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

/// The gateway knob key that turns on forward-auth. A bool knob with this key, declared on
/// the module filling the `gateway` role and resolving truthy, makes the planner pull in the
/// `auth`-role provider and gate the shared listener (see [`AuthConfig`]). The contract lives
/// here (the planner owns the wiring); the baseline catalog's `envoy` module declares the knob
/// under this same key.
pub const ENVOY_AUTH_KNOB: &str = "ENVOY_AUTH";

/// The [`Provides::extras`](crate::Provides::extras) key by which the `auth`-role provider declares the HTTP `ext_authz`
/// endpoint path prefix the gateway posts authorization checks to. Provider-specific (Authelia
/// uses `/api/authz/ext-authz/`, other forward-auth proxies differ), so it lives in catalog data
/// rather than as a planner constant — keeping implementation names out of the planner.
pub const EXT_AUTHZ_PATH_EXTRA: &str = "ext_authz_path";

/// The trusted identity headers the gateway strips on ingress and re-adds from the auth
/// response — so the upstream always reads an auth-provider-asserted value, never a forged one.
/// These are the forward-auth convention (shared across providers, not implementation-specific),
/// so the planner supplies them as the default for any `auth`-role provider.
const AUTH_IDENTITY_HEADERS: [&str; 4] = [
    "Remote-User",
    "Remote-Email",
    "Remote-Name",
    "Remote-Groups",
];

/// Whether the gateway's forward-auth knob ([`ENVOY_AUTH_KNOB`]) resolves on for this
/// selection. Reads the bool knob off whichever catalog module fills the `gateway` role,
/// honouring a [`Selection::knob_overrides`] value over the knob's default. Returns `Ok(false)`
/// when there is no gateway module or it declares no such knob.
fn gateway_auth_enabled(selection: &Selection, catalog: &Catalog) -> Result<bool, PlanError> {
    for gateway_id in catalog.providers_of(Role::GATEWAY) {
        let Some(module) = catalog.get(gateway_id) else {
            continue;
        };
        let Some(knob) = module.knobs().iter().find(|k| k.key == ENVOY_AUTH_KNOB) else {
            continue;
        };
        let overrides = selection.knob_overrides.get(gateway_id);
        if let Some(value) = resolve_knob(gateway_id, knob, overrides)? {
            return Ok(value == "true");
        }
    }
    Ok(false)
}

/// Forward-auth (single-sign-on) configuration for the gateway: present only when the
/// gateway's `ENVOY_AUTH` knob is on. It tells the Envoy renderer to gate the shared
/// listener (API + UI routes) behind an `ext_authz` HTTP filter pointed at the auth
/// provider, and which identity headers to treat as trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthConfig {
    /// The upstream cluster the `ext_authz` filter calls (an entry in
    /// [`GatewayConfig::clusters`]). The bundled provider is `authelia`.
    pub cluster: String,
    /// The auth provider's port (the `server_uri` authority for the `ext_authz` filter).
    pub port: u16,
    /// The auth endpoint path prefix the filter posts to (Authelia's
    /// `/api/authz/ext-authz/`).
    pub path_prefix: String,
    /// The client-facing path prefix the auth provider's **login portal** is served at, routed
    /// through the gateway to the provider's cluster with `ext_authz` disabled on that route (so
    /// a logged-out user can actually reach the login page the deny-redirect sends them to). The
    /// provider's `authelia_url` / session config must point here. Defaults to `/authelia`.
    pub portal_prefix: String,
    /// The trusted identity headers the gateway both **strips on ingress** (so a client
    /// cannot forge them) and **allows upstream** from the auth response (so the app reads
    /// an auth-provider-asserted value). E.g. `Remote-User` / `Remote-Email` / `Remote-Name`
    /// / `Remote-Groups`.
    pub identity_headers: Vec<String>,
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
    /// Forward-auth configuration, present only when the gateway's `ENVOY_AUTH` knob is on.
    /// When set, the Envoy renderer gates the shared listener behind `ext_authz`.
    pub auth: Option<AuthConfig>,
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

/// The gateway facts the address resolver needs but cannot derive from the model: the
/// gateway's compose service name and its internal / host-published ports. Captured from
/// [`PlanCtx`] at plan time and held on the [`Plan`], so addressing a service
/// ([`Plan::service`] → [`ServiceRef::address`]) needs no separately-constructed context.
#[derive(Clone, Debug)]
pub(crate) struct GatewayFacts {
    /// The gateway's compose service / DNS name (e.g. `"envoy"`).
    pub service: String,
    /// The gateway's listening port inside the compose network (e.g. `10000`).
    pub internal_port: u16,
    /// The gateway's host-published port (e.g. `9080`).
    pub host_port: u16,
}

/// A fully-assigned environment: the resolved graph, the routing plan, the rendered
/// modules, and the consolidated head + gateway config. Everything is data; the
/// consumer does the I/O (serialize, write, mount, launch).
///
/// Build one with [`Catalog::plan`](crate::Catalog::plan). Render its artifacts with
/// [`render_all`](crate::render_all), and resolve a service's address from a given
/// [`Vantage`](crate::Vantage) with [`Plan::service`] / [`Plan::service_by_role`] →
/// [`ServiceRef::address`].
///
/// Not `PartialEq`/`Eq`: it embeds the [`ResolvedGraph`], whose modules are trait objects.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The resolved, ordered module graph.
    pub graph: ResolvedGraph,
    /// The per-endpoint route assignments the [`address`](crate::address) resolver
    /// consumes.
    pub routes: RoutePlan,
    /// Each module's injected env (the values the planner decided for it).
    pub injected: BTreeMap<ModuleId, InjectedEnv>,
    /// Each module's services, resolved against its knobs (the same values the planner routed
    /// over). Exposed so consumers — and the artifact renderers — read the settled services
    /// instead of recomputing `module.services(...)`.
    pub services: BTreeMap<ModuleId, Vec<ServiceSpec>>,
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
    /// The gateway facts the address resolver reads (captured from [`PlanCtx`]). Private:
    /// callers reach addressing through [`Plan::service`] → [`ServiceRef::address`], never
    /// by reconstructing this.
    pub(crate) gateway_facts: GatewayFacts,
}

impl Plan {
    /// Flatten this plan's full rendered output — the stack artifacts plus every module
    /// fragment and config file — into one write-ready
    /// [`MaterializedOutput`](crate::MaterializedOutput) in the project's on-disk layout. Pure
    /// (no I/O); with the `std-io` feature,
    /// [`MaterializedOutput::write_to`](crate::MaterializedOutput::write_to) writes it to disk.
    pub fn materialize(&self) -> crate::render::output::MaterializedOutput {
        crate::render::output::materialize(self)
    }

    /// A handle to address the module `module`'s first service. Most modules contribute a
    /// single service; use [`service_named`](Self::service_named) to pick among several.
    pub fn service(&self, module: &ModuleId) -> Option<ServiceRef<'_>> {
        let spec = self.services.get(module)?.first()?;
        Some(ServiceRef::new(spec, &self.routes, &self.gateway_facts))
    }

    /// A handle to address the module `module`'s service named `name`.
    pub fn service_named(&self, module: &ModuleId, name: &str) -> Option<ServiceRef<'_>> {
        let spec = self.services.get(module)?.iter().find(|s| s.name == name)?;
        Some(ServiceRef::new(spec, &self.routes, &self.gateway_facts))
    }

    /// A handle to address the single service filling `role`.
    ///
    /// A caller usually knows a role ("the catalog", "the object store") rather than a module
    /// id. Errors with [`AddressError::NoSuchRole`] if nothing fills it, or
    /// [`AddressError::AmbiguousRole`] if more than one service does (address one by id then).
    pub fn service_by_role(&self, role: &Role) -> Result<ServiceRef<'_>, AddressError> {
        let matches: Vec<&ServiceSpec> = self
            .services
            .values()
            .flatten()
            .filter(|s| &s.role == role)
            .collect();
        match matches.as_slice() {
            [] => Err(AddressError::NoSuchRole(role.as_str().to_string())),
            [only] => Ok(ServiceRef::new(only, &self.routes, &self.gateway_facts)),
            many => {
                let mut services: Vec<String> = many.iter().map(|s| s.name.clone()).collect();
                services.sort();
                Err(AddressError::AmbiguousRole {
                    role: role.as_str().to_string(),
                    services,
                })
            }
        }
    }

    /// The gateway's host-published port — what a host-side caller reaches the shared listener
    /// at (e.g. for printing `http://localhost:<port>`). The full URL to a specific service is
    /// [`service`](Self::service) → [`ServiceRef::address`].
    pub fn gateway_host_port(&self) -> u16 {
        self.gateway_facts.host_port
    }
}

impl Catalog {
    /// Resolve `selection` against this catalog under `ctx` into a fully-materialized [`Plan`].
    ///
    /// This is the producer the rest of the crate feeds: it resolves the dependency graph,
    /// assigns non-colliding gateway routes, resolves knobs and connections, and renders every
    /// module. See the [module docs](crate::plan) for the prefix-derivation rules. Returns a
    /// [`Plan`], or a [`PlanError`] (notably [`PlanError::PrefixCollision`]).
    pub fn plan(&self, selection: &Selection, ctx: &PlanCtx) -> Result<Plan, PlanError> {
        plan_env(selection, self, ctx)
    }
}

/// The planner implementation behind [`Catalog::plan`]. Kept as a free function (rather than
/// inlined into the method) because it is long and recursion-free over `catalog`/`selection`.
fn plan_env(selection: &Selection, catalog: &Catalog, ctx: &PlanCtx) -> Result<Plan, PlanError> {
    // Forward-auth is a gateway knob (`ENVOY_AUTH`), not a separately-selected module: when
    // it resolves on, pull the auth provider into the selection so it joins the graph like any
    // other module (and the routing pass below wires the `ext_authz` filter + cluster). The
    // knob lives on whichever module fills the `gateway` role, and the provider is whichever
    // fills the `auth` role — both looked up by role, so a differently-named gateway/provider
    // still works. Done here, before resolution, because adding a module changes the graph.
    // The single auth-role provider to wire, resolved once by role and reused for both the
    // selection augmentation and the cluster/`AuthConfig` emission below — so the two never
    // diverge (emitting an ext_authz filter pointed at a provider that isn't in the graph).
    // `None` when the knob is off or the catalog has no `auth`-role provider; in the latter case
    // auth stays off entirely rather than gating the gateway behind a phantom upstream.
    let auth_provider: Option<ModuleId> = if gateway_auth_enabled(selection, catalog)? {
        catalog
            .providers_of(Role::AUTH)
            .first()
            .map(|p| (*p).clone())
    } else {
        None
    };
    let augmented;
    let selection = if let Some(provider) = &auth_provider {
        let mut s = selection.clone();
        if !s.modules.contains(provider) {
            s.modules.push(provider.clone());
        }
        augmented = s;
        &augmented
    } else {
        selection
    };

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

    // Resolve each module's knobs to their canonical values once, up front, and compute each
    // module's services from them. Knob resolution depends only on the knob defaults and the
    // selection overrides (never on services), so there is no circularity: the services a
    // module emits may *vary* with its knobs, but the knobs are settled first. Every later
    // pass reads these caches instead of re-resolving or re-deriving.
    let mut resolved_knobs: BTreeMap<ModuleId, ResolvedKnobs> = BTreeMap::new();
    for module in &graph.nodes {
        let overrides = selection.knob_overrides.get(module.id());
        let mut knobs = ResolvedKnobs::new();
        for knob in module.knobs() {
            if let Some(value) = resolve_knob(module.id(), knob, overrides)? {
                knobs.set(&knob.key, value);
            }
        }
        resolved_knobs.insert(module.id().clone(), knobs);
    }
    let services_of: BTreeMap<ModuleId, Vec<ServiceSpec>> = graph
        .nodes
        .iter()
        .map(|module| {
            (
                module.id().clone(),
                module.services(&resolved_knobs[module.id()]),
            )
        })
        .collect();

    // Resources to provision, grouped by the *chosen provider* (deduped, in dependency
    // order). Grouping by provider — not by abstract role — is what lets one object-store
    // demand land on SeaweedFS and another on Azurite, each provisioning on its own init.
    // Walk modules in graph (dependency) order so the grouped names stay deterministic.
    let mut provisioned_by: BTreeMap<ModuleId, Vec<String>> = BTreeMap::new();
    for module in &graph.nodes {
        for (idx, demand) in module.needs().iter().enumerate() {
            let provider = chosen[&(module.id().clone(), idx)].provider.clone();
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
            for service in &services_of[module.id()] {
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
    rewrite_gatewayed_object_store_endpoints(
        &mut chosen,
        &services_of,
        &graph,
        ctx,
        &dedicated_ports,
    );

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
        for (k, v) in module.provides().env_vars.iter() {
            module_env.set(k, v);
        }
        // Inject the module's resolved knob values (computed once up front): each lands under
        // the knob's `key`, so a fragment or a mounted config file reads it as `{{ env.KEY }}`
        // — the same injection point as `DATA_ROOT` and `BASE_PATH`.
        for (k, v) in resolved_knobs[module.id()].iter() {
            module_env.set(k, v);
        }
        // Bind each demand's resolved connection back into the consuming module's env, by
        // typed field. The connection was resolved once up front (in `chosen`).
        for (idx, demand) in module.needs().iter().enumerate() {
            let connection = &chosen[&(module.id().clone(), idx)].connection;
            bind_connection(&mut module_env, module.id(), demand, connection)?;
        }
        for service in &services_of[module.id()] {
            for endpoint in &service.endpoints {
                match &endpoint.intent {
                    RouteIntent::Internal => {} // no route
                    RouteIntent::Api => {
                        // The mount prefix is the endpoint's typed `mount_prefix` (its `path`
                        // stays empty so the resolver's `join(prefix, path)` round-trips to
                        // exactly the prefix).
                        let prefix = endpoint.mount_prefix.clone().unwrap_or_default();
                        require_prefix(&prefix, service, &endpoint.id)?;
                        claim(&mut claimed, &prefix, service, &endpoint.id)?;
                        let rewrite = api_rewrite(&endpoint.rewrite, &service.base_path, &prefix);
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
                        let base = service.base_path.clone();
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
        injected.insert(module.id().clone(), module_env);
    }

    // The user's app, when present, becomes the gateway's catch-all: an `app` cluster and a `/`
    // route on the shared listener. The `/` route is the shortest prefix, so the
    // most-specific-first sort below settles it last — exactly where a catch-all belongs. Run
    // the same collision checks every module route gets (rather than blindly injecting), so an
    // app-vs-module conflict fails loudly here instead of as an Envoy config error at launch.
    if let Some(app) = &ctx.app {
        // A module that already claimed `/` (an Api `mount_prefix: "/"` or a UiPrefixable
        // `base_path: "/"`) would be silently shadowed by the catch-all; reject instead.
        if let Some(first) = claimed.get("/") {
            return Err(PlanError::AppUpstreamCollision {
                conflict: first.clone(),
                reason: "it already claims the `/` route the app catch-all needs".into(),
            });
        }
        // `app` is the reserved cluster name for the upstream; a module service named `app`
        // would produce a duplicate Envoy cluster.
        if let Some(c) = gateway.clusters.iter().find(|c| c.name == "app") {
            return Err(PlanError::AppUpstreamCollision {
                conflict: c.name.clone(),
                reason: "a module contributes a service named `app`, the cluster name the app \
                         upstream reserves"
                    .into(),
            });
        }
        gateway.clusters.push(ClusterConfig {
            name: "app".into(),
            host: app.service.clone(),
            port: app.port,
        });
        shared_routes.push(GatewayRoute {
            prefix: "/".into(),
            cluster: "app".into(),
            rewrite: None,
        });
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

    // Forward-auth: when an auth provider was resolved (knob on + a provider exists), add its
    // upstream cluster and record the `AuthConfig` so the Envoy renderer gates the shared
    // listener behind `ext_authz`. The provider joined the graph above (same `auth_provider`), so
    // it is a real service; we derive the cluster name/host/port from its resolved service and
    // `Internal` endpoint — not from hardcoded constants — so a differently-named or -ported
    // provider is wired correctly. The cluster is emitted via `ensure_cluster` (the same helper
    // the routing loop uses) for dedup + port-conflict detection; the routing loop itself skips
    // it because the auth endpoint is `Internal` (no route).
    if let Some(provider) = &auth_provider {
        let service =
            services_of[provider]
                .first()
                .ok_or_else(|| PlanError::AuthProviderMisconfigured {
                    module: provider.clone(),
                    reason: "declares no service".into(),
                })?;
        let endpoint = service
            .endpoints
            .iter()
            .find(|e| e.intent == RouteIntent::Internal)
            .ok_or_else(|| PlanError::AuthProviderMisconfigured {
                module: provider.clone(),
                reason: "service has no Internal endpoint".into(),
            })?;
        let path_prefix = catalog
            .get(provider)
            .and_then(|m| m.provides().extras.get(EXT_AUTHZ_PATH_EXTRA))
            .ok_or_else(|| PlanError::AuthProviderMisconfigured {
                module: provider.clone(),
                reason: format!("does not declare the `{EXT_AUTHZ_PATH_EXTRA}` extra"),
            })?
            .clone();
        ensure_cluster(&mut gateway, service, endpoint)?;
        // The login portal is routed through the gateway at `/<service>` (a generic, provider-
        // agnostic path — `/authelia` for the bundled provider), so the deny-redirect a
        // logged-out browser follows actually resolves. The provider's session config must point
        // its `authelia_url` here.
        let portal_prefix = format!("/{}", service.name);
        gateway.auth = Some(AuthConfig {
            cluster: service.name.clone(),
            port: endpoint.internal_port,
            path_prefix,
            portal_prefix,
            identity_headers: AUTH_IDENTITY_HEADERS
                .iter()
                .map(|h| (*h).to_string())
                .collect(),
        });
    }

    gateway.clusters.sort_by(|a, b| a.name.cmp(&b.name));

    // Aggregate the stack's env vars for `.env`, last-writer-wins in dependency order:
    // each module's *declared* env vars, plus the coordinates injected to satisfy its
    // demands (a fragment reads those as `${VAR}`, so compose must resolve them from
    // `.env` at run time). Render-only injections (`BASE_PATH`, `DATA_ROOT`) stay out of
    // `.env` — they are rendered into the fragment at plan time.
    let mut env = InjectedEnv::new();
    for module in &graph.nodes {
        for (k, v) in module.provides().env_vars.iter() {
            env.set(k, v);
        }
        // A provider contributes its connections' conventional SDK env vars (an object
        // store's `AWS_*` / `AZURE_STORAGE_CONNECTION_STRING`), derived from the typed
        // credential so it is stated once. Only an in-graph (i.e. chosen) provider reaches
        // here, so an unselected backend's credentials never leak into `.env`. The values
        // are name-independent, so the template's credential is read directly.
        for template in module.provides().resource_kinds.values() {
            for (k, v) in template.0.standard_env() {
                env.set(k, v);
            }
        }
        for (idx, demand) in module.needs().iter().enumerate() {
            let connection = &chosen[&(module.id().clone(), idx)].connection;
            bind_connection(&mut env, module.id(), demand, connection)?;
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
        let module_env = injected.get(module.id()).cloned().unwrap_or_default();
        // The typed connections resolved for this module's demands, grouped by role, so a
        // `Template` fragment can branch on the chosen credential flavour. Alongside them,
        // the resolved `depends_on` gates — one per demand whose chosen provider advertises a
        // startup gate — so a fragment iterates them instead of hard-coding which backend's
        // service it waits on.
        let mut connections: BTreeMap<String, Vec<Connection>> = BTreeMap::new();
        let mut dependencies: Vec<crate::catalog::module::DepGate> = Vec::new();
        for (idx, demand) in module.needs().iter().enumerate() {
            let provider = &chosen[&(module.id().clone(), idx)].provider;
            connections
                .entry(demand.resource.clone())
                .or_default()
                .push(chosen[&(module.id().clone(), idx)].connection.clone());
            if let Some(gate) = provider_dep_gate(catalog, provider) {
                dependencies.push(gate);
            }
        }
        // A *provider* renders against its own role too: the names it provisions (`objects`,
        // for an init block to iterate) plus its own connection resolved for each — so its
        // fragment reads e.g. `connections.object_store.0.credential` rather than a `${VAR}`.
        let objects = provisioned_by.get(module.id()).cloned().unwrap_or_default();
        for (role, template) in module.provides().resource_kinds.iter() {
            let role_conns = connections.entry(role.clone()).or_default();
            for name in &objects {
                role_conns.push(template.resolve(name));
            }
        }
        // The gateway module publishes every listener's host port: its fragment iterates
        // `published_ports` to render compose `ports:`, so dedicated listeners (object stores)
        // are reachable from the host without the fragment hard-coding a port list.
        let published_ports = if services_of[module.id()]
            .iter()
            .any(|s| s.role == Role::gateway())
        {
            gateway
                .listeners
                .iter()
                .map(|l| crate::catalog::module::PortMapping {
                    host: l.host_port,
                    container: l.internal_port,
                })
                // The Envoy admin endpoint is not a routing listener, so it isn't in
                // `gateway.listeners`; publish it explicitly (1:1) alongside them.
                .chain(std::iter::once(crate::catalog::module::PortMapping {
                    host: ctx.gateway_admin_port,
                    container: ctx.gateway_admin_port,
                }))
                .collect()
        } else {
            Vec::new()
        };
        let render_ctx = crate::catalog::module::RenderCtx {
            env: &module_env,
            connections,
            dependencies,
            objects,
            published_ports,
        };
        let mut out = module
            .render()
            .render(&render_ctx)
            .map_err(|source| PlanError::Render {
                module: module.id().0.clone(),
                source,
            })?;
        // Root each emitted file under the module's own directory (`modules/<id>/<path>`),
        // so a module never hard-codes the global layout and its files sit beside its
        // fragment. The rewritten path is the single source of truth the consumer writes to
        // and the compose `configs: file:` references.
        for f in &mut out.files {
            f.path = format!("modules/{}/{}", module.id().as_str(), f.path);
            if let Some(alias) = &f.alias {
                if let Some(first) = alias_owner.get(alias) {
                    return Err(PlanError::ConfigAliasCollision {
                        alias: alias.clone(),
                        first: first.clone(),
                        second: module.id().clone(),
                    });
                }
                alias_owner.insert(alias.clone(), module.id().clone());
                configs.push(ConfigDecl {
                    alias: alias.clone(),
                    path: f.path.clone(),
                });
            }
        }
        if !out.fragment.trim().is_empty() {
            includes.push(ComposeInclude {
                module: module.id().clone(),
                fragment: out.fragment.clone(),
            });
        }
        renders.push((module.id().clone(), out));
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

    Ok(Plan {
        graph,
        routes,
        injected,
        services: services_of,
        connections,
        renders,
        head,
        gateway,
        postgres_databases,
        s3_buckets,
        azure_containers,
        env,
        gateway_facts: GatewayFacts {
            service: ctx.gateway_service.clone(),
            internal_port: ctx.gateway_internal_port,
            host_port: ctx.gateway_host_port,
        },
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
    // (provider ordered before consumer, like a `requires`). These are passed to `resolve`
    // as `extra_edges` rather than folded into any module — a module is never mutated.
    let mut demand_edges: ExtraEdges = ExtraEdges::new();
    let mut selected = resolve_selection(selection, catalog)?;

    // Fixed point: resolve → scan demands → add missing providers → repeat. Bounded by
    // the catalog size (each iteration adds ≥1 module).
    for _ in 0..=catalog.modules().len() {
        let graph = resolve(&selected, catalog.modules())?;
        let mut added = false;
        for module in &graph.nodes {
            for demand in module.needs() {
                let provider = choose_provider(ctx, catalog, demand, Some(module.id()))?;
                let edges = demand_edges.entry(module.id().clone()).or_default();
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
            // Final resolve with the demand edges threaded in, so providers start before
            // the consumers that demand their resources.
            return resolve_with(&selected, catalog.modules(), &demand_edges).map_err(Into::into);
        }
    }
    // Unreachable in practice (the loop bound exceeds the max modules addable).
    resolve_with(&selected, catalog.modules(), &demand_edges).map_err(Into::into)
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
    demand: &crate::catalog::module::ResourceDemand,
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

/// Bind a demand's resolved [`Connection`] into `env` by typed field, per the demand's
/// [`ConnectionBinding`](crate::ConnectionBinding).
///
/// Each `(field, key)` pair sets `key` to the connection's value for `field`. A field the
/// connection variant does not carry is a [`PlanError::UnboundConnectionField`].
fn bind_connection(
    env: &mut InjectedEnv,
    consumer: &ModuleId,
    demand: &crate::catalog::module::ResourceDemand,
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
    knob: &crate::catalog::module::Knob,
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
fn coerce_knob(raw: &str, kind: &crate::catalog::module::KnobKind) -> Option<String> {
    use crate::catalog::module::KnobKind;
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
        for (idx, demand) in module.needs().iter().enumerate() {
            let provider_id = choose_provider(ctx, catalog, demand, Some(module.id()))?;
            let provider = catalog
                .get(&provider_id)
                .expect("provider id is in catalog");
            let template = provider
                .provides()
                .resource_kinds
                .get(&demand.resource)
                .expect("chosen provider provisions the demanded role");
            let connection = template.resolve(&demand.name);
            chosen.insert(
                (module.id().clone(), idx),
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
        for demand in module.needs() {
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
            .provides()
            .resource_kinds
            .keys()
            .map(String::as_str)
            .collect();
        if let Some(cap) = module.provider_of() {
            roles.insert(cap);
        }
        for role in roles {
            by_role
                .entry(role.to_string())
                .or_default()
                .push(module.id().clone());
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
                .flat_map(|m| m.needs())
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
    services_of: &BTreeMap<ModuleId, Vec<ServiceSpec>>,
    graph: &ResolvedGraph,
    ctx: &PlanCtx,
    dedicated_ports: &BTreeMap<(String, String), u16>,
) {
    // provider module id → the in-network gateway origin its store is now reached at.
    let mut provider_origin: BTreeMap<ModuleId, String> = BTreeMap::new();
    for module in &graph.nodes {
        for service in &services_of[module.id()] {
            for endpoint in &service.endpoints {
                if endpoint.intent == RouteIntent::Gatewayed
                    && let Some(port) =
                        dedicated_ports.get(&(service.name.clone(), endpoint.id.clone()))
                {
                    provider_origin.insert(
                        module.id().clone(),
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
    if let crate::model::connection::ObjectStoreCredential::AzureBlob { connection_string } =
        credential
    {
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
/// [`DepGate`](crate::catalog::module::DepGate) the planner hands the consumer's render. An
/// unrecognized or missing condition defaults to
/// [`ServiceStarted`](crate::DependsCondition::ServiceStarted), the weakest compose gate.
fn provider_dep_gate(
    catalog: &Catalog,
    provider: &ModuleId,
) -> Option<crate::catalog::module::DepGate> {
    let module = catalog.get(provider)?;
    let gate = module.provides().extras.get(DEP_GATE_EXTRA)?;
    let (service, condition) = match gate.split_once(':') {
        Some((s, c)) => (s, crate::catalog::module::DependsCondition::parse(c)),
        None => (gate.as_str(), None),
    };
    Some(crate::catalog::module::DepGate {
        service: service.to_string(),
        condition: condition.unwrap_or(crate::catalog::module::DependsCondition::ServiceStarted),
    })
}

/// The gateway rewrite for an API route, for the structured [`GatewayConfig`], from the
/// endpoint's typed [`Rewrite`] and its service's `base_path`.
///
/// Tri-state result: `None` means forward the path unchanged (no rewrite emitted);
/// `Some(path)` means rewrite the matched prefix to `path`.
///
/// - [`Rewrite::Passthrough`] forwards unchanged (`None`).
/// - [`Rewrite::To`] rewrites to the literal path.
/// - [`Rewrite::Inherit`] joins the service's `base_path` with the client `prefix`; a
///   service serving at root (empty base path) needs no rewrite (`None`).
fn api_rewrite(rewrite: &Rewrite, base_path: &str, prefix: &str) -> Option<String> {
    match rewrite {
        Rewrite::Passthrough => None,
        Rewrite::To(path) => Some(path.clone()),
        Rewrite::Inherit => {
            if base_path.is_empty() {
                None
            } else {
                Some(join_path(base_path, prefix))
            }
        }
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

    fn shared_routes(plan: &Plan) -> &[GatewayRoute] {
        &plan.gateway.listeners[0].routes
    }

    fn route_for<'a>(plan: &'a Plan, prefix: &str) -> &'a GatewayRoute {
        shared_routes(plan)
            .iter()
            .find(|r| r.prefix == prefix)
            .unwrap_or_else(|| panic!("no route for {prefix}"))
    }

    /// A one-service module with `endpoints`, for exercising planner guards. Returns a
    /// concrete [`DataModule`] so a test can still tweak fields (e.g. `render`) before
    /// wrapping it as an `Arc<dyn Module>` for the catalog.
    fn module_with(
        id: &str,
        endpoints: Vec<Endpoint>,
        provides: crate::catalog::module::Provides,
    ) -> crate::catalog::module::DataModule {
        crate::catalog::module::DataModule {
            id: id.into(),
            display_name: None,
            summary: None,
            category: None,
            provider_of: None,
            requires: vec![],
            conflicts_with: vec![],
            needs: vec![],
            service_specs: vec![ServiceSpec {
                name: id.to_string(),
                role: crate::model::role::Role::new("svc"),
                placement: Placement::Container {
                    service: id.to_string(),
                },
                endpoints,
                depends_on: vec![],
                base_path: String::new(),
            }],
            provides,
            knobs: vec![],
            render: Default::default(),
        }
    }

    fn ep(id: &str, port: u16, intent: RouteIntent) -> Endpoint {
        // An `Api` endpoint needs a mount prefix; default it to `/<id>` so the guard tests
        // (which exercise `Api`) have a non-empty prefix. Non-`Api` intents ignore it.
        let mount_prefix = matches!(intent, RouteIntent::Api).then(|| format!("/{id}"));
        Endpoint {
            id: id.into(),
            scheme: crate::model::endpoint::Scheme::Http,
            internal_port: port,
            host_port: None,
            intent,
            path: String::new(),
            mount_prefix,
            rewrite: Rewrite::Inherit,
        }
    }

    /// Wrap a [`DataModule`] as an `Arc<dyn Module>` for the catalog.
    fn arc(
        m: crate::catalog::module::DataModule,
    ) -> std::sync::Arc<dyn crate::catalog::module::Module> {
        std::sync::Arc::new(m)
    }

    /// A minimal gateway module (role `gateway`) carrying the `ENVOY_AUTH` knob, under a
    /// deliberately NON-`envoy` id — so the by-role lookups are exercised, not an id match.
    fn gateway_module(auth_default_on: bool) -> crate::catalog::module::DataModule {
        let mut m = module_with(
            "edge",
            vec![ep("http", 10000, RouteIntent::Internal)],
            crate::catalog::module::Provides::default(),
        );
        m.provider_of = Some("gateway".into());
        m.service_specs[0].role = Role::gateway();
        m.knobs = vec![crate::catalog::module::Knob {
            key: ENVOY_AUTH_KNOB.into(),
            title: None,
            kind: crate::catalog::module::KnobKind::Bool,
            default: Some(if auth_default_on { "true" } else { "false" }.into()),
            required: false,
            help: None,
        }];
        m
    }

    /// An auth-role provider deliberately NOT named `authelia` and NOT on 9091, with its own
    /// ext_authz path — so the planner must read the provider's real coordinates, not constants.
    fn auth_provider_module() -> crate::catalog::module::DataModule {
        let mut provides = crate::catalog::module::Provides::default();
        provides
            .extras
            .insert(EXT_AUTHZ_PATH_EXTRA.into(), "/verify".into());
        let mut m = module_with(
            "gatekeeper",
            vec![ep("http", 4180, RouteIntent::Internal)],
            provides,
        );
        m.provider_of = Some("auth".into());
        m.service_specs[0].role = Role::auth();
        m
    }

    #[test]
    fn ext_authz_wiring_follows_a_renamed_reported_auth_provider() {
        // Knob on, provider `gatekeeper` on :4180 with path /verify: the emitted cluster and the
        // AuthConfig must follow the resolved provider, never the bundled authelia/9091.
        let p = plan_env(
            &Selection::modules(["edge"]),
            &Catalog::from_modules([arc(gateway_module(true)), arc(auth_provider_module())]),
            &PlanCtx::default(),
        )
        .expect("plan succeeds");
        let auth = p.gateway.auth.expect("auth wired when a provider exists");
        assert_eq!(auth.cluster, "gatekeeper");
        assert_eq!(auth.port, 4180);
        assert_eq!(auth.path_prefix, "/verify");
        assert_eq!(auth.portal_prefix, "/gatekeeper");
        // The provider's cluster is emitted under its real name/port; no phantom authelia.
        let c = p
            .gateway
            .clusters
            .iter()
            .find(|c| c.name == "gatekeeper")
            .expect("provider cluster emitted");
        assert_eq!((c.host.as_str(), c.port), ("gatekeeper", 4180));
        assert!(!p.gateway.clusters.iter().any(|c| c.name == "authelia"));
    }

    #[test]
    fn knob_on_but_no_auth_provider_leaves_the_gateway_open() {
        // Divergent-guard fix: the knob defaults on but the catalog has no `auth`-role provider.
        // Auth must stay off entirely rather than gate the gateway behind a phantom upstream.
        let p = plan_env(
            &Selection::modules(["edge"]),
            &Catalog::from_modules([arc(gateway_module(true))]),
            &PlanCtx::default(),
        )
        .expect("plan succeeds");
        assert!(
            p.gateway.auth.is_none(),
            "no auth provider → no AuthConfig, gateway left open"
        );
    }

    #[test]
    fn auth_provider_without_ext_authz_path_extra_is_rejected() {
        // A misconfigured auth provider (Internal endpoint but no ext_authz path extra) is a
        // plan-time error, not a silently malformed gateway config.
        let mut provider = module_with(
            "gatekeeper",
            vec![ep("http", 4180, RouteIntent::Internal)],
            crate::catalog::module::Provides::default(),
        );
        provider.provider_of = Some("auth".into());
        provider.service_specs[0].role = Role::auth();
        let err = plan_env(
            &Selection::modules(["edge"]),
            &Catalog::from_modules([arc(gateway_module(true)), arc(provider)]),
            &PlanCtx::default(),
        )
        .unwrap_err();
        assert!(
            matches!(err, PlanError::AuthProviderMisconfigured { .. }),
            "expected AuthProviderMisconfigured, got {err:?}"
        );
    }

    #[test]
    fn empty_ui_base_path_is_rejected_not_a_catch_all() {
        // A UiPrefixable with no base_path would mount at `/` and shadow everything.
        let m = module_with(
            "svc",
            vec![ep("ui", 8080, RouteIntent::UiPrefixable)],
            Default::default(),
        );
        let err = plan_env(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([arc(m)]),
            &PlanCtx::default(),
        )
        .unwrap_err();
        assert_eq!(err, PlanError::MissingPrefix("svc.ui".into()));
    }

    #[test]
    fn duplicate_config_alias_across_modules_is_rejected() {
        use crate::catalog::module::RenderSpec;
        use crate::render::RenderFile;

        // Two modules each declare a `RenderFile` under the same alias. Aliases share one
        // top-level `configs:` namespace, so the planner must refuse rather than let one
        // silently shadow the other.
        let with_alias = |id: &str| {
            let mut m = module_with(id, vec![], Default::default());
            m.render = RenderSpec {
                fragment: format!("services:\n  {id}: {{}}\n"),
                files: vec![RenderFile {
                    path: "conf.yaml".into(),
                    contents: "k: v\n".into(),
                    alias: Some("shared_alias".into()),
                }],
            };
            arc(m)
        };
        let err = plan_env(
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
        use crate::catalog::module::RenderSpec;
        use crate::render::RenderFile;

        // A single aliased file is rooted under its module dir and surfaces as one `configs:`
        // declaration on the head.
        let mut m = module_with("svc", vec![], Default::default());
        m.render = RenderSpec {
            fragment: "services:\n  svc: {}\n".into(),
            files: vec![RenderFile {
                path: "app.toml".into(),
                contents: "x = 1\n".into(),
                alias: Some("svc_config".into()),
            }],
        };
        let p = plan_env(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([arc(m)]),
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
        // service-named cluster. Each `Api` endpoint carries its own `mount_prefix`
        // (`/a`, `/b`) via the `ep` helper, so they don't collide on prefix — only on port.
        let m = module_with(
            "svc",
            vec![
                ep("a", 8080, RouteIntent::Api),
                ep("b", 9090, RouteIntent::Api),
            ],
            Default::default(),
        );
        let err = plan_env(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([arc(m)]),
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
        let p = plan_env(
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
        let s3 = plan_env(
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
        let az = plan_env(
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
        let p = plan_env(
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
        let p = plan_env(
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
        let p = plan_env(
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
        let p = plan_env(&sel, &baseline_catalog(), &PlanCtx::default()).unwrap();
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
        let err = plan_env(&sel, &baseline_catalog(), &PlanCtx::default()).unwrap_err();
        assert_eq!(err, PlanError::UnknownCapability("telepathy".into()));
    }

    #[test]
    fn plan_is_deterministic_regardless_of_selection_order() {
        let cat = baseline_catalog();
        let a = plan_env(&default_selection(), &cat, &PlanCtx::default()).unwrap();
        let reversed =
            Selection::modules(["unity-catalog", "mlflow", "seaweedfs", "postgres", "envoy"]);
        let b = plan_env(&reversed, &cat, &PlanCtx::default()).unwrap();
        // `Plan` is not `Eq` (its graph holds trait objects); compare the
        // observable, contracted-deterministic artifacts instead.
        assert_eq!(a.gateway, b.gateway);
        assert_eq!(a.head, b.head);
        assert_eq!(a.routes, b.routes);
    }

    #[test]
    fn resolve_knob_prefers_override_then_default() {
        use crate::catalog::module::{Knob, KnobKind};

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
        use crate::catalog::module::{Knob, KnobKind};

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
        use crate::catalog::module::{Knob, KnobKind};

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
        use crate::catalog::module::{Knob, KnobKind};

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
