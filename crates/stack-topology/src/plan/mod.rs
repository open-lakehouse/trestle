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
//! 1. resolves the dependency graph ([`resolve`](fn@crate::resolve)) — transitive `requires`,
//!    conflicts, topological order;
//! 2. derives a non-colliding gateway route for every surface endpoint, **erroring
//!    loudly on a real collision** rather than silently shadowing one route with
//!    another (the failure mode hand-authored routes invite);
//! 3. emits a structured [`GatewayConfig`] (listeners + clusters) for the gateway,
//!    and a [`RoutePlan`] the [`address`](crate::address) resolver consumes unchanged;
//! 4. renders every module to a [`RenderOutput`] and assembles a consolidated
//!    [`HeadFile`] (config mounts + injected env at the head, then `include:`s).
//!
//! When a module declares [`impersonated_hosts`](crate::Provides::impersonated_hosts), the
//! planner also gives the gateway a TLS listener for them ([`TlsListenerConfig`]): the
//! gateway holds each host as a network alias, and every connection vended by that module
//! carries the [`TlsTrust`](crate::TlsTrust) a consumer needs to trust the local CA.
//!
//! # How a prefix is derived (the crux)
//!
//! Prefixes are derived from *declared semantic facts*, never invented from names:
//!
//! - An [`Api`](RouteIntent::Api) endpoint's prefix is its declared client mount
//!   ([`Endpoint::mount_prefix`](crate::Endpoint::mount_prefix); the endpoint's own `path`
//!   stays empty so the resolver does not double it). Its upstream rewrite follows the
//!   endpoint's typed [`Rewrite`](crate::Rewrite): `Inherit` joins the service's `base_path`
//!   with the prefix, `To` rewrites to a literal path, `Passthrough` forwards unchanged.
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

use serde::{Deserialize, Serialize};

mod client_env;
mod demands;
mod gateway;
mod impersonation;
mod knobs;
pub mod resolve;
pub mod routing;

use crate::address::{AddressError, ServiceRef};
use crate::catalog::Catalog;
use crate::catalog::baseline::{
    CLIENT_ENV_KEYS_VAR, DATA_ROOT_DEFAULT, DATA_ROOT_VAR, GATEWAY_AUTH_VAR,
    GATEWAY_INTERNAL_URL_VAR, GATEWAY_URL_VAR,
};
use crate::catalog::module::{ExtraResource, ModuleId, StackFacts};
use crate::model::connection::Connection;
use crate::model::endpoint::RouteIntent;
use crate::model::placement::Vantage;
use crate::model::role::{Role, ServiceSpec};
use crate::plan::resolve::{ResolveError, ResolvedGraph};
use crate::plan::routing::{AssignedRoute, Listener, RoutePlan};
use crate::render::{InjectedEnv, RenderOutput};

use client_env::resolve_client_env;
use demands::{
    bind_connection, check_role_exclusivity, provider_dep_gate, provisioned_by_provider,
    resolve_demand_providers, resolve_with_demands,
};
pub use gateway::{
    AuthConfig, ClusterConfig, ENVOY_AUTH_KNOB, ENVOY_AUTH_KNOB_LEGACY, EXT_AUTHZ_PATH_EXTRA,
    GATEWAY_TLS_PORT, GatewayConfig, GatewayRoute, ListenerConfig, PolicyConfig, TlsListenerConfig,
    TlsVirtualHost,
};
use gateway::{
    allocate_dedicated_ports, api_rewrite, claim, ensure_cluster, gateway_auth_enabled,
    require_prefix, selected_auth_provider, selected_policy_provider, wire_app_upstream, wire_auth,
    wire_policy,
};
use impersonation::{plan_impersonation, rewrite_gatewayed_object_store_endpoints};
use knobs::resolve_module_knobs;

