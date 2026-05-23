//! Schema for `template.yaml` (top-level templates and components).
//!
//! Templates declare variables, components, profiles, post-init hooks, and a `provides:`
//! block (for components). The parser is intentionally tolerant of missing fields so that
//! authoring a minimal manifest stays cheap.

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
    pub authors: Vec<String>,
    #[serde(default)]
    pub trestle_min_version: Option<String>,

    /// User-facing variables (text/bool/enum) that drive rendering.
    #[serde(default)]
    pub variables: Vec<Variable>,

    /// Components, in declaration order. Both shared (`kind: shared`) and local
    /// (`kind: local`) components are listed here.
    #[serde(default)]
    pub components: Vec<Component>,

    /// Named bundles selectable with `--profile <name>`. Each value is a list of
    /// component names (referring to either the shared library or template-private
    /// components).
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

    /// Other components this component depends on. Resolved transitively.
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Typed contributions aggregated into the `stack.*` template context.
    #[serde(default)]
    pub provides: Provides,
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
