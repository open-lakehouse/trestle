//! What a *module* is ([`Module`]): the unit of selection and composition that a
//! catalog is made of, and that the planner resolves into an environment.
//!
//! A module is the reusable building block both consumers already have under
//! different names — trestle's *components* (a `template.yaml` plus a compose
//! fragment) and hydrofoil's `env-modules` registry entries. This type is their
//! common denominator. A module:
//!
//! - declares the [`ServiceSpec`]s it contributes to the topology (often more than
//!   one — Postgres plus its init job, an object store plus its bucket-init);
//! - declares non-routing [`Provides`] (databases and buckets it needs, ports,
//!   free-form extras) — but **never** its own gateway routes or clusters, which
//!   are the planner's to assign across all modules at once;
//! - declares its dependency edges ([`requires`](Module::requires)) and any
//!   [`conflicts_with`](Module::conflicts_with);
//! - exposes optional config [`Knob`]s (which can drive a generated UI); and
//! - carries a [`RenderSpec`] describing how it produces its
//!   [`RenderOutput`](crate::RenderOutput) — a MiniJinja template this crate renders
//!   against the typed [`Connection`](crate::Connection)s so a fragment can read plan-resolved
//!   values and branch on the chosen credential flavour.
//!
//! The module declares *intent and ingredients*; the planner decides *routing and
//! wiring*. Keeping routes out of the module is the whole point — only the planner,
//! seeing every module at once, can assign prefixes that don't collide (see
//! [`plan`](crate::plan) and [`RoutePlan`](crate::RoutePlan)).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::connection::{ConnectionField, ConnectionTemplate};
use crate::render::{InjectedEnv, RenderFile, RenderOutput};
use crate::role::ServiceSpec;

/// A module's stable identifier within a catalog (e.g. `"mlflow"`).
///
/// An **open set** — a string newtype, not an enum — exactly like
/// [`Role`](crate::Role): a new module drops into a catalog as *data*, with no
/// change to this crate. Conventionally lower-kebab-case to match the directory
/// names catalogs are discovered from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModuleId(pub String);

