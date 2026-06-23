//! Schema for `template.yaml` (top-level templates and components).
//!
//! Templates come in two flavours and share one manifest type:
//! - **Base templates** (e.g. `_base/lakehouse/`) declare an `always:` baseline and
//!   user-facing `categories:` for the wizard.
//! - **App templates** (e.g. `_apps/databricks-app-rust/`) declare their own
//!   `categories:` plus a `lakehouse_requires:` block describing the shared
//!   components they need.
//!
//! The legacy `variables` / `components` / `profiles` fields stay around so
//! externally-authored templates that pre-date this restructure keep working
//! during the transition (we just stop using them in our own embedded templates).

use std::collections::BTreeMap;

use serde::Deserialize;

/// A trestle template manifest (top-level, lives at `<template>/template.yaml`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub trestle_min_version: Option<String>,

    /// User-facing variables (text/bool/enum) that drive rendering.
    ///
    /// Each [`Variable`] may be answered interactively, supplied via `--set` /
    /// `--values`, or fall back to its declared default. Contrast with
    /// [`template_context`](Self::template_context), which holds fixed
    /// author-supplied constants that are never prompted for.
    #[serde(default)]
    pub variables: Vec<Variable>,

    /// Components, in declaration order. Both shared (`kind: shared`) and local
    /// (`kind: local`) components are listed here. Legacy field; new templates
    /// use `always:` + `categories:` instead.
    #[serde(default)]
    pub components: Vec<Component>,

    /// Component names that are always activated for this template, with no
    /// wizard prompt.
    ///
    /// This is the opinionated baseline of a base template (e.g. a gateway and a
    /// metadata store). The resolver pushes every name here into the active set
    /// before applying selections, app requirements, or legacy `--profile` /
    /// `--with` picks, so an `always:` component is never hidden behind a
    /// question.
    #[serde(default)]
    pub always: Vec<String>,

    /// Category pickers shown in the wizard.
    ///
    /// Base templates declare lakehouse-wide categories (e.g. `storage`,
    /// `catalog`); app templates declare app-private categories (e.g.
    /// `frontend`, `ci`) using the same [`Category`] schema. App-private
    /// categories are namespaced as `app.<app-name>.<category>` when referenced
    /// from `--select` and from a `--values` file.
    #[serde(default)]
    pub categories: Vec<Category>,

    /// App-template only: requirements this app places on lakehouse components.
    ///
    /// See [`LakehouseRequires`] for how `hard` and `soft` differ. The base
    /// template's own copy of this field is ignored — only apps layered on top
    /// contribute requirements.
    #[serde(default)]
    pub lakehouse_requires: LakehouseRequires,

    /// Named bundles of component names selectable with `--profile <name>`.
    ///
    /// Each key is a profile name and each value is the list of components that
    /// profile activates. Legacy field resolved only against the base
    /// template's block; new templates use [`categories`](Self::categories)
    /// instead.
    #[serde(default)]
    pub profiles: BTreeMap<String, Vec<String>>,

    /// Static context merged into every render, in addition to the
    /// user-supplied [`variables`](Self::variables).
    ///
    /// Unlike `variables`, these values are never prompted for and cannot be
    /// overridden from the CLI; they are fixed constants the template author
    /// wants available to MiniJinja (e.g. a derived service name). Values may
    /// themselves be MiniJinja expressions evaluated against the collected
    /// variables. When apps are layered on a base, each app's
    /// `template_context` is merged over the base's.
    #[serde(default)]
    pub template_context: BTreeMap<String, serde_yaml::Value>,

    /// Optional shell commands to run after rendering is complete.
    #[serde(default)]
    pub post_init: Vec<PostInitHook>,
}

/// A user-facing category picker in the wizard.
///
/// Categories with an explicit `options:` list use those values verbatim (useful
/// for app-private categories like `frontend: [react, none]` where the values
/// aren't component names). Categories without `options:` auto-discover eligible
/// components by matching `ComponentManifest::category == id`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default = "default_true")]
    pub multi: bool,
    #[serde(default = "default_true")]
    pub optional: bool,
    /// Default selection. Single-string and list-of-strings YAML forms both
    /// deserialise into a `Vec<String>` here.
    #[serde(default)]
    pub default: DefaultList,
    /// Explicit option list for app-private categories that aren't component
    /// names. When unset, the wizard auto-discovers components by category id.
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub help: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Wrapper that accepts either `default: foo` or `default: [foo, bar]` in YAML
/// and produces a `Vec<String>`.
#[derive(Debug, Clone, Default)]
pub struct DefaultList(pub Vec<String>);

impl<'de> Deserialize<'de> for DefaultList {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Vec<String>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or a list of strings")
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Vec<String>, E> {
                Ok(vec![v.to_string()])
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Vec<String>, E> {
                Ok(vec![v])
            }
            fn visit_seq<A>(self, mut access: A) -> std::result::Result<Vec<String>, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut out = Vec::new();
                while let Some(item) = access.next_element::<String>()? {
                    out.push(item);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_any(Visitor).map(DefaultList)
    }
}

impl std::ops::Deref for DefaultList {
    type Target = Vec<String>;
    fn deref(&self) -> &Vec<String> {
        &self.0
    }
}

impl DefaultList {
    /// Convenience: convert to a plain `Vec<String>` for downstream consumers.
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }
}

