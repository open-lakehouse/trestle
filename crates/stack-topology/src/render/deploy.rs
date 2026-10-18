//! The environment's deployment steps, rendered in order ([`render_steps`]), and the README
//! that walks an operator through them ([`render_readme`]).
//!
//! Every module's [`deploy_steps`](crate::Module::deploy_steps) and the deploy target's own
//! [`deploy_steps`](crate::DeployTarget::deploy_steps) are rendered against a
//! [`DeployCtx`], then ordered by [`DeployPhase`], within a phase in plan (dependency) order
//! with the target's steps last. A step's script is wrapped in a common preamble and laid out
//! as `deploy/<module>-<step>.sh`, a path that stays put when other modules come and go.
//!
//! Pure (no I/O), like the rest of rendering.

use std::fmt::Write as _;

use crate::catalog::deploy::{DeployCtx, DeployPhase, DeployStepSpec};
use crate::catalog::module::{ModuleId, ResolvedKnobs};
use crate::plan::Plan;
use crate::render::RenderError;
use crate::target::DeployTarget;

/// Where the deployment README is written: the environment's root.
pub const DEPLOY_README_PATH: &str = "README.md";

/// The directory, under the environment's root, that holds the steps' scripts.
pub const DEPLOY_SCRIPTS_DIR: &str = "deploy";

/// The git-ignored file, beside `env.toml`, that secrets may be kept in. The scripts load it
/// and save what they create to it.
pub const SECRETS_ENV_PATH: &str = "secrets.env";

/// One rendered deployment step, in its place in the deployment order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployStep {
    /// The step's 1-based position in the deployment order.
    pub order: usize,
    /// When it happens.
    pub phase: DeployPhase,
    /// The module that contributed it; `None` for the deploy target's own step.
    pub module: Option<ModuleId>,
    /// The step's id, unique within its module.
    pub id: String,
    /// Its title.
    pub title: String,
    /// What to do, as Markdown.
    pub instructions: String,
    /// The script that does it, if it has one.
    pub script: Option<DeployScript>,
}

/// A [`DeployStep`]'s script: where it is written and what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployScript {
    /// The path relative to the environment's root (`deploy/<module>-<step>.sh`).
    pub path: String,
    /// The whole script, preamble included.
    pub contents: String,
    /// The step's own commands, without the preamble: safe to paste into a shell that has run
    /// [`DEPLOY_SHELL_SETUP`] from the environment's root.
    pub body: String,
}

/// Render every deployment step of `plan` on `target`, in deployment order.
///
/// Fails with [`RenderError::DeployStep`] when a step's template doesn't render.
pub fn render_steps(
    plan: &Plan,
    target: &dyn DeployTarget,
) -> Result<Vec<DeployStep>, RenderError> {
    let target_id = target.id();
    let mut steps = Vec::new();
    for module in &plan.graph.nodes {
        let id = module.id();
        let objects = plan.provisioned_by(id);
        let ctx = DeployCtx {
            env_name: &plan.env_name,
            target: &target_id,
            module: Some(id),
            knobs: plan
                .bindings
                .get(id)
                .map_or(ResolvedKnobs::empty_ref(), |b| &b.knobs),
            objects,
            stack: &plan.stack_facts,
            hosted: plan.hosted.get(id).map(|r| r.step_view(objects)),
        };
        for spec in module.deploy_steps() {
            steps.extend(render_one(spec, &ctx, id.as_str(), Some(id))?);
        }
    }
    let ctx = DeployCtx {
        env_name: &plan.env_name,
        target: &target_id,
        module: None,
        knobs: ResolvedKnobs::empty_ref(),
        objects: &[],
        stack: &plan.stack_facts,
        hosted: None,
    };
    for spec in target.deploy_steps(plan) {
        steps.extend(render_one(&spec, &ctx, target_id.as_str(), None)?);
    }
    // A stable sort keeps plan order (and the target's steps last) within each phase.
    steps.sort_by_key(|s| s.phase);
    for (i, step) in steps.iter_mut().enumerate() {
        step.order = i + 1;
    }
    Ok(steps)
}

