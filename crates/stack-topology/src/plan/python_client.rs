//! The **Python client workspace**: a uv project under `client/` that a user runs against the
//! stack, aggregated from every selected module's [`python`](crate::Provides::python)
//! contribution. Opt-in via [`Selection::client`](crate::Selection::client).

use serde::{Deserialize, Serialize};

use super::Plan;
use crate::catalog::module::{ClientNotebook, ModuleId};

/// The packages every client workspace gets, whatever the selection: the notebook runtime and
/// the dotenv loader it uses to read `host.env`, plus an HTTP client for the base notebook.
pub const PYTHON_CLIENT_BASE_DEPENDENCIES: &[&str] = &["marimo>=0.14", "python-dotenv", "httpx"];

/// Whether to generate the Python client workspace, and whether to fill it with the selected
/// modules' example notebooks. Both off by default, so a manifest that omits the section
/// renders exactly as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientSelection {
    /// Generate the workspace (`client/pyproject.toml`, a README and a connection notebook).
    #[serde(default)]
    pub python: bool,
    /// Also emit each selected module's example notebooks. Implies [`python`](Self::python).
    #[serde(default)]
    pub examples: bool,
}

impl ClientSelection {
    /// Whether the workspace is generated at all.
    pub fn enabled(&self) -> bool {
        self.python || self.examples
    }

    /// Whether the section is all defaults (so a manifest can omit it).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The settled Python client workspace (see [`Plan::python_client`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PythonWorkspace {
    /// The PEP 508 requirements for `pyproject.toml`: the
    /// [base](PYTHON_CLIENT_BASE_DEPENDENCIES) set, then each module's in dependency order,
    /// one entry per package (a later module's requirement replaces an earlier one in place).
    pub dependencies: Vec<String>,
    /// The example notebooks to emit, with the module that contributed each, in dependency
    /// order. Empty unless the selection asked for examples.
    pub notebooks: Vec<(ModuleId, ClientNotebook)>,
}

/// Aggregate the graph's Python contributions, or `None` when the selection doesn't ask for
/// the workspace.
pub(super) fn resolve_python_client(
    plan: &Plan,
    client: ClientSelection,
) -> Option<PythonWorkspace> {
    if !client.enabled() {
        return None;
    }
    let mut workspace = PythonWorkspace::default();
    for dep in PYTHON_CLIENT_BASE_DEPENDENCIES {
        add_dependency(&mut workspace.dependencies, dep);
    }
    for module in &plan.graph.nodes {
        // An external provider runs nowhere the stack controls, but a client still talks to it,
        // so its packages stay in.
        let python = &module.provides().python;
        for dep in &python.dependencies {
            add_dependency(&mut workspace.dependencies, dep);
        }
        if client.examples {
            for notebook in &python.notebooks {
                workspace.notebooks.retain(|(_, n)| n.file != notebook.file);
                workspace
                    .notebooks
                    .push((module.id().clone(), notebook.clone()));
            }
        }
    }
    Some(workspace)
}

/// Add `dep`, replacing an earlier requirement for the same package in place.
fn add_dependency(deps: &mut Vec<String>, dep: &str) {
    let name = package_name(dep);
    match deps.iter_mut().find(|d| package_name(d) == name) {
        Some(existing) => *existing = dep.to_string(),
        None => deps.push(dep.to_string()),
    }
}

/// The normalized package name of a PEP 508 requirement (`"MLflow[extras]>=3"` → `"mlflow"`).
fn package_name(requirement: &str) -> String {
    requirement
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace(['_', '.'], "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_normalize() {
        assert_eq!(package_name("mlflow>=3"), "mlflow");
        assert_eq!(
            package_name("Scikit_Learn[all] ; python_version>'3'"),
            "scikit-learn"
        );
        assert_eq!(package_name("python-dotenv"), "python-dotenv");
    }

    #[test]
    fn a_later_requirement_replaces_an_earlier_one_in_place() {
        let mut deps = Vec::new();
        for dep in ["marimo", "mlflow>=3", "httpx", "MLflow[skinny]>=3.1"] {
            add_dependency(&mut deps, dep);
        }
        assert_eq!(deps, ["marimo", "MLflow[skinny]>=3.1", "httpx"]);
    }
}
