//! **Deploy targets**: what an environment runs on, and how a [`Plan`] becomes its files.
//!
//! The planner is target-neutral — a [`Plan`] holds each module's typed
//! [`ModuleBinding`](crate::ModuleBinding), never rendered text. A [`DeployTarget`] owns the rest:
//! it renders every module from its target-specific [`RenderSpec`](crate::RenderSpec) (see
//! [`Module::render_for`](crate::Module::render_for)) and lays the result out on disk as a
//! [`MaterializedOutput`].
//!
//! An environment picks one target ([`PlanCtx::target`](crate::PlanCtx::target), persisted in
//! `env.toml`). The planner rejects a selected module that has no rendering for it
//! ([`PlanError::UnsupportedOnTarget`](crate::PlanError::UnsupportedOnTarget)), so a module
//! never silently vanishes from a stack.
//!
//! The one built-in target today is [Docker Compose](Compose) ([`TargetId::COMPOSE`]); look a
//! target up by id with [`builtin`]. A new target implements [`DeployTarget`] and registers a
//! [`RenderSpec`](crate::RenderSpec) per module under its id.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::plan::Plan;
use crate::render::RenderError;
use crate::render::output::MaterializedOutput;

/// A deploy target's stable id (e.g. `"compose"`), as written in `env.toml` and used to key a
/// module's per-target [`RenderSpec`](crate::RenderSpec)s.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TargetId(pub String);

impl TargetId {
    /// The id of the built-in Docker Compose target.
    pub const COMPOSE: &'static str = "compose";

    /// A target id from a string.
    pub fn new(s: impl Into<String>) -> Self {
        TargetId(s.into())
    }

    /// The built-in Docker Compose target's id.
    pub fn compose() -> Self {
        TargetId::new(Self::COMPOSE)
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TargetId {
    /// Docker Compose, the default target.
    fn default() -> Self {
        TargetId::compose()
    }
}

impl From<&str> for TargetId {
    fn from(s: &str) -> Self {
        TargetId::new(s)
    }
}

impl std::fmt::Display for TargetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Something an environment can be deployed to: it turns a [`Plan`] into write-ready files.
///
/// Implementations are pure (no I/O): they return a [`MaterializedOutput`] the consumer writes
/// (with the `std-io` feature, [`MaterializedOutput::write_to`]) or, in a browser, just shows.
pub trait DeployTarget: Send + Sync {
    /// The target's stable id.
    fn id(&self) -> TargetId;
    /// A human-readable name for a wizard/UI.
    fn display_name(&self) -> &str;
    /// Render `plan` and lay it out in this target's on-disk layout.
    ///
    /// Errors with a [`RenderError`] when a module's template fails, or when a module has no
    /// [`RenderSpec`](crate::RenderSpec) for this target (a plan made for a different target).
    fn materialize(&self, plan: &Plan) -> Result<MaterializedOutput, RenderError>;
}

/// The built-in **Docker Compose** target: a top-level `compose.yaml` that `include:`s one
/// fragment per module under `modules/<id>/`, plus the `.env` overlay, the Envoy bootstrap and
/// the client environments (see [`render::compose`](crate::render::compose)).
#[derive(Clone, Copy, Debug, Default)]
pub struct Compose;

impl DeployTarget for Compose {
    fn id(&self) -> TargetId {
        TargetId::compose()
    }
    fn display_name(&self) -> &str {
        "Docker Compose"
    }
    fn materialize(&self, plan: &Plan) -> Result<MaterializedOutput, RenderError> {
        crate::render::compose::materialize(plan)
    }
}

/// Every built-in deploy target, in display order.
pub fn builtin_targets() -> Vec<Arc<dyn DeployTarget>> {
    vec![Arc::new(Compose)]
}

/// The built-in deploy target with id `id`, if there is one.
pub fn builtin(id: &TargetId) -> Option<Arc<dyn DeployTarget>> {
    builtin_targets().into_iter().find(|t| &t.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_is_the_default_builtin_target() {
        assert_eq!(TargetId::default(), TargetId::compose());
        let compose = builtin(&TargetId::default()).expect("compose is built in");
        assert_eq!(compose.display_name(), "Docker Compose");
        assert!(builtin(&TargetId::from("helm")).is_none());
    }

    #[test]
    fn target_id_serializes_as_a_bare_string() {
        #[derive(Serialize, Deserialize)]
        struct Wrap {
            target: TargetId,
        }
        let toml = toml::to_string(&Wrap {
            target: TargetId::compose(),
        })
        .unwrap();
        assert_eq!(toml.trim(), r#"target = "compose""#);
        let back: Wrap = toml::from_str(&toml).unwrap();
        assert_eq!(back.target, TargetId::compose());
    }
}
