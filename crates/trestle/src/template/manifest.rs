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
    #[serde(default)]
    pub variables: Vec<Variable>,

    /// Components, in declaration order. Both shared (`kind: shared`) and local
    /// (`kind: local`) components are listed here. Legacy field; new templates
    /// use `always:` + `categories:` instead.
    #[serde(default)]
    pub components: Vec<Component>,

    /// Base-template only: components that are always rendered, without any
    /// wizard prompt. The opinionated baseline (e.g. Envoy as the gateway)
    /// lives here.
    #[serde(default)]
    pub always: Vec<String>,

    /// Categories shown in the wizard. Base templates declare lakehouse-wide
    /// categories (storage, catalog, …); app templates declare app-private
    /// categories (frontend, ci, …) using the same schema.
    #[serde(default)]
    pub categories: Vec<Category>,

    /// App-template only: hard/soft requirements on lakehouse components. The
    /// wizard silently enables `hard:` items and pre-checks `soft:` items in
    /// their respective category pickers.
    #[serde(default)]
    pub lakehouse_requires: LakehouseRequires,

    /// Named bundles selectable with `--profile <name>`. Legacy field; new
    /// templates use `categories:` instead.
    #[serde(default)]
    pub profiles: BTreeMap<String, Vec<String>>,

    /// Optional static context exposed to all renders. Useful for declaring derived
    /// constants like `app_service_name` without forcing the user to type them.
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

/// Apps-only: declare which lakehouse components this app needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LakehouseRequires {
    /// Components auto-enabled silently when this app is selected.
    #[serde(default)]
    pub hard: Vec<String>,
    /// Components pre-checked in the wizard; the user may still deselect them.
    #[serde(default)]
    pub soft: Vec<String>,
    /// Per-category default selections nudged when this app is picked.
    /// Useful for "if you pick the Rust app, default the `ml` picker to MLflow".
    #[serde(default)]
    pub recommended_categories: BTreeMap<String, Vec<String>>,
}

/// Component manifest (`<component>/template.yaml`).
///
/// Components contribute one or more of: files (under `template/`), envoy routes,
/// envoy clusters, postgres databases, S3 buckets, environment variables, and Docker
/// Compose `include:` paths.
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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    #[serde(default = "VarKind::default_kind")]
    #[serde(rename = "type")]
    pub kind: VarKind,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub default: Option<serde_yaml::Value>,
    #[serde(default)]
    pub options: Vec<String>,
    /// Optional regex validation (for `string` kind).
    #[serde(default)]
    pub validate: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VarKind {
    String,
    Bool,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    #[serde(default)]
    pub compose_includes: Vec<String>,
    #[serde(default)]
    pub postgres_databases: Vec<String>,
    #[serde(default)]
    pub s3_buckets: Vec<String>,
    #[serde(default)]
    pub envoy_routes: Vec<EnvoyRoute>,
    #[serde(default)]
    pub envoy_clusters: Vec<EnvoyCluster>,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub ports: Vec<PortDecl>,
    /// Free-form per-component data exposed at `stack.extras.<component_name>`.
    /// Useful for component-specific options that the parent template wants to
    /// consume without inventing a new typed `provides:` key.
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostInitHook {
    pub run: String,
    /// When `true`, prompt the user before executing. Defaults to `false`.
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