impl ModuleId {
    /// Construct a module id from anything string-like.
    pub fn new(s: impl Into<String>) -> Self {
        ModuleId(s.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModuleId {
    fn from(s: &str) -> Self {
        ModuleId(s.to_string())
    }
}

impl From<String> for ModuleId {
    fn from(s: String) -> Self {
        ModuleId(s)
    }
}

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The non-routing contributions a module makes to the assembled environment.
///
/// This deliberately **omits gateway routes and clusters**: a module declares only
/// its endpoints' [`RouteIntent`](crate::RouteIntent) (on its [`ServiceSpec`]s) and
/// lets the planner assign the actual prefixes/rewrites/listeners. Clusters are
/// likewise derived by the planner from each service's
/// [`Placement`](crate::Placement) and endpoint port — never authored here. What
/// remains is genuinely declarative module data: the named resources it needs and
/// any free-form extras.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provides {
    /// Resource kinds this module *provisions* for other modules, keyed by kind
    /// (e.g. `"relational_db"`, `"object_store"`). A consuming module declares a
    /// [`ResourceDemand`] for a kind; the planner finds the provider here, ensures it
    /// is deployed, provisions the named resource, and resolves the provider's
    /// [`ConnectionTemplate`] into a typed [`Connection`](crate::Connection) the consumer
    /// binds back (see [`ResourceDemand`] and [`ConnectionBinding`]).
    #[serde(default)]
    pub resource_kinds: BTreeMap<String, ConnectionTemplate>,
    /// Named, defaulted ports this module exposes (a knob-like surface for ports
    /// without forcing every one through [`Knob`]).
    #[serde(default)]
    pub ports: Vec<PortDecl>,
    /// Environment variables this module contributes to the materialized stack
    /// (compose-`${VAR}`-style values are preserved verbatim). The planner merges
    /// these across modules in dependency order — a later module overrides an
    /// earlier one for the same key — into the environment it renders to `.env`.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    /// Free-form per-module data, for options that don't warrant a dedicated typed
    /// field. The planner namespaces each entry by the contributing module id when
    /// it surfaces them to a consumer.
    #[serde(default)]
    pub extras: BTreeMap<String, String>,
}

/// A named, defaulted port a module exposes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDecl {
    /// The port's logical name (e.g. `"http"`, `"s3"`).
    pub name: String,
    /// The default port number.
    pub default: u16,
    /// Whether this port is only reachable inside the compose network (never
    /// host-published).
    #[serde(default)]
    pub internal_only: bool,
}

/// A resource a module needs: a `(role, name)` the planner must ensure exists, plus how
/// to bind the resolved [`Connection`](crate::Connection) back into this module's env.
///
/// [`resource`](ResourceDemand::resource) names an abstract *role* (e.g.
/// `"object_store"`), not a specific implementation — the planner chooses which
/// registered provider satisfies it (by [`provider`](ResourceDemand::provider) pin,
/// `PlanCtx` preference, uniqueness, or catalog default), deploys it if absent,
/// provisions the named resource, resolves the provider's
/// [`ConnectionTemplate`](crate::ConnectionTemplate), and binds each
/// [`ConnectionBinding`] field into this module's environment. Naming the role (not the
/// implementation) is what lets one consumer run on, say, SeaweedFS in one environment and
/// Azurite in another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDemand {
    /// The resource *role* needed (e.g. `"object_store"`, `"relational_db"`) — matched
    /// against providers' [`Provides::resource_kinds`].
    pub resource: String,
    /// The concrete resource name to provision (e.g. `"unitycatalog"`, `"unity"`).
    pub name: String,
    /// Pin a specific provider module for this demand, overriding preference/default.
    /// The escape hatch for when a consumer truly needs one backend; normally `None`
    /// and the planner chooses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ModuleId>,
    /// How the resolved connection's typed fields map into the demanding module's env so
    /// it can discover the resource at run time.
    #[serde(default)]
    pub bind: ConnectionBinding,
}

/// How a demand maps the resolved [`Connection`](crate::Connection)'s typed fields into
/// the consuming module's [`InjectedEnv`].
///
/// Each `(field, key)` pair binds one [`ConnectionField`] to the env-var `key` the
/// consumer's fragment/files read as `${KEY}` (e.g. `(ConnectionField::Url,
/// "UC_DATABASE_URL")`). A field the chosen provider's connection variant does not carry
/// is a [`PlanError::UnboundConnectionField`](crate::PlanError::UnboundConnectionField).
/// The default (empty) binding injects nothing — the demand still provisions the resource.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionBinding {
    /// The `(typed field, env-var name)` pairs to bind, in declaration order.
    pub bind: Vec<(ConnectionField, String)>,
}

/// The kind of value a [`Knob`] holds — the bridge to a generated config UI.
///
/// Each variant maps 1:1 to a JSON Schema primitive, so a catalog can emit a schema
/// for a module's knobs and a UI can render the right control. (Schema emission is a
/// later phase; the variants are defined now so knobs are typed from the start.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum KnobKind {
    /// A free-text string.
    String,
    /// A boolean toggle.
    Bool,
    /// A choice from a fixed set of string options.
    Enum {
        /// The allowed values.
        options: Vec<String>,
    },
    /// An integer, optionally bounded.
    Integer {
        /// Inclusive minimum, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        /// Inclusive maximum, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// A TCP port (an integer constrained to `1..=65535`, rendered as a port
    /// control by a UI).
    Port,
}

/// One user-tunable configuration value a module exposes.
///
/// A knob's [`key`](Knob::key) is the [`InjectedEnv`] variable name its value lands
/// under (`${KEY}`), so the module's fragment and mounted files consume it through
/// the one uniform substitution mechanism. The remaining fields are UI metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knob {
    /// The injected-env variable name this knob feeds (e.g. `"MLFLOW_PORT"`).
    pub key: String,
    /// A short human-readable title for a UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The kind/shape of the value.
    pub kind: KnobKind,
    /// The default value (as a string; coerced per [`kind`](Knob::kind) by a
    /// consumer). Used when the user does not override the knob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Whether the user must supply a value (no usable default).
    #[serde(default)]
    pub required: bool,
    /// Optional longer help text for a UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// How a module produces its [`RenderOutput`]: a MiniJinja template the crate renders