/// Apps-only: declares which lakehouse (base-template) components an app needs.
///
/// Both `hard` and `soft` names are pushed into the active component set during
/// resolution — the distinction is *how the wizard presents them*:
///
/// * `hard` requirements are non-negotiable. The wizard activates them silently
///   (it only logs `pulls in: <name> (required)`), and the user cannot turn
///   them off.
/// * `soft` requirements are recommendations. The wizard pre-checks them in the
///   relevant category picker so the user sees them selected by default but may
///   deselect them.
///
/// In non-interactive resolution there is no picker to express the difference,
/// so both lists are simply activated.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LakehouseRequires {
    /// Components silently force-enabled when this app is selected; the user
    /// cannot deselect them.
    #[serde(default)]
    pub hard: Vec<String>,
    /// Components pre-checked in the wizard's category pickers; the user may
    /// still deselect them.
    #[serde(default)]
    pub soft: Vec<String>,
    /// Per-category default selections nudged when this app is picked.
    ///
    /// Keyed by category id; the values become the default selection for that
    /// picker (e.g. defaulting the `ml` category to a particular provider when
    /// this app is chosen).
    #[serde(default)]
    pub recommended_categories: BTreeMap<String, Vec<String>>,
}

/// Manifest for a single component (`<component>/template.yaml`).
///
/// A component is a reusable subtree that contributes one or more of: files
/// (under `template/`), Envoy routes, Envoy clusters, Postgres databases, S3
/// buckets, environment variables, declared ports, and Docker Compose
/// `include:` paths. Its typed contributions are declared under
/// [`provides`](Self::provides) and aggregated into the `stack.*` render
/// context.
///
/// Components come in two flavours, distinguished by [`ComponentKind`] where the
/// parent template references them:
///
/// * **Shared** components live in the shared component library
///   (`templates/_components/`) and are reused across templates.
/// * **Local** components live under a parent template's own `components/`
///   directory.
///
/// The manifest schema is identical for both; only the on-disk location and
/// lookup strategy differ.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,

    /// Categorisation used by the wizard to group components into pickers. Free-form
    /// string so new categories (e.g. `security`, `lineage`) are additive. Components
    /// without a category aren't surfaced in any category picker; they can still be
    /// pulled in by name via an app's `lakehouse_requires` or the CLI's `--with`.
    #[serde(default)]
    pub category: Option<String>,

    /// Finer-grained slot within a category. Multiple components may share the same
    /// `provider_of` (e.g. several `object_store` providers under `storage`) so the
    /// wizard can offer one-of-N picks within a category if it wants to.
    #[serde(default)]
    pub provider_of: Option<String>,

    /// Human-readable name shown in the wizard (falls back to `name` if missing).
    #[serde(default)]
    pub display_name: Option<String>,

    /// One-line summary used as the second column in the wizard's pickers.
    #[serde(default)]
    pub summary: Option<String>,

    /// Components that cannot be enabled at the same time as this one. The wizard
    /// surfaces a clear error if a selection would activate a conflicting pair.
    #[serde(default)]
    pub conflicts_with: Vec<String>,

    /// Human-readable wiring hints for the pre-render preview (env vars set, URLs
    /// exposed, etc.). The `preview.rs` module groups these by component.
    #[serde(default)]
    pub wire_help: Vec<WireHelp>,

    /// Other components this component depends on. Resolved transitively.
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Typed contributions aggregated into the `stack.*` template context.
    #[serde(default)]
    pub provides: Provides,
}

/// A single wiring hint shown in the pre-render preview. Exactly one of `env` or
/// `url` is expected; both are accepted as optional so authors can mix and match.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireHelp {
    /// Environment variable this component sets in the rendered `.env.example`
    /// (and downstream services).
    #[serde(default)]
    pub env: Option<String>,
    /// URL (or URL template) this component exposes.
    #[serde(default)]
    pub url: Option<String>,
    /// Short human-readable description.
    #[serde(default)]
    pub note: Option<String>,
}

/// A user-facing variable declaration.
///
/// Values are resolved with this precedence: a `--set` override, then a
/// `--values` file entry, then (interactively) the answer to the prompt, then
/// the declared [`default`](Self::default). A variable with a `default` and no
/// `prompt` is treated as a silent default and never asked about.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    #[serde(default = "VarKind::default_kind")]
    #[serde(rename = "type")]
    pub kind: VarKind,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Fallback value used when the variable is neither overridden nor answered.
    ///
    /// In `--non-interactive` mode a variable with no resolved value and no
    /// default is a hard error. The value is interpreted per [`kind`](Self::kind):
    /// a string for [`VarKind::String`] / [`VarKind::Enum`], a bool for
    /// [`VarKind::Bool`].
    #[serde(default)]
    pub default: Option<serde_yaml::Value>,
    /// Allowed values for [`VarKind::Enum`]; ignored for other kinds. Required
    /// (must be non-empty) when `kind` is `enum`.
    #[serde(default)]
    pub options: Vec<String>,
    /// Optional regular expression the answer must match.
    ///
    /// Applied only to [`VarKind::String`] variables — both for CLI-supplied
    /// values and for interactive input. A malformed regex, or a value that
    /// fails to match, is reported as an error.
    #[serde(default)]
    pub validate: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
}

