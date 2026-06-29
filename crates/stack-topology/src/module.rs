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
//!   [`RenderOutput`](crate::RenderOutput) — either as static text this crate
//!   substitutes `${VAR}` into purely, or as a MiniJinja template this crate renders
//!   against the typed [`Connection`](crate::Connection)s so a fragment can branch on the
//!   chosen credential flavour.
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

/// A module's stable identifier within a catalog (e.g. `"local-stack-mlflow"`).
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

/// How a module produces its [`RenderOutput`].
///
/// Both variants yield the same `RenderOutput` (a compose fragment plus mountable
/// files); they differ only in *how much* the fragment needs to know. The crate is pure
/// in both: [`Static`](RenderSpec::Static) is flat `${VAR}` substitution; the richer
/// [`Template`](RenderSpec::Template) is rendered in-crate with MiniJinja against the
/// typed [`RenderCtx`] so a fragment can branch on a resolved
/// [`Connection`](crate::Connection) — e.g. emit S3 keys vs an Azure connection string
/// for whichever object-store backend the planner chose. Reach for `Template` only when a
/// fragment genuinely must branch; `Static` is the default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RenderSpec {
    /// Literal text with `${VAR}` holes the planner fills from an [`InjectedEnv`].
    /// This crate substitutes them itself (no templating engine).
    Static {
        /// The compose fragment, with `${VAR}` placeholders.
        fragment: String,
        /// Files to write and mount, with `${VAR}` placeholders in their contents.
        #[serde(default)]
        files: Vec<RenderFile>,
    },
    /// MiniJinja template source the crate renders against the [`RenderCtx`] (the injected
    /// `env` plus the module's typed `connections`). `${VAR}` compose refs in the source
    /// pass through untouched (MiniJinja interprets only `{{ }}`/`{% %}`), so a template
    /// freely mixes plan-time branching with run-time compose substitution.
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
    /// For [`Static`](RenderSpec::Static), `${VAR}` placeholders are substituted from the
    /// context's [`env`](RenderCtx::env) (no templating engine). For
    /// [`Template`](RenderSpec::Template), the source is rendered with MiniJinja against the
    /// full [`RenderCtx`] — so a fragment can branch on a demand's typed
    /// [`Connection`](crate::Connection) (e.g. `{% if c.credential.flavour == "s3" %}`),
    /// which flat `${VAR}` substitution cannot express.
    pub fn render(&self, ctx: &RenderCtx<'_>) -> RenderOutput {
        match self {
            RenderSpec::Static { fragment, files } => RenderOutput {
                fragment: substitute(fragment, ctx.env),
                files: files
                    .iter()
                    .map(|f| RenderFile {
                        path: substitute(&f.path, ctx.env),
                        contents: substitute(&f.contents, ctx.env),
                    })
                    .collect(),
            },
            RenderSpec::Template { fragment, files } => RenderOutput {
                fragment: render_template(fragment, ctx),
                files: files
                    .iter()
                    .map(|f| RenderFile {
                        path: render_template(&f.path, ctx),
                        contents: render_template(&f.contents, ctx),
                    })
                    .collect(),
            },
        }
    }
}

/// The context a module's render reads: the planner-decided [`InjectedEnv`] plus the typed
/// [`Connection`](crate::Connection)s resolved for the module's demands, grouped by role.
///
/// A [`Static`](RenderSpec::Static) render uses only [`env`](RenderCtx::env). A
/// [`Template`](RenderSpec::Template) render gets the whole context as MiniJinja globals:
/// `env` (a `{KEY: value}` map, so `{{ env.UC_DATABASE_URL }}` works) and `connections` (a
/// `{role: [connection, …]}` map a template can branch on — e.g.
/// `{% set obj = connections.object_store.0 %}{% if obj.credential.flavour == "s3" %}`).
#[derive(Clone, Debug, Serialize)]
pub struct RenderCtx<'a> {
    /// The planner-decided environment-variable substitutions.
    pub env: &'a InjectedEnv,
    /// The typed connections resolved for the module's demands, keyed by resource role.
    /// More than one connection per role is possible (a module with two same-role demands).
    pub connections: BTreeMap<String, Vec<crate::connection::Connection>>,
}

impl<'a> RenderCtx<'a> {
    /// A context carrying just an [`InjectedEnv`] and no connections — the shape a module
    /// with no resource demands renders against.
    pub fn from_env(env: &'a InjectedEnv) -> Self {
        RenderCtx {
            env,
            connections: BTreeMap::new(),
        }
    }
}