/// against the typed [`RenderCtx`].
///
/// The fragment is rendered in-crate with MiniJinja against the injected `env`, the module's
/// typed `connections`, the resolved `dependencies`, and the provisioned `objects` — so it
/// can read plan-resolved values (`{{ env.DATA_ROOT }}`, `{{ connections.object_store.0.uri }}`)
/// and branch on a resolved [`Connection`](crate::Connection) (e.g. emit S3 keys vs an Azure
/// connection string for whichever object-store backend the planner chose). Plan-time values
/// are rendered *concrete*; any literal compose `${VAR}` left in the source passes through
/// untouched (MiniJinja interprets only `{{ }}`/`{% %}`), so a fragment can still defer a value
/// to a container's own runtime env where that is genuinely a container contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RenderSpec {
    /// MiniJinja template source the crate renders against the [`RenderCtx`].
    Template {
        /// The fragment's MiniJinja template source.
        fragment: String,
        /// MiniJinja template sources for files to write and mount.
        #[serde(default)]
        files: Vec<RenderFile>,
    },
}

impl RenderSpec {
    /// Produce the [`RenderOutput`] for this spec given the planner's render context.
    ///
    /// The source is rendered with MiniJinja against the full [`RenderCtx`] — so a fragment
    /// can branch on a demand's typed [`Connection`](crate::Connection)
    /// (e.g. `{% if c.credential.flavour == "s3" %}`) and read plan-resolved values directly.
    ///
    /// Returns [`RenderError`] when the template fails to compile or render — e.g. a
    /// malformed fragment, or a reference to a field absent from the context (a module
    /// authored as an on-disk `module.yaml` is external input, so this is a recoverable
    /// error the planner surfaces, not a panic).
    pub fn render(&self, ctx: &RenderCtx<'_>) -> Result<RenderOutput, RenderError> {
        match self {
            RenderSpec::Template { fragment, files } => {
                let mut env = minijinja::Environment::new();
                Ok(RenderOutput {
                    fragment: render_template(&mut env, fragment, ctx)?,
                    files: files
                        .iter()
                        .map(|f| {
                            Ok(RenderFile {
                                path: render_template(&mut env, &f.path, ctx)?,
                                contents: render_template(&mut env, &f.contents, ctx)?,
                                alias: f
                                    .alias
                                    .as_deref()
                                    .map(|a| render_template(&mut env, a, ctx))
                                    .transpose()?,
                            })
                        })
                        .collect::<Result<_, RenderError>>()?,
                })
            }
        }
    }
}

/// A [`Template`](RenderSpec::Template) fragment failed to compile or render.
///
/// Carries the templating engine's message. Surfaced (not panicked) because a module can
/// be authored as an external on-disk `module.yaml`, so a bad template is recoverable
/// input, not an internal invariant violation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("rendering module template failed: {0}")]
pub struct RenderError(pub String);

/// A compose `depends_on` condition — *what* readiness state of a dependency a service
/// waits for before it starts.
///
/// These are the three Compose-spec long-form conditions. The string each renders to (its
/// serde value) is exactly the compose token, so a template emits
/// `condition: {{ dep.condition }}` directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependsCondition {
    /// `service_started` — the dependency's container has started (the weakest gate).
    ServiceStarted,
    /// `service_healthy` — the dependency reports healthy (its healthcheck passes).
    ServiceHealthy,
    /// `service_completed_successfully` — the dependency ran to a successful exit (a
    /// one-shot init job).
    ServiceCompletedSuccessfully,
}