/// The type of a user-facing [`Variable`], selected by the manifest's `type:`
/// key (defaults to [`VarKind::String`]).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VarKind {
    /// Free-form text, optionally constrained by [`Variable::validate`].
    String,
    /// A boolean. CLI values accept `true`/`yes`/`1`/`y` and
    /// `false`/`no`/`0`/`n`; interactively it renders as a yes/no confirm.
    Bool,
    /// A single choice from [`Variable::options`], rendered as a select menu.
    /// Spelled `enum` in the manifest YAML.
    Enum,
}

impl VarKind {
    pub(crate) fn default_kind() -> Self {
        VarKind::String
    }
}

/// A reference to a component declared in a template manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub name: String,
    #[serde(default = "default_component_kind")]
    pub kind: ComponentKind,
    /// Path relative to the template root. Required when `kind == local`. Optional
    /// for shared components (the loader looks them up by name).
    #[serde(default)]
    pub path: Option<String>,
    /// Optional minijinja expression evaluated against the variable context; the
    /// component is enabled iff the expression is truthy.
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Lives in the parent template's `components/` directory.
    Local,
    /// Lives in the shared component library (`templates/_components/`).
    Shared,
}

fn default_component_kind() -> ComponentKind {
    ComponentKind::Local
}

/// Typed contributions a component makes to the aggregated `stack.*` render
/// context.
///
/// Every active component's `Provides` block is merged into a single
/// [`StackContext`](crate::template::StackContext) in dependency-resolved
/// order. Order-sensitive lists (like [`envoy_routes`](Self::envoy_routes),
/// where order is match priority) are preserved verbatim; set-like lists (like
/// [`postgres_databases`](Self::postgres_databases)) are deduplicated.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    /// Docker Compose `include:` paths contributed by this component,
    /// deduplicated across components.
    #[serde(default)]
    pub compose_includes: Vec<String>,
    /// Postgres database names this component needs, deduplicated.
    #[serde(default)]
    pub postgres_databases: Vec<String>,
    /// S3 (object-store) bucket names this component needs, deduplicated.
    #[serde(default)]
    pub s3_buckets: Vec<String>,
    /// Envoy route entries; order is preserved because it determines match
    /// priority.
    #[serde(default)]
    pub envoy_routes: Vec<EnvoyRoute>,
    /// Envoy upstream clusters, deduplicated by name.
    #[serde(default)]
    pub envoy_clusters: Vec<EnvoyCluster>,
    /// Environment variables for the rendered stack. When several components
    /// set the same key, the later contributor (in topological order) wins.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    /// Ports this component exposes, deduplicated by name.
    #[serde(default)]
    pub ports: Vec<PortDecl>,
    /// Free-form per-component data, for options that don't warrant a dedicated
    /// typed `provides:` key.
    ///
    /// Each entry is namespaced by the contributing component when aggregated,
    /// so it surfaces in the render context as
    /// `stack.extras["<component_name>__<key>"]`. For example, a component
    /// named `metastore` declaring:
    ///
    /// ```yaml
    /// provides:
    ///   extras:
    ///     schema: bronze
    /// ```
    ///
    /// is read in a template as `{{ stack.extras["metastore__schema"] }}`.
    #[serde(default)]
    pub extras: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvoyRoute {
    pub prefix: String,
    pub cluster: String,
    /// Optional URL rewrite. If empty/missing, the path is passed through unchanged.
    #[serde(default)]
    pub rewrite: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvoyCluster {
    pub name: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortDecl {
    pub name: String,
    pub default: u16,
    #[serde(default)]
    pub internal_only: bool,
}

/// A shell command run after rendering completes.
///
/// Hooks run in declaration order, gated by an optional [`when`](Self::when)
/// expression. See [`confirm`](Self::confirm) for the interaction with
/// `--non-interactive`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostInitHook {
    pub run: String,
    /// When `true`, the user is asked to approve the command before it runs.
    ///
    /// Defaults to `false` (the command runs unprompted). Because approval
    /// needs a prompt, a hook with `confirm: true` is **skipped entirely** in
    /// `--non-interactive` mode rather than run unattended.
    #[serde(default)]
    pub confirm: bool,
    /// Optional minijinja expression; the hook runs iff it evaluates to truthy.
    #[serde(default)]
    pub when: Option<String>,
    /// Optional human-readable description shown in the confirm prompt.
    #[serde(default)]
    pub description: Option<String>,
}

/// A named profile shorthand for a bundle of components.
pub type Profile = Vec<String>;
