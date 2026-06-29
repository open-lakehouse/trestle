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
//!   [`RenderOutput`](crate::RenderOutput) — either as static text this crate can
//!   substitute into purely, or as opaque template source the consumer renders.
//!
//! The module declares *intent and ingredients*; the planner decides *routing and
//! wiring*. Keeping routes out of the module is the whole point — only the planner,
//! seeing every module at once, can assign prefixes that don't collide (see
//! [`plan`](crate::plan) and [`RoutePlan`](crate::RoutePlan)).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    /// (e.g. `"postgres_database"`, `"s3_bucket"`). A consuming module declares a
    /// [`ResourceDemand`] for a kind; the planner finds the provider here, ensures it
    /// is deployed, provisions the named resource, and renders the provider's
    /// coordinate templates back into the consumer (see [`ResourceProvider`]).
    #[serde(default)]
    pub resource_kinds: BTreeMap<String, ResourceProvider>,
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

/// How a provider module renders the runtime *coordinates* of a resource it
/// provisions, so a consumer can discover it.
///
/// A coordinate is a named, renderable fact about a provisioned resource — a
/// connection URL, the bucket name, an endpoint. Each template may use the
/// placeholder `{name}` for the concrete resource name (substituted by the planner at
/// plan time) and `${VAR}` compose-style refs (left untouched, resolved at run time by
/// compose). For example a relational-DB provider might offer a `"url"` coordinate
/// `postgresql://${POSTGRES_USER:-postgres}@db:5432/{name}`, and an object store a
/// `"bucket"` coordinate `{name}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceProvider {
    /// The named coordinate templates a consumer can request, keyed by coordinate
    /// name (e.g. `"url"`, `"bucket"`, `"endpoint"`).
    #[serde(default)]
    pub coordinates: BTreeMap<String, String>,
}

impl ResourceProvider {
    /// The template for a named coordinate, if this provider offers it.
    pub fn coordinate(&self, name: &str) -> Option<&str> {
        self.coordinates.get(name).map(String::as_str)
    }
}

/// A resource a module needs from a provider: a `(kind, name)` the planner must
/// ensure exists, plus where to inject the resolved coordinates back.
///
/// The planner resolves `resource` (the kind) to a provider module via the catalog's
/// resource index, deploys that provider if it isn't already selected, provisions the
/// named resource, and renders each [`Injection`]'s coordinate into this module's
/// environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDemand {
    /// The resource kind needed (e.g. `"postgres_database"`, `"s3_bucket"`) — matched
    /// against providers' [`Provides::resource_kinds`].
    pub resource: String,
    /// The concrete resource name to provision (e.g. `"unitycatalog"`, `"unity"`).
    pub name: String,
    /// The coordinates to inject back into the demanding module's environment so it
    /// can discover the resource at run time.
    #[serde(default)]
    pub inject: Vec<Injection>,
}

/// One coordinate value the planner injects back into the demanding module's
/// [`InjectedEnv`] under [`key`](Injection::key), sourced from the provider's named
/// [`coordinate`](ResourceProvider::coordinate).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Injection {
    /// The env-var name the resolved value lands under in the consumer (e.g.
    /// `"UC_DATABASE_URL"`); the consumer's fragment/files read it as `${KEY}`.
    pub key: String,
    /// Which of the provider's named coordinates to render (e.g. `"url"`, `"bucket"`).
    pub coordinate: String,
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
/// files); they differ only in *who* does the rendering. The crate stays pure: it
/// performs simple `${VAR}` substitution for [`Static`](RenderSpec::Static), and for
/// [`Template`](RenderSpec::Template) it only *carries* the source — a consumer that
/// owns a templating engine (trestle's MiniJinja) renders it.
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
    /// Opaque template source the crate stores but never executes; the consumer
    /// renders it with its own engine and produces the `RenderOutput`.
    Template {
        /// The fragment's template source (engine-specific; not interpreted here).
        fragment: String,
        /// Template sources for files to write and mount.
        #[serde(default)]
        files: Vec<RenderFile>,
    },
}

impl RenderSpec {
    /// Produce the [`RenderOutput`] for this spec given the planner's
    /// [`InjectedEnv`].
    ///
    /// For [`Static`](RenderSpec::Static), `${VAR}` placeholders are substituted
    /// here. For [`Template`](RenderSpec::Template), the source is returned verbatim
    /// (placeholders, if any, are the consumer's engine to resolve) — this crate
    /// never interprets template syntax.
    pub fn render(&self, env: &InjectedEnv) -> RenderOutput {
        match self {
            RenderSpec::Static { fragment, files } => RenderOutput {
                fragment: substitute(fragment, env),
                files: files
                    .iter()
                    .map(|f| RenderFile {
                        path: substitute(&f.path, env),
                        contents: substitute(&f.contents, env),
                    })
                    .collect(),
            },
            RenderSpec::Template { fragment, files } => RenderOutput {
                fragment: fragment.clone(),
                files: files.clone(),
            },
        }
    }
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

    #[test]
    fn static_render_substitutes_known_vars_in_fragment_and_files() {
        let spec = RenderSpec::Static {
            fragment: "command: mlflow --static-prefix ${BASE_PATH}\n".into(),
            files: vec![RenderFile {
                path: "config/${NAME}.yaml".into(),
                contents: "base_path: ${BASE_PATH}\n".into(),
            }],
        };
        let out = spec.render(&env(&[("BASE_PATH", "/mlflow"), ("NAME", "mlflow")]));
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
        let out = spec.render(&InjectedEnv::new());
        assert_eq!(out.fragment, "user: ${POSTGRES_USER}\n");
    }

    #[test]
    fn default_form_uses_default_when_unset_and_value_when_set() {
        let spec = RenderSpec::Static {
            fragment: "port: ${ENVOY_PORT:-9080}\n".into(),
            files: vec![],
        };
        assert_eq!(
            spec.render(&InjectedEnv::new()).fragment,
            "port: 9080\n",
            "default applies when unset"
        );
        assert_eq!(
            spec.render(&env(&[("ENVOY_PORT", "8080")])).fragment,
            "port: 8080\n",
            "injected value wins over default"
        );
    }

    #[test]
    fn template_render_passes_source_through_untouched() {
        let spec = RenderSpec::Template {
            fragment: "name: {{ project }}\nbase: ${BASE_PATH}\n".into(),
            files: vec![],
        };
        // Neither MiniJinja `{{ }}` nor `${ }` is interpreted here.
        let out = spec.render(&env(&[("BASE_PATH", "/mlflow")]));
        assert_eq!(out.fragment, "name: {{ project }}\nbase: ${BASE_PATH}\n");
    }

    #[test]
    fn substitution_is_utf8_safe() {
        let spec = RenderSpec::Static {
            fragment: "# café ${X} ☕\n".into(),
            files: vec![],
        };
        assert_eq!(spec.render(&env(&[("X", "ok")])).fragment, "# café ok ☕\n");
    }
}