impl DependsCondition {
    /// The compose token for this condition (e.g. `"service_healthy"`).
    pub fn as_str(self) -> &'static str {
        match self {
            DependsCondition::ServiceStarted => "service_started",
            DependsCondition::ServiceHealthy => "service_healthy",
            DependsCondition::ServiceCompletedSuccessfully => "service_completed_successfully",
        }
    }

    /// Parse a compose condition token, returning `None` for an unrecognized value.
    pub fn parse(s: &str) -> Option<DependsCondition> {
        match s {
            "service_started" => Some(DependsCondition::ServiceStarted),
            "service_healthy" => Some(DependsCondition::ServiceHealthy),
            "service_completed_successfully" => {
                Some(DependsCondition::ServiceCompletedSuccessfully)
            }
            _ => None,
        }
    }
}

/// One resolved `depends_on` gate a module's render should emit: a compose service to wait
/// for and the [`DependsCondition`] to wait for it to reach.
///
/// The planner produces these from a consumer's resource demands — for each demand it reads
/// the *chosen* provider's [`DEP_GATE_EXTRA`](crate::catalog::baseline::DEP_GATE_EXTRA) and
/// resolves it into a `DepGate` — and hands them to the render via
/// [`RenderCtx::dependencies`]. A template renders its whole `depends_on` block by iterating
/// them (`{% for dep in dependencies %}{{ dep.service }}: {condition: {{ dep.condition }}}`),
/// so it never hard-codes which backend's service it waits on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepGate {
    /// The compose service name to depend on (e.g. `"db"`, `"seaweedfs-init"`).
    pub service: String,
    /// The condition to wait for.
    pub condition: DependsCondition,
}

/// The context a module's render reads: the planner-decided [`InjectedEnv`], the typed
/// [`Connection`](crate::Connection)s resolved for the module's demands (grouped by role),
/// and the resolved [`DepGate`]s its `depends_on` block should wait on.
///
/// A [`Template`](RenderSpec::Template) render gets the whole context as MiniJinja globals:
/// `env` (a `{KEY: value}` map, so `{{ env.UC_DATABASE_URL }}` works), `connections` (a
/// `{role: [connection, …]}` map a template can branch on — e.g.
/// `{% set obj = connections.object_store.0 %}{% if obj.credential.flavour == "s3" %}`), and
/// `dependencies` (the `[{service, condition}, …]` list a template iterates to write its
/// `depends_on` block — see [`DepGate`]); and `objects` (the resource *names* this module's
/// own role provisions, for a provider's init block to iterate).
#[derive(Clone, Debug, Serialize)]
pub struct RenderCtx<'a> {
    /// The planner-decided environment-variable substitutions.
    pub env: &'a InjectedEnv,
    /// The typed connections resolved for the module's demands, keyed by resource role.
    /// More than one connection per role is possible (a module with two same-role demands).
    /// For a *provider* module, this also carries its own role's connection (resolved for
    /// each name it provisions) so its fragment can read e.g.
    /// `connections.object_store.0.credential.connection_string` instead of a `${VAR}`.
    pub connections: BTreeMap<String, Vec<crate::connection::Connection>>,
    /// The resolved `depends_on` gates the module's render should emit, in dependency
    /// (demand) order. Empty for a module with no demands that gate startup.
    #[serde(default)]
    pub dependencies: Vec<DepGate>,
    /// The resource *names* this module provisions for its own provided role (e.g. the
    /// buckets/containers an object-store provider must create), deduplicated in dependency
    /// order. A provider's init block iterates these (`{% for o in objects %}`) instead of
    /// the planner splicing pre-formatted shell lines through a `${VAR}` placeholder. Empty
    /// for a non-provider module.
    #[serde(default)]
    pub objects: Vec<String>,
    /// The gateway's `host:container` port mappings to publish, populated only for the
    /// gateway module: the shared listener plus one entry per dedicated listener the planner
    /// allocated (e.g. an object store's). The gateway fragment iterates these to render its
    /// compose `ports:` — so dedicated listeners are reachable from the host without the
    /// fragment hard-coding a port list. Empty for every non-gateway module.
    #[serde(default)]
    pub published_ports: Vec<PortMapping>,
}