/// What modules an environment should contain.
///
/// Modules can be picked directly (combine technologies) or by capability ("I want
/// experiment tracking"); the planner unions both. Capability names are matched
/// against the catalog's [`provider_of`](crate::Module::provider_of) index.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Module ids selected directly.
    #[serde(default)]
    pub modules: Vec<ModuleId>,
    /// Capabilities selected; each is mapped to its provider module(s) in the catalog.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Per-module knob overrides: module id → (knob `key` → value). A value present
    /// here wins over the knob's declared [`default`](crate::Knob::default); a knob
    /// absent from this map falls back to its default. The value is available to the
    /// module's template as `{{ knobs.key }}` (and to [`Module::services`] via
    /// [`ResolvedKnobs`](crate::catalog::module::ResolvedKnobs)).
    ///
    /// This is the channel a config UI feeds: it surfaces a module's knobs from the
    /// catalog, lets the user tune them, and hands the chosen values back here.
    #[serde(default)]
    pub knob_overrides: BTreeMap<ModuleId, BTreeMap<String, String>>,
    /// Resources to provision at the environment level that no module consumes — extra
    /// databases or object-store buckets the operator wants created for a process *outside*
    /// the stack (e.g. a host tool querying a bucket through the gateway). Each is provisioned
    /// on its role's chosen provider (pulling that provider into the graph if no module already
    /// demands the role) but never bound into any module's env. See [`ExtraResource`].
    #[serde(default)]
    pub extra_resources: Vec<ExtraResource>,
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
            ..Default::default()
        }
    }
}

/// The user's own application, which the gateway fronts with a `/` catch-all so any
/// request not matched by a module route reaches the app. Supplied via
/// [`PlanCtx::app`]; the planner emits its cluster and the trailing catch-all route into
/// the [`GatewayConfig`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
///
/// Every field is `#[serde(default)]` so a persisted manifest can omit any it does not
/// override; the missing fields fall back to this type's [`Default`] (the same values the
/// code uses), keeping a hand-edited TOML terse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCtx {
    /// The compose project / environment name (used as the head file's `name:`).
    #[serde(default = "PlanCtx::default_env_name")]
    pub env_name: String,
    /// The gateway's compose service / DNS name (e.g. `"envoy"`).
    #[serde(default = "PlanCtx::default_gateway_service")]
    pub gateway_service: String,
    /// The gateway's listening port inside the compose network (e.g. `10000`).
    #[serde(default = "PlanCtx::default_gateway_internal_port")]
    pub gateway_internal_port: u16,
    /// The gateway's host-published port (e.g. `9080`).
    #[serde(default = "PlanCtx::default_gateway_host_port")]
    pub gateway_host_port: u16,
    /// The gateway's Envoy admin port, on both sides (`address:0.0.0.0:<port>` in the
    /// bootstrap and the `<port>:<port>` host publish). Configurable so two stacks rendered
    /// on the same host don't collide on the admin endpoint. Defaults to `9901`.
    #[serde(default = "PlanCtx::default_gateway_admin_port")]
    pub gateway_admin_port: u16,
    /// Explicit host ports to hand out, in order, to endpoints that need their own
    /// dedicated listener ([`UiFixed`](RouteIntent::UiFixed) /
    /// [`Gatewayed`](RouteIntent::Gatewayed)). Consumed first; once exhausted the planner
    /// auto-allocates from [`dedicated_listener_port_base`](Self::dedicated_listener_port_base),
    /// so "every object store is exposed" works with no explicit list.
    #[allow(clippy::struct_field_names)]
    #[serde(default)]
    pub dedicated_listener_ports: Vec<u16>,
    /// The base host port the planner auto-allocates dedicated listeners from when
    /// [`dedicated_listener_ports`](Self::dedicated_listener_ports) is exhausted: the first
    /// auto listener binds this port, the next `+1`, and so on. Defaults to `9100` (clear of
    /// the gateway's `9080`).
    #[allow(clippy::struct_field_names)]
    #[serde(default = "PlanCtx::default_dedicated_listener_port_base")]
    pub dedicated_listener_port_base: u16,
    /// Ordered provider preference per resource role — the environment's say in which
    /// implementation satisfies an abstract demand (e.g. `object_store` →
    /// `["azurite", "rustfs"]` for an env that prefers Azurite). The planner picks the first
    /// preferred provider present in the catalog; an empty/absent entry falls back to
    /// uniqueness then the catalog default. A demand's own `provider` pin still wins over this.
    #[serde(default)]
    pub provider_preference: BTreeMap<String, Vec<ModuleId>>,
    /// The stack's root data directory, injected into every module's render env as
    /// [`DATA_ROOT`](crate::DATA_ROOT_VAR) and resolved at plan time. A module that persists
    /// state mounts it under `${DATA_ROOT}/<module>` by convention, so relocating all
    /// persistence is this single knob (e.g. an absolute path) rather than an edit per
    /// fragment. Defaults to [`DATA_ROOT_DEFAULT`] (`./.data`,
    /// relative to the compose file).
    #[serde(default = "PlanCtx::default_data_root")]
    pub data_root: String,
    /// The user's app, if any: when set, the gateway gets an `app` cluster and a `/`
    /// catch-all route (emitted last, after every module route) forwarding unmatched
    /// requests to it. Defaults to `None`. Part of the plan because the catch-all is a
    /// structural fact about the gateway, not a render-time decoration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppUpstream>,
}