/// Render one MiniJinja template string against the [`RenderCtx`].
///
/// Pure and in-memory (no `loader`, no filesystem). A malformed template or a reference to
/// a missing field is a module-authoring bug, surfaced as a panic with the engine's message
/// — the same posture as the embedded Envoy template renderer.
fn render_template(source: &str, ctx: &RenderCtx<'_>) -> String {
    let mut env = minijinja::Environment::new();
    env.add_template("fragment", source)
        .expect("module template source is valid MiniJinja");
    env.get_template("fragment")
        .expect("the template was just added")
        .render(ctx)
        .expect("rendering a module template with a valid context cannot fail")
}

impl Default for RenderSpec {
    fn default() -> Self {
        RenderSpec::Static {
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

/// Substitute `${VAR}` (and `${VAR:-default}`) occurrences in `text` from `env`.
///
/// A pure, dependency-free pass — *not* a shell or a templating engine. It handles
/// exactly the compose-style forms the render contract uses:
///
/// - `${VAR}` → the value of `VAR`, or left **unexpanded** if `VAR` is not in `env`
///   (so compose itself can still resolve it at run time);
/// - `${VAR:-default}` → the value of `VAR`, or `default` if unset.
///
/// Leaving unknown bare `${VAR}` untouched is deliberate: the planner injects only
/// the values it decided; everything else stays a compose substitution the running
/// environment provides.
fn substitute(text: &str, env: &InjectedEnv) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        // Look for the start of a `${...}` expression.
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(close) = text[i + 2..].find('}') {
                let inner = &text[i + 2..i + 2 + close];
                out.push_str(&expand(inner, env));
                i = i + 2 + close + 1;
                continue;
            }
        }
        // Not a substitution start — copy this char verbatim. Index by char to stay
        // UTF-8 correct (the `$`/`{`/`}` checks above are all ASCII).
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Expand a single `${...}` body: either `VAR` or `VAR:-default`.
fn expand(inner: &str, env: &InjectedEnv) -> String {
    if let Some((var, default)) = inner.split_once(":-") {
        match env.get(var) {
            Some(v) => v.to_string(),
            None => default.to_string(),
        }
    } else {
        match env.get(inner) {
            Some(v) => v.to_string(),
            // Unknown bare var: leave the placeholder for compose to resolve.
            None => format!("${{{inner}}}"),
        }
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

    /// Render `spec` against an env-only context (no connections).
    fn render_env(spec: &RenderSpec, env: &InjectedEnv) -> RenderOutput {
        spec.render(&RenderCtx::from_env(env))
    }

    #[test]
    fn static_render_substitutes_known_vars_in_fragment_and_files() {
        let spec = RenderSpec::Static {
            fragment: "command: mlflow --static-prefix ${BASE_PATH}\n".into(),
            files: vec![RenderFile {
                path: "config/${NAME}.yaml".into(),
                contents: "base_path: ${BASE_PATH}\n".into(),
            }],
        };
        let out = render_env(&spec, &env(&[("BASE_PATH", "/mlflow"), ("NAME", "mlflow")]));
        assert_eq!(out.fragment, "command: mlflow --static-prefix /mlflow\n");
        assert_eq!(out.files[0].path, "config/mlflow.yaml");
        assert_eq!(out.files[0].contents, "base_path: /mlflow\n");
    }

    #[test]
    fn unknown_bare_var_is_left_for_compose() {
        let spec = RenderSpec::Static {
            fragment: "user: ${POSTGRES_USER}\n".into(),
            files: vec![],
        };
        // Not injected → placeholder preserved verbatim.
        let out = render_env(&spec, &InjectedEnv::new());
        assert_eq!(out.fragment, "user: ${POSTGRES_USER}\n");
    }

    #[test]
    fn default_form_uses_default_when_unset_and_value_when_set() {
        let spec = RenderSpec::Static {
            fragment: "port: ${ENVOY_PORT:-9080}\n".into(),
            files: vec![],
        };
        assert_eq!(
            render_env(&spec, &InjectedEnv::new()).fragment,
            "port: 9080\n",
            "default applies when unset"
        );
        assert_eq!(
            render_env(&spec, &env(&[("ENVOY_PORT", "8080")])).fragment,
            "port: 8080\n",
            "injected value wins over default"
        );
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
        let out = spec.render(&RenderCtx {
            env: &env,
            connections,
        });
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
    fn substitution_is_utf8_safe() {
        let spec = RenderSpec::Static {
            fragment: "# café ${X} ☕\n".into(),
            files: vec![],
        };
        assert_eq!(
            render_env(&spec, &env(&[("X", "ok")])).fragment,
            "# café ok ☕\n"
        );
    }
}