/// A `host:container` port mapping a module publishes (currently only the gateway, for its
/// listeners). Serialized as `{host, container}` so a template renders `"{{ p.host }}:{{ p.container }}"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    /// The host-published port (compose `ports:` left side).
    pub host: u16,
    /// The in-container port the listener binds (compose `ports:` right side).
    pub container: u16,
}

impl<'a> RenderCtx<'a> {
    /// A context carrying just an [`InjectedEnv`] and no connections, dependencies, or
    /// objects — the shape a module with no resource demands renders against.
    pub fn from_env(env: &'a InjectedEnv) -> Self {
        RenderCtx {
            env,
            connections: BTreeMap::new(),
            dependencies: Vec::new(),
            objects: Vec::new(),
            published_ports: Vec::new(),
        }
    }
}

/// Render one MiniJinja template string against the [`RenderCtx`], reusing `env`.
///
/// Pure and in-memory (no `loader`, no filesystem). A compile or render failure (malformed
/// source, or a reference to a field absent from `ctx`) is returned as a [`RenderError`]
/// rather than panicking — a `Template` can come from an external on-disk manifest.
/// `render_named_str` compiles and renders in one call, so no template name is registered.
fn render_template(
    env: &mut minijinja::Environment<'_>,
    source: &str,
    ctx: &RenderCtx<'_>,
) -> Result<String, RenderError> {
    env.render_named_str("fragment", source, ctx)
        .map_err(|e| RenderError(e.to_string()))
}

impl Default for RenderSpec {
    fn default() -> Self {
        RenderSpec::Template {
            fragment: String::new(),
            files: Vec::new(),
        }
    }
}

/// A reusable building block in a catalog: the services it contributes, what it
/// needs, its dependencies, its config knobs, and how it renders.
///
/// Selection picks a set of modules (directly or via capabilities); the planner
/// resolves their dependency graph and assigns routing. See [`plan`](crate::plan).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// The module's stable id within its catalog.
    pub id: ModuleId,
    /// A human-readable name for a wizard/UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// A one-line summary for a wizard/UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The wizard category this module slots into (e.g. `"ml"`, `"storage"`,
    /// `"catalog"`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The capability this module provides, if any (e.g. `"experiment_tracking"`).
    /// Used to build the capability → module index for capability-based selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_of: Option<String>,
    /// Other modules (by id) this one requires; pulled in transitively and ordered
    /// before this module by the resolver.
    #[serde(default)]
    pub requires: Vec<ModuleId>,
    /// Modules (by id) this one cannot coexist with; the planner rejects a
    /// selection containing both.
    #[serde(default)]
    pub conflicts_with: Vec<ModuleId>,
    /// Resources this module needs a provider to vend (databases, buckets, …). Unlike
    /// [`requires`](Module::requires) — a dependency on a *specific* module — a
    /// [`ResourceDemand`] names a resource *kind*; the planner finds (and auto-deploys)
    /// a provider for it, provisions the named resource, and injects its coordinates
    /// back into this module.
    #[serde(default)]
    pub needs: Vec<ResourceDemand>,
    /// The topology services this module contributes (often more than one).
    #[serde(default)]
    pub services: Vec<ServiceSpec>,
    /// Non-routing declarative contributions (resource kinds it provisions, ports,
    /// env vars, extras).
    #[serde(default)]
    pub provides: Provides,
    /// User-tunable config knobs this module exposes.
    #[serde(default)]
    pub knobs: Vec<Knob>,
    /// How this module produces its compose fragment and mountable files.
    #[serde(default)]
    pub render: RenderSpec,
}

