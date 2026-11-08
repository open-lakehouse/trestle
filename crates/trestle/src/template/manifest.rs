//! Schema for `template.yaml` (top-level templates and components).
//!
//! Templates come in two flavours and share one manifest type:
//! - **Base templates** (e.g. `_base/lakehouse/`) are always rendered.
//! - **App templates** (e.g. `_apps/databricks-app-rust/`) are layered on top with
//!   `--app`. They declare option `categories:` for the wizard and `--select`, and
//!   their own `components:`.
//!
//! A component ([`ComponentManifest`]) is a subtree under the declaring template's
//! `components/` directory, enabled by a `when:` expression.

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

    /// Components, in declaration order. Each lives under this template's
    /// `components/` directory and is enabled by its `when:` expression.
    #[serde(default)]
    pub components: Vec<Component>,

    /// App-template option pickers shown in the wizard (e.g. `frontend`, `ci`).
    ///
    /// Referenced as `app.<app-name>.<category>` from `--select` and from a
    /// `--values` file. A base template's categories are ignored.
    #[serde(default)]
    pub categories: Vec<Category>,

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

/// A user-facing option picker in the wizard (e.g. `frontend: [react, none]`).
///
/// The picked values are option strings, not component names; `trestle new` maps
/// them onto the `with_*` variables that components' `when:` expressions read.
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
    /// The values to choose from. `none` means "no pick".
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

/// Manifest for a single component (`<component>/template.yaml`).
///
/// A component is a subtree of files (under `template/`) that renders into the
/// project when its parent template enables it.
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
    /// Path relative to the template root. Defaults to `components/<name>`.
    #[serde(default)]
    pub path: Option<String>,
    /// Optional minijinja expression evaluated against the variable context; the
    /// component is enabled iff the expression is truthy.
    #[serde(default)]
    pub when: Option<String>,
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
