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

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::catalog::deploy::{DeployPhase, DeployStepSpec};
use crate::plan::Plan;
use crate::render::RenderError;
use crate::render::deploy::SECRETS_ENV_PATH;
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
    /// The deployment steps this target adds to the modules' (see
    /// [`catalog::deploy`](crate::catalog::deploy)): typically supplying secrets and starting
    /// the stack. Within a phase they follow every module's steps.
    fn deploy_steps(&self, plan: &Plan) -> Vec<DeployStepSpec> {
        let _ = plan;
        Vec::new()
    }
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
    fn deploy_steps(&self, plan: &Plan) -> Vec<DeployStepSpec> {
        let mut steps = Vec::new();
        // The texts are built here, not templated: wrap them so nothing in them is read as
        // template syntax.
        let raw = |text: String| format!("{{% raw %}}{text}{{% endraw %}}");

        let secrets: BTreeSet<String> = plan
            .hosted
            .values()
            .flat_map(|r| r.connection.secret_refs())
            .collect();
        if !secrets.is_empty() {
            let list = secrets
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ");
            steps.push(DeployStepSpec::new(
                "secrets",
                DeployPhase::Configure,
                "Supply the hosted resources' secrets",
                raw(format!(
                    "The hosted resources read {list}. Set them in `{SECRETS_ENV_PATH}` beside \
                     `env.toml` (git-ignored; the scripts above save what they create there) or \
                     export them, then render again so the stack picks them up:\n\n\
                     ```bash\ntrestle env render .\n```"
                )),
            ));
        }

        let reach = match plan.gateway_url() {
            Some(url) => format!("The gateway then serves the stack at <{url}>."),
            None => "Each service is then published on its own host port.".into(),
        };
        steps.push(DeployStepSpec::new(
            "up",
            DeployPhase::Start,
            "Start the stack",
            raw(format!(
                "Start every service and wait until each one is healthy:\n\n\
                 ```bash\ndocker compose up -d --wait\n```\n\n{reach} [LAYOUT.md](LAYOUT.md) \
                 lists every route and address."
            )),
        ));

        if let Some(secure) = &plan.gateway.secure {
            steps.push(DeployStepSpec::new(
                "trust",
                DeployPhase::Verify,
                "Trust the gateway's local certificate authority",
                raw(format!(
                    "The gateway serves HTTPS with a certificate from a CA minted for this \
                     environment. Copy it out, then add it to your browser or pass it to \
                     `curl --cacert`:\n\n\
                     ```bash\ndocker compose cp {}:{} .\n```",
                    secure.mint_service, secure.trust.ca_pem
                )),
            ));
        }
        steps
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