impl Module {
    /// Look up one of this module's services by `name`.
    pub fn service(&self, name: &str) -> Option<&ServiceSpec> {
        self.services.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> InjectedEnv {
        let mut e = InjectedEnv::new();
        for (k, v) in pairs {
            e.set(*k, *v);
        }
        e
    }

    #[test]
    fn template_render_reads_env_and_branches_on_connection_flavour() {
        use crate::connection::{Connection, ObjectStoreCredential};

        // A Template fragment reads `${...}`-free MiniJinja: `env.*` for injected values and
        // `connections.*` to branch on the chosen credential flavour.
        let spec = RenderSpec::Template {
            fragment: "url: {{ env.DB_URL }}\n\
                       {%- set o = connections.object_store.0 %}\n\
                       {%- if o.credential.flavour == \"s3\" %}\n\
                       key: {{ o.credential.access_key_id }}\n\
                       {%- else %}\n\
                       conn: {{ o.credential.connection_string }}\n\
                       {%- endif %}\n"
                .into(),
            files: vec![],
        };

        let s3 = Connection::ObjectStore {
            uri: "s3://b".into(),
            bucket: "b".into(),
            endpoint: "http://store:1".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "AKIA".into(),
                secret_access_key: "shh".into(),
                region: "us-east-1".into(),
            },
        };
        let mut connections = BTreeMap::new();
        connections.insert("object_store".to_string(), vec![s3]);
        let env = env(&[("DB_URL", "postgresql://db/x")]);
        let out = spec
            .render(&RenderCtx {
                env: &env,
                connections,
                dependencies: Vec::new(),
                objects: Vec::new(),
                published_ports: Vec::new(),
            })
            .expect("template renders");
        assert!(out.fragment.contains("url: postgresql://db/x"));
        assert!(
            out.fragment.contains("key: AKIA"),
            "S3 branch taken: {out:?}"
        );
        assert!(
            !out.fragment.contains("conn:"),
            "Azure branch skipped: {out:?}"
        );
    }

    #[test]
    fn template_iterates_typed_dependencies_into_depends_on() {
        // A template renders its whole `depends_on` block by iterating the typed
        // `dependencies` the planner resolved — service + condition, no env-var strings.
        let spec = RenderSpec::Template {
            fragment: "depends_on:\n\
                       {%- for dep in dependencies %}\n\
                       \x20 {{ dep.service }}:\n\
                       \x20   condition: {{ dep.condition }}\n\
                       {%- endfor %}\n"
                .into(),
            files: vec![],
        };
        let ctx = RenderCtx {
            env: &InjectedEnv::new(),
            connections: BTreeMap::new(),
            dependencies: vec![
                DepGate {
                    service: "db".into(),
                    condition: DependsCondition::ServiceHealthy,
                },
                DepGate {
                    service: "seaweedfs-init".into(),
                    condition: DependsCondition::ServiceCompletedSuccessfully,
                },
            ],
            objects: Vec::new(),
            published_ports: Vec::new(),
        };
        let out = spec.render(&ctx).expect("template renders");
        // The serde value of each condition is the exact compose token.
        assert!(out.fragment.contains("db:\n    condition: service_healthy"));
        assert!(
            out.fragment
                .contains("seaweedfs-init:\n    condition: service_completed_successfully")
        );
    }

    #[test]
    fn malformed_template_is_a_recoverable_error_not_a_panic() {
        // A `Template` can come from an external on-disk manifest, so a bad fragment is a
        // returned `RenderError`, never a panic.
        let bad_syntax = RenderSpec::Template {
            fragment: "{% if %}".into(), // unparsable
            files: vec![],
        };
        assert!(
            bad_syntax
                .render(&RenderCtx::from_env(&InjectedEnv::new()))
                .is_err(),
            "malformed template syntax must return Err"
        );

        // Referencing a context field that isn't there (no object_store connection) also
        // errors rather than panicking.
        let missing_field = RenderSpec::Template {
            fragment: "{{ connections.object_store.0.uri }}".into(),
            files: vec![],
        };
        assert!(
            missing_field
                .render(&RenderCtx::from_env(&InjectedEnv::new()))
                .is_err(),
            "indexing an absent connection must return Err"
        );
    }
}
