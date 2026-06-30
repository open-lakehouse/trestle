//! The planner↔template render handshake ([`RenderOutput`], [`InjectedEnv`]).
//!
//! This crate is pure: it does not render anything (no MiniJinja, no I/O). What it
//! defines is the **data contract** on both sides of a module's render step, so the
//! consuming tool (trestle's scaffolder, hydrofoil's runtime generator) and the
//! modules agree on shape:
//!
//! - The planner hands a module's render an [`InjectedEnv`] — the values it decided
//!   that the module could not know on its own (a UI's chosen base path, assigned
//!   ports, the mount root for the module's files). These are delivered as compose
//!   **environment-variable substitutions**: the single uniform injection point
//!   that both a service's command-line flags *and* the contents of a mounted
//!   config file can read. The module never learns *which* the value feeds — that
//!   stays inside its own fragment/files.
//! - The module's render produces a [`RenderOutput`]: one compose fragment plus zero
//!   or more [`RenderFile`]s to be written and mounted (e.g. a config file whose
//!   contents reference the injected variables).
//!
//! Why env-var substitution as the contract: we mount config files anyway, so a
//! single substitution mechanism covers every application style — `command:
//! mlflow --static-prefix ${BASE_PATH}`, `environment: APP_BASE_PATH=${BASE_PATH}`,
//! or a mounted `config.yaml` containing `base_path: ${BASE_PATH}`. The planner
//! stays oblivious to which; the template decides.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The environment-variable substitutions the planner injects into a module's
/// render — the values the module could not decide for itself.
///
/// Keys are compose env-var names (e.g. `"BASE_PATH"`, `"HOST_PORT"`); a module's
/// fragment/files reference them as `${KEY}`. The planner owns these values; the
/// template owns how (and whether) each is consumed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InjectedEnv {
    vars: BTreeMap<String, String>,
}

impl InjectedEnv {
    /// An empty injection set.
    pub fn new() -> Self {
        InjectedEnv::default()
    }

    /// Set `key` to `value`, replacing any previous value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    /// The value for `key`, if set.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    /// Iterate the injected `(key, value)` pairs in deterministic (key) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// One file a module's render emits to be written to disk and mounted into the
/// service (e.g. a config file). Its `contents` may reference injected variables
/// as `${KEY}` for compose to substitute at run time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderFile {
    /// Path the file should be written to. Module-relative (a bare basename or a
    /// short subpath); the consumer roots it under the module's own directory
    /// (`modules/<module-id>/<path>`). The consumer turns this into the host side of
    /// the file's mount.
    pub path: String,
    /// The file's contents (possibly containing `${KEY}` substitutions).
    pub contents: String,
    /// The top-level compose `configs:` alias this file is mounted under, if any.
    ///
    /// When set, the generated root compose declares
    /// `configs: <alias>: { file: <rooted-path> }` and the module's own fragment
    /// references it via `configs: - source: <alias>`. `None` means a plain file the
    /// consumer just writes (e.g. a bind-mounted file the fragment mounts by path).
    /// Aliases share one top-level namespace, so they must be unique across modules;
    /// the planner rejects a collision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// What a module's render produces: a compose fragment plus any files to mount.
///
/// The consuming renderer is responsible for the actual rendering (templating the
/// fragment, materializing the files) and for wiring [`InjectedEnv`] into the
/// compose environment; this type is just the agreed output shape.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderOutput {
    /// The compose fragment for this module (a `services:` snippet, typically
    /// `include:`d by the generated top-level compose).
    pub fragment: String,
    /// Files the module needs written and mounted alongside its fragment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<RenderFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_env_is_ordered_and_addressable() {
        let mut env = InjectedEnv::new();
        env.set("HOST_PORT", "9080");
        env.set("BASE_PATH", "/mlflow");
        assert_eq!(env.get("BASE_PATH"), Some("/mlflow"));
        // Deterministic key order (BTreeMap): BASE_PATH before HOST_PORT.
        let pairs: Vec<_> = env.iter().collect();
        assert_eq!(pairs, vec![("BASE_PATH", "/mlflow"), ("HOST_PORT", "9080")]);
    }

    #[test]
    fn render_output_carries_fragment_and_mountable_files() {
        let out = RenderOutput {
            fragment: "services:\n  mlflow: {}\n".into(),
            files: vec![RenderFile {
                path: "mlflow.yaml".into(),
                // A mounted config file referencing an injected variable —
                // compose substitutes ${BASE_PATH} at run time.
                contents: "base_path: ${BASE_PATH}\n".into(),
                alias: Some("mlflow_config".into()),
            }],
        };
        assert_eq!(out.files.len(), 1);
        assert_eq!(out.files[0].path, "mlflow.yaml");
        assert!(out.files[0].contents.contains("${BASE_PATH}"));
        assert_eq!(out.files[0].alias.as_deref(), Some("mlflow_config"));
    }
}