impl PlanCtx {
    fn default_env_name() -> String {
        "lakehouse".into()
    }
    fn default_gateway_service() -> String {
        "envoy".into()
    }
    fn default_gateway_internal_port() -> u16 {
        10000
    }
    fn default_gateway_host_port() -> u16 {
        9080
    }
    fn default_gateway_admin_port() -> u16 {
        9901
    }
    fn default_dedicated_listener_port_base() -> u16 {
        9100
    }
    fn default_data_root() -> String {
        DATA_ROOT_DEFAULT.into()
    }
}

impl Default for PlanCtx {
    fn default() -> Self {
        PlanCtx {
            env_name: PlanCtx::default_env_name(),
            gateway_service: PlanCtx::default_gateway_service(),
            gateway_internal_port: PlanCtx::default_gateway_internal_port(),
            gateway_host_port: PlanCtx::default_gateway_host_port(),
            gateway_admin_port: PlanCtx::default_gateway_admin_port(),
            dedicated_listener_ports: Vec::new(),
            dedicated_listener_port_base: PlanCtx::default_dedicated_listener_port_base(),
            provider_preference: BTreeMap::new(),
            data_root: PlanCtx::default_data_root(),
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
    /// Two upstreams claim the same gateway cluster name on different ports (e.g. two
    /// modules' emulated hostnames forwarding to one service on different ports).
    #[error("cluster `{service}` is claimed on conflicting ports {first} and {second}")]
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
    /// A module's [`client_env`](crate::Provides::client_env) entry could not be resolved —
    /// it names a service or endpoint the module does not have.
    #[error("client env `{key}` of module `{module}`: {source}")]
    ClientEnv {
        /// The contributing module.
        module: ModuleId,
        /// The env-var name.
        key: String,
        /// Why the value did not resolve.
        #[source]
        source: AddressError,
    },
    /// A `policy`-role module was selected but gateway auth is off (the `auth` knob is
    /// explicitly `false`, or the catalog has no auth provider). The policy engine decides on
    /// the identity the auth provider asserts, so without auth it could only deny everything.
    #[error(
        "policy engine `{module}` needs gateway auth to assert an identity; turn the gateway's \
         `auth` knob on"
    )]
    PolicyRequiresAuth {
        /// The selected policy-role module.
        module: ModuleId,
    },
    /// The selected `policy`-role provider does not declare what the gateway needs to wire its
    /// gRPC `ext_authz` check.
    #[error("policy provider `{module}` is misconfigured: {reason}")]
    PolicyProviderMisconfigured {
        /// The policy-role provider module.
        module: ModuleId,
        /// What specifically is missing.
        reason: String,
    },
    /// Two modules declared the same [`impersonated_hosts`](crate::Provides::impersonated_hosts)
    /// hostname. The gateway can only forward a host to one upstream.
    #[error("impersonated host `{host}` is declared by both `{first}` and `{second}`")]
    ImpersonatedHostCollision {
        /// The hostname both modules claim.
        host: String,
        /// The module that declared it first (in dependency order).
        first: ModuleId,
        /// The module that declared it again.
        second: ModuleId,
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

/// A top-level Compose `secrets:` declaration: an alias and its sensitive host file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretDecl {
    /// The alias mounted by services under `/run/secrets/<alias>`.
    pub alias: String,
    /// The host file path supplying the secret.
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
    /// Top-level Compose secret declarations, sorted by alias.
    pub secrets: Vec<SecretDecl>,
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
    /// Every resource each provider provisions, keyed by the chosen provider: the databases,
    /// buckets or containers its consumers demand, then the selection's
    /// [`extra_resources`](Selection::extra_resources) that landed on it, deduplicated in
    /// dependency order. This is exactly what each provider's init job creates (its
    /// `RenderCtx.objects`); the per-provider fields below are convenience views of it.
    pub provisioned: BTreeMap<ModuleId, Vec<String>>,
    /// Postgres databases the selected modules need, deduplicated in dependency order. These
    /// also reach the postgres provider's render as `RenderCtx.objects`, which its init-script
    /// `RenderFile` iterates; this field is the same list exposed for consumers.
    pub postgres_databases: Vec<String>,
    /// Object-store buckets provisioned on RustFS (when it is the chosen object-store
    /// provider), deduplicated in dependency order.
    pub s3_buckets: Vec<String>,
    /// Object-store containers provisioned on Azurite (when it is the chosen object-store
    /// provider), deduplicated in dependency order.
    pub azure_containers: Vec<String>,
    /// The stack's aggregated environment variables (drives the `.env` artifact),
    /// last-writer-wins in dependency order.
    pub env: InjectedEnv,
    /// The client environment resolved from the host (see [`Plan::client_env`]).
    pub client_env_host: InjectedEnv,
    /// The client environment resolved from inside the compose network (see
    /// [`Plan::client_env`]).
    pub client_env_container: InjectedEnv,
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

    /// The **client environment** from `from`: every variable a client (an app, notebook, or
    /// engine) loads to reach this stack — catalog, tracking, lineage and tracing endpoints,
    /// object-store coordinates — aggregated from each module's
    /// [`client_env`](crate::Provides::client_env) and resolved against this plan's routes.
    ///
    /// Credential-free by construction: clients authenticate separately. A host or in-process
    /// caller gets `localhost` addresses; a container on the compose network gets in-network
    /// ones.
    pub fn client_env(&self, from: Vantage) -> &InjectedEnv {
        match from {
            Vantage::Container => &self.client_env_container,
            Vantage::Host | Vantage::InProcess => &self.client_env_host,
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
    // An explicitly selected provider wins over the catalog's first `auth`-role provider.
    let auth_provider: Option<ModuleId> = if gateway_auth_enabled(selection, catalog)? {
        selected_auth_provider(selection, catalog).or_else(|| {
            catalog
                .providers_of(Role::AUTH)
                .first()
                .map(|p| (*p).clone())
        })
    } else {
        None
    };
    // A policy engine authorizes the identity forward-auth asserts, so it needs auth on.
    let policy_provider = selected_policy_provider(selection, catalog);
    if let (Some(module), None) = (&policy_provider, &auth_provider) {
        return Err(PlanError::PolicyRequiresAuth {
            module: module.clone(),
        });
    }
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
    check_role_exclusivity(&graph, selection)?;

    // Resolve each module's knobs to their canonical values once, up front, and compute each
    // module's services from them. Knob resolution depends only on the knob defaults and the
    // selection overrides (never on services), so there is no circularity: the services a
    // module emits may *vary* with its knobs, but the knobs are settled first. Every later
    // pass reads these caches instead of re-resolving or re-deriving.
    let resolved_knobs = resolve_module_knobs(&graph, selection)?;
    // Stack-wide facts a module's topology may depend on, settled before any services are
    // computed. Forward-auth is wired exactly when an auth provider was resolved above.
    let stack_facts = StackFacts {
        gateway_auth: auth_provider.is_some(),
    };
    let services_of: BTreeMap<ModuleId, Vec<ServiceSpec>> = graph
        .nodes
        .iter()
        .map(|module| {
            (
                module.id().clone(),
                module.services_in(&resolved_knobs[module.id()], &stack_facts),
            )
        })
        .collect();

    let provisioned_by = provisioned_by_provider(&graph, &chosen, selection, catalog, ctx)?;
    let provisioned_for = |id: &str| -> Vec<String> {
        provisioned_by
            .get(&ModuleId::from(id))
            .cloned()
            .unwrap_or_default()
    };
    // Convenience views for the artifact renderers (single-provider roles today).
    let postgres_databases = provisioned_for("postgres");
    let s3_buckets = provisioned_for("rustfs");
    let azure_containers = provisioned_for("azurite");

    // Allocate dedicated listener ports up front, before the routing loop, so a gatewayed
    // object store's connection `endpoint` can be rewritten to the gateway *before* consumers
    // bind it.
    let dedicated_ports = allocate_dedicated_ports(&graph, &services_of, ctx);

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
    // Emulated public hostnames: give the gateway a TLS listener for them, and mark every
    // connection those providers vend with the trust a consumer needs for the local CA.
    gateway.tls = plan_impersonation(&graph, &provisioned_by, &mut gateway)?;
    if let Some(tls) = &gateway.tls {
        let emulating: BTreeSet<&ModuleId> = graph
            .nodes
            .iter()
            .filter(|m| !m.provides().impersonated_hosts.is_empty())
            .map(|m| m.id())
            .collect();
        for resolved in chosen.values_mut() {
            if emulating.contains(&resolved.provider)
                && let Connection::ObjectStore { tls_trust, .. } = &mut resolved.connection
            {
                *tls_trust = Some(Box::new(tls.trust.clone()));
            }
        }
    }
    let mut injected: BTreeMap<ModuleId, InjectedEnv> = BTreeMap::new();

    let mut shared_routes: Vec<GatewayRoute> = Vec::new();
    let mut dedicated: Vec<ListenerConfig> = Vec::new();

    // Track claimed prefixes (per listener) to detect collisions. The shared listener
    // is keyed by its port; dedicated listeners never collide (own port).
    let mut claimed: BTreeMap<String, String> = BTreeMap::new();

    // Walk modules in dependency order so emitted routes/clusters are deterministic.
    let gateway_facts = GatewayFacts {
        service: ctx.gateway_service.clone(),
        internal_port: ctx.gateway_internal_port,
        host_port: ctx.gateway_host_port,
    };
    // What a host-side browser reaches the shared listener at (see `GATEWAY_URL_VAR`), and
    // what an in-network caller reaches it at (see `GATEWAY_INTERNAL_URL_VAR`).
    let gateway_url = format!("http://localhost:{}", ctx.gateway_host_port);
    let gateway_internal_url = format!(
        "http://{}:{}",
        ctx.gateway_service, ctx.gateway_internal_port
    );
    // Every client-environment key the stack defines (see `CLIENT_ENV_KEYS_VAR`).
    let client_env_keys = graph
        .nodes
        .iter()
        .flat_map(|m| m.provides().client_env.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    for module in &graph.nodes {
        let mut module_env = InjectedEnv::new();
        // The stack's root data directory, available to every fragment as a render-only value
        // (baked at plan time, like `BASE_PATH`). A module that persists state mounts it under
        // `{{ env.DATA_ROOT }}/<module>` rather than hard-coding a `./.data/...` path, so the
        // whole stack's persistence relocates via the one `PlanCtx::data_root` knob.
        module_env.set(DATA_ROOT_VAR, &ctx.data_root);
        module_env.set(GATEWAY_URL_VAR, &gateway_url);
        module_env.set(GATEWAY_INTERNAL_URL_VAR, &gateway_internal_url);
        module_env.set(
            GATEWAY_AUTH_VAR,
            if stack_facts.gateway_auth {
                "true"
            } else {
                "false"
            },
        );
        module_env.set(CLIENT_ENV_KEYS_VAR, &client_env_keys);
        // Seed each module's render env with its declared env vars.
        for (k, v) in module.provides().env_vars.iter() {
            module_env.set(k, v);
        }
        // Config knobs stay out of `env` — they are passed separately as `RenderCtx.knobs`
        // so templates read `{{ knobs.image }}` rather than conflating them with planner
        // injections / container env vars.
        // Bind each demand's resolved connection back into the consuming module's env, by
        // typed field. The connection was resolved once up front (in `chosen`).
        for (idx, demand) in module.needs().iter().enumerate() {
            let connection = &chosen[&(module.id().clone(), idx)].connection;
            bind_connection(&mut module_env, module.id(), demand, connection)?;
        }
        for service in &services_of[module.id()] {
            // A service that relocates its surface under a base path is told it at render
            // time (e.g. via a `[ui].base_path` config line), whichever endpoint intents it
            // routes the prefix with.
            if !service.base_path.is_empty() {
                module_env.set("BASE_PATH", &service.base_path);
            }
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
                        let cluster = ensure_cluster(&mut gateway, service, endpoint)?;
                        shared_routes.push(GatewayRoute {
                            prefix: prefix.clone(),
                            cluster: cluster.clone(),
                            rewrite: rewrite.clone(),
                            options: endpoint.route.clone(),
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
                        let cluster = ensure_cluster(&mut gateway, service, endpoint)?;
                        shared_routes.push(GatewayRoute {
                            prefix: base.clone(),
                            cluster: cluster.clone(),
                            rewrite: None,
                            options: endpoint.route.clone(),
                        });
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
                        let cluster = ensure_cluster(&mut gateway, service, endpoint)?;
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
                                cluster: cluster.clone(),
                                rewrite: None,
                                options: endpoint.route.clone(),
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
        wire_app_upstream(app, &claimed, &mut gateway, &mut shared_routes)?;
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
        wire_auth(provider, &services_of, catalog, &mut gateway)?;
    }
    // Authorization: a selected policy engine checks every request on the shared listener after
    // forward-auth (validated above to be on).
    if let Some(provider) = &policy_provider {
        wire_policy(provider, &services_of, &mut gateway)?;
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
    let mut secrets: Vec<SecretDecl> = Vec::new();
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
        // A consumer that trusts the gateway's CA must not start before it is minted (a JVM
        // reads its truststore once, at startup).
        if let Some(tls) = &gateway.tls {
            let trusts = connections.values().flatten().any(|c| {
                matches!(
                    c,
                    Connection::ObjectStore {
                        tls_trust: Some(_),
                        ..
                    }
                )
            });
            if trusts {
                dependencies.push(crate::catalog::module::DepGate {
                    service: tls.mint_service.clone(),
                    condition:
                        crate::catalog::module::DependsCondition::ServiceCompletedSuccessfully,
                });
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
        let is_gateway = services_of[module.id()]
            .iter()
            .any(|s| s.role == Role::gateway());
        let tls = match &gateway.tls {
            Some(t) if is_gateway => Some(crate::catalog::module::GatewayTls {
                hosts: t.hosts.clone(),
                trust: t.trust.clone(),
                mint_service: t.mint_service.clone(),
            }),
            _ => None,
        };
        let published_ports = if is_gateway {
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
            knobs: &resolved_knobs[module.id()],
            connections,
            dependencies,
            objects,
            published_ports,
            tls,
        };
        let mut out = module
            .render()
            .render(&render_ctx)
            .map_err(|source| PlanError::Render {
                module: module.id().0.clone(),
                source,
            })?;
        // Root each emitted file under the module's own directory (`modules/<id>/<path>`),
        // unless the module marked it `at_root` (operator-facing files live beside
        // `compose.yaml`). The rewritten path is the single source of truth the consumer
        // writes to and the compose `configs: file:` / `secrets: file:` references.
        for f in &mut out.files {
            if !f.at_root {
                f.path = format!("modules/{}/{}", module.id().as_str(), f.path);
            }
            if f.alias.is_some() && f.secret_alias.is_some() {
                return Err(PlanError::Render {
                    module: module.id().0.clone(),
                    source: crate::catalog::module::RenderError(
                        "a rendered file cannot be both a config and a secret".into(),
                    ),
                });
            }
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
            if let Some(alias) = &f.secret_alias {
                if let Some(first) = alias_owner.get(alias) {
                    return Err(PlanError::ConfigAliasCollision {
                        alias: alias.clone(),
                        first: first.clone(),
                        second: module.id().clone(),
                    });
                }
                alias_owner.insert(alias.clone(), module.id().clone());
                secrets.push(SecretDecl {
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
    secrets.sort_by(|a, b| a.alias.cmp(&b.alias));

    let head = HeadFile {
        name: ctx.env_name.clone(),
        env: env.clone(),
        includes,
        configs,
        secrets,
    };

    // The typed connections, exposed for downstream consumers (deterministic: built from
    // `chosen`, which is populated walking the graph in dependency order).
    let connections = chosen
        .into_iter()
        .map(|(key, c)| (key, c.connection))
        .collect();

    let mut plan = Plan {
        graph,
        routes,
        injected,
        services: services_of,
        connections,
        renders,
        head,
        gateway,
        provisioned: provisioned_by,
        postgres_databases,
        s3_buckets,
        azure_containers,
        env,
        gateway_facts,
        client_env_host: InjectedEnv::new(),
        client_env_container: InjectedEnv::new(),
    };
    // Resolved last: client addresses read the settled routes through the plan's own
    // addressing API.
    plan.client_env_host = resolve_client_env(&plan, Vantage::Host)?;
    plan.client_env_container = resolve_client_env(&plan, Vantage::Container)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::baseline_catalog;
    use crate::model::endpoint::{Endpoint, Rewrite};
    use crate::model::placement::Placement;
    use knobs::resolve_knob;

    fn default_selection() -> Selection {
        Selection::modules(["envoy", "postgres", "rustfs", "mlflow", "unity-catalog"])
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
            route: Default::default(),
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
            aliases: vec![],
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
    fn selecting_an_auth_provider_turns_gateway_auth_on() {
        // Picking the IdP without touching the gateway's knob must still wire it in; otherwise
        // it would run as an orphan that authenticates nothing.
        for selection in [
            Selection::modules(["envoy", "authelia", "mlflow"]),
            Selection {
                capabilities: vec![Role::AUTH.into()],
                ..Selection::modules(["envoy", "mlflow"])
            },
        ] {
            let p = plan_env(&selection, &baseline_catalog(), &PlanCtx::default())
                .expect("plan succeeds");
            let auth = p.gateway.auth.as_ref().expect("gateway auth on");
            assert_eq!(auth.cluster, "authelia");
        }
    }

    #[test]
    fn an_explicit_auth_off_override_wins_over_a_selected_provider() {
        let mut selection = Selection::modules(["envoy", "authelia", "mlflow"]);
        selection
            .knob_overrides
            .entry(ModuleId::from("envoy"))
            .or_default()
            .insert(ENVOY_AUTH_KNOB.into(), "false".into());
        let p =
            plan_env(&selection, &baseline_catalog(), &PlanCtx::default()).expect("plan succeeds");
        assert!(p.gateway.auth.is_none(), "explicit auth=false is honoured");
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
                    sensitive: false,
                    secret_alias: None,
                    at_root: false,
                    preserve: false,
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
                sensitive: false,
                secret_alias: None,
                at_root: false,
                preserve: false,
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
    fn endpoints_on_different_ports_of_one_service_get_their_own_clusters() {
        // Two API endpoints on the same service but different ports: the first port keeps the
        // service-named cluster, the second gets `<service>_<endpoint>`, and each route
        // forwards to the cluster for its own port.
        let m = module_with(
            "svc",
            vec![
                ep("a", 8080, RouteIntent::Api),
                ep("b", 9090, RouteIntent::Api),
            ],
            Default::default(),
        );
        let p = plan_env(
            &Selection::modules(["svc"]),
            &Catalog::from_modules([arc(m)]),
            &PlanCtx::default(),
        )
        .expect("plan should succeed");
        let port_of = |name: &str| {
            p.gateway
                .clusters
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no cluster {name}"))
                .port
        };
        assert_eq!(port_of("svc"), 8080);
        assert_eq!(port_of("svc_b"), 9090);
        assert_eq!(route_for(&p, "/a").cluster, "svc");
        assert_eq!(route_for(&p, "/b").cluster, "svc_b");
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

        // MLflow OTLP ingest is mounted Databricks-style and rewritten onto the path MLflow
        // serves it at under its base (`/mlflow/v1/traces`).
        let otel = route_for(&p, "/api/2.0/otel");
        assert_eq!(otel.rewrite.as_deref(), Some("/mlflow"));

        // The AI Gateway is mounted at `/ai-gateway`, rewritten onto MLflow's gateway router,
        // and routed for streaming with caller credentials dropped.
        let ai = route_for(&p, "/ai-gateway");
        assert_eq!(ai.cluster, "mlflow");
        assert_eq!(ai.rewrite.as_deref(), Some("/mlflow/gateway"));
        assert!(ai.options.streaming && ai.options.strip_client_credentials);

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

        // Default (RustFS): db healthy + rustfs-init completed.
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
                .contains("rustfs-init:\n        condition: service_completed_successfully")
        );
        assert!(!mlflow.fragment.contains("azurite-init"));

        // Azurite-preferred: the object-store gate switches to azurite-init.
        let mut preference = BTreeMap::new();
        preference.insert(
            "object_store".to_string(),
            vec![ModuleId::from("azurite"), ModuleId::from("rustfs")],
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
        assert!(!mlflow_az.fragment.contains("rustfs-init"));
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
        // mlflow + its transitive requires (postgres, rustfs, envoy).
        for id in ["mlflow", "postgres", "rustfs", "envoy"] {
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
            Selection::modules(["unity-catalog", "mlflow", "rustfs", "postgres", "envoy"]);
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
            key: "serve_ui".into(),
            title: None,
            kind: KnobKind::Bool,
            default: Some("true".into()),
            required: false,
            help: None,
            aliases: vec!["SERVE_UI".into()],
        };

        // No override → the declared default.
        assert_eq!(
            resolve_knob(&module, &knob, None).unwrap().as_deref(),
            Some("true")
        );

        // An override wins over the default.
        let overrides = BTreeMap::from([("serve_ui".to_string(), "false".to_string())]);
        assert_eq!(
            resolve_knob(&module, &knob, Some(&overrides))
                .unwrap()
                .as_deref(),
            Some("false")
        );

        // A legacy alias also wins.
        let legacy = BTreeMap::from([("SERVE_UI".to_string(), "false".to_string())]);
        assert_eq!(
            resolve_knob(&module, &knob, Some(&legacy))
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
            key: "k".into(),
            title: None,
            kind,
            default: Some(default.into()),
            required: false,
            help: None,
            aliases: vec![],
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
                key: "k".into(),
                title: None,
                kind: kind.clone(),
                default: Some(value.into()),
                required: false,
                help: None,
                aliases: vec![],
            };
            assert_eq!(
                resolve_knob(&module, &k, None).unwrap_err(),
                PlanError::InvalidKnobValue {
                    module: module.clone(),
                    key: "k".into(),
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
            key: "token".into(),
            title: None,
            kind: KnobKind::String,
            default: None,
            required: false,
            help: None,
            aliases: vec![],
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
                key: "token".into(),
            }
        );
    }
}