/// Render one step, its script named after `owner` (the module, or the target).
fn render_one(
    spec: &DeployStepSpec,
    ctx: &DeployCtx<'_>,
    owner: &str,
    module: Option<&ModuleId>,
) -> Result<Option<DeployStep>, RenderError> {
    let Some(rendered) = spec.render(ctx).map_err(|source| RenderError::DeployStep {
        step: format!("{owner}/{}", spec.id),
        source,
    })?
    else {
        return Ok(None);
    };
    let script = rendered.script.map(|body| {
        let body = body.trim_start_matches('\n');
        DeployScript {
            path: format!("{DEPLOY_SCRIPTS_DIR}/{owner}-{}.sh", spec.id),
            contents: script_contents(&rendered.title, owner, ctx.env_name, body),
            body: if body.ends_with('\n') {
                body.to_string()
            } else {
                format!("{body}\n")
            },
        }
    });
    Ok(Some(DeployStep {
        order: 0,
        phase: spec.phase,
        module: module.cloned(),
        id: spec.id.clone(),
        title: rendered.title,
        instructions: rendered.instructions,
        script,
    }))
}

/// A step's script: the common preamble, then its body.
fn script_contents(title: &str, owner: &str, env_name: &str, body: &str) -> String {
    let mut out = String::from("#!/usr/bin/env bash\n");
    let _ = writeln!(
        out,
        "# {title} (`{owner}`, environment `{env_name}`). Generated by trestle; re-render to \
         update."
    );
    out.push_str(STRICT_PREAMBLE);
    out.push_str("\n\n");
    out.push_str(DEPLOY_SHELL_SETUP);
    out.push('\n');
    out.push_str(body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// What every script starts with before [`DEPLOY_SHELL_SETUP`]: strict mode, and the
/// environment's root as working directory. Neither belongs in an interactive shell, where
/// `set -e` would close it on the first failure and `$0` is the shell.
const STRICT_PREAMBLE: &str = r#"set -euo pipefail
cd "$(dirname "$0")/..""#;

/// What a step's [commands](DeployScript::body) need first, in a script or in a shell opened at
/// the environment's root: `secrets.env` loaded, and the `save_secret` helper. Safe to run
/// interactively, and more than once.
pub const DEPLOY_SHELL_SETUP: &str = r#"# Secrets kept in secrets.env (git-ignored) are read like exported ones.
if [ -f secrets.env ]; then
  set -a
  # shellcheck source=/dev/null
  . ./secrets.env
  set +a
fi

# save_secret NAME VALUE: record NAME in secrets.env (and this shell), unless secrets.env
# already sets it.
save_secret() {
  if [ -f secrets.env ] && grep -q "^$1=" secrets.env; then
    echo "secrets.env already sets $1; leaving it as is"
  else
    (umask 077 && printf '%s=%s\n' "$1" "$2" >> secrets.env)
    echo "saved $1 to secrets.env"
  fi
  export "$1=$2"
}
"#;

/// The deployment README of `plan`: every step in order, under its phase, with the command
/// that runs its script.
pub fn render_readme(plan: &Plan, steps: &[DeployStep]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Deploying `{}`\n", plan.env_name);
    out.push_str(
        "Generated by `trestle env render`; re-render to update. Follow the steps in order. A \
         step's script\nruns from anywhere and is safe to re-run. The services, their routes \
         and addresses are in\n[LAYOUT.md](LAYOUT.md).\n",
    );
    for phase in DeployPhase::ALL {
        let mut in_phase = steps.iter().filter(|s| s.phase == phase).peekable();
        if in_phase.peek().is_none() {
            continue;
        }
        let _ = writeln!(out, "\n## {}", phase.heading());
        for step in in_phase {
            let _ = write!(out, "\n### {}. {}", step.order, step.title);
            if let Some(module) = &step.module {
                let _ = write!(out, " (`{module}`)");
            }
            let _ = writeln!(out, "\n\n{}", step.instructions);
            if let Some(script) = &step.script {
                let _ = writeln!(out, "\n```bash\nbash {}\n```", script.path);
            }
        }
    }
    out
}
