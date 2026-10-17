//! **Deploy steps**: what an operator does to bring an environment up beyond what the deploy
//! target runs: create a hosted database or bucket, review the sign-in accounts, start the
//! stack.
//!
//! A module declares its steps as data ([`Provides::deploy`](crate::Provides::deploy)), and a
//! deploy target adds its own ([`DeployTarget::deploy_steps`](crate::DeployTarget::deploy_steps)).
//! [`render_steps`](crate::render::deploy::render_steps) renders them all in deployment order
//! (by [`DeployPhase`], then dependencies first), and the target collects them into one
//! README. Each step is Markdown instructions plus an optional shell script that does the work.
//!
//! Steps are target-neutral: a hosted provider renders nothing on any target, yet its resources
//! still need creating. Their templates read a [`DeployCtx`], which carries a hosted
//! resource's coordinates and the *names* of its secret variables, never a secret's value, so
//! rendered steps are safe to commit and to show in a browser.

use serde::{Deserialize, Serialize};

use crate::catalog::module::{ModuleId, ResolvedKnobs, StackFacts, TemplateError};
use crate::plan::hosted::HostedStepView;
use crate::target::TargetId;

/// When a [`DeployStepSpec`] happens, relative to starting the stack. Steps are ordered by
/// phase first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployPhase {
    /// Create what the stack uses but doesn't run: hosted databases, buckets, accounts.
    Provision,
    /// Prepare the environment's own files and settings: secrets, local accounts.
    Configure,
    /// Start the stack.
    Start,
    /// Check that it works, once it runs.
    Verify,
}

impl DeployPhase {
    /// Every phase, in deployment order.
    pub const ALL: [DeployPhase; 4] = [
        DeployPhase::Provision,
        DeployPhase::Configure,
        DeployPhase::Start,
        DeployPhase::Verify,
    ];

    /// The phase's heading in a README.
    pub fn heading(self) -> &'static str {
        match self {
            DeployPhase::Provision => "Provision",
            DeployPhase::Configure => "Configure",
            DeployPhase::Start => "Start",
            DeployPhase::Verify => "Verify",
        }
    }
}

/// One deployment step a module (or deploy target) contributes: a title, Markdown instructions
/// and, optionally, a shell script that does it.
///
/// `title`, `instructions` and `script` are MiniJinja templates rendered against a
/// [`DeployCtx`]. A step whose instructions render to nothing but whitespace is dropped, so a
/// template can make a step conditional (`{% if hosted.vendor == "neon" %}…{% endif %}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployStepSpec {
    /// The step's id, unique within its module (e.g. `create-databases`). Names the step's
    /// script.
    pub id: String,
    /// When the step happens.
    pub phase: DeployPhase,
    /// A short imperative title (e.g. "Create the databases").
    pub title: String,
    /// What to do, as Markdown. When the step has a script, say what it does and what it
    /// needs (CLIs, credentials); the README adds the command that runs it.
    pub instructions: String,
    /// The body of a bash script that does the step. It runs from the environment's root
    /// with `set -euo pipefail`, `secrets.env` loaded, and a `save_secret NAME VALUE` helper
    /// that records a discovered value there. Write it to be safe to re-run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
}

impl DeployStepSpec {
    /// A step with no script.
    pub fn new(
        id: impl Into<String>,
        phase: DeployPhase,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Self {
        DeployStepSpec {
            id: id.into(),
            phase,
            title: title.into(),
            instructions: instructions.into(),
            script: None,
        }
    }

    /// This step with `script` as its script body.
    pub fn with_script(mut self, script: impl Into<String>) -> Self {
        self.script = Some(script.into());
        self
    }

    /// Render the step against `ctx`: `None` when its instructions render empty.
    pub fn render(&self, ctx: &DeployCtx<'_>) -> Result<Option<RenderedStep>, TemplateError> {
        let mut env = minijinja::Environment::new();
        env.set_keep_trailing_newline(true);
        env.add_filter("shell", shell_quote);
        let render = |source: &str| {
            env.render_named_str("step", source, ctx)
                .map_err(|e| TemplateError(e.to_string()))
        };
        let instructions = render(&self.instructions)?;
        if instructions.trim().is_empty() {
            return Ok(None);
        }
        let script = match &self.script {
            Some(script) => Some(render(script)?).filter(|s| !s.trim().is_empty()),
            None => None,
        };
        Ok(Some(RenderedStep {
            title: render(&self.title)?.trim().to_string(),
            instructions: instructions.trim().to_string(),
            script,
        }))
    }
}

/// A [`DeployStepSpec`] rendered: its title, instructions and script body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedStep {
    /// The rendered title.
    pub title: String,
    /// The rendered Markdown instructions.
    pub instructions: String,
    /// The rendered script body, if the step has one.
    pub script: Option<String>,
}

/// What a [`DeployStepSpec`]'s templates read, as MiniJinja globals: `env_name`, `target`,
/// `module` (absent for a target's step), `knobs`, `objects` (the resource names the module
/// provisions), `stack` (see [`StackFacts`]), and `hosted` (a hosted provider's
/// [`HostedStepView`]).
///
/// Unlike a module's [`RenderCtx`](crate::RenderCtx) it carries no connections: those hold
/// resolved secrets, and a step must never contain one.
///
/// A template can pass a value to a shell safely with the `shell` filter
/// (`{{ hosted.values.region | shell }}`): it single-quotes a literal and double-quotes a value
/// holding a `${VAR}` reference, so the shell expands the reference and nothing else.
#[derive(Clone, Debug, Serialize)]
pub struct DeployCtx<'a> {
    /// The environment's name.
    pub env_name: &'a str,
    /// The deploy target.
    pub target: &'a TargetId,
    /// The module the step belongs to; `None` for a deploy target's step.
    pub module: Option<&'a ModuleId>,
    /// The module's resolved knobs.
    pub knobs: &'a ResolvedKnobs,
    /// The resource names the module provisions for its role.
    pub objects: &'a [String],
    /// The stack-wide facts.
    pub stack: &'a StackFacts,
    /// A hosted provider's resource, as steps see it.
    pub hosted: Option<HostedStepView>,
}

/// The `shell` template filter: `value` as one shell word. A value holding a `${VAR}`
/// reference is double-quoted with `"`, `\` and `` ` `` escaped, so the shell expands the
/// reference and nothing else; any other value is single-quoted.
fn shell_quote(value: String) -> String {
    if value.contains("${") {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('"');
        for c in value.chars() {
            if matches!(c, '"' | '\\' | '`') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_expands_only_references() {
        assert_eq!(shell_quote("us-east-1".into()), "'us-east-1'");
        assert_eq!(shell_quote("it's".into()), r"'it'\''s'");
        assert_eq!(shell_quote("${PGHOST}".into()), "\"${PGHOST}\"");
        assert_eq!(shell_quote("a`b\"${X}".into()), "\"a\\`b\\\"${X}\"");
    }

    #[test]
    fn a_step_with_empty_instructions_is_dropped() {
        let ctx = DeployCtx {
            env_name: "lh",
            target: &TargetId::compose(),
            module: None,
            knobs: ResolvedKnobs::empty_ref(),
            objects: &["a".into(), "b".into()],
            stack: &StackFacts::default(),
            hosted: None,
        };
        let step = DeployStepSpec::new(
            "x",
            DeployPhase::Provision,
            "Make {{ objects | length }}",
            "{% if hosted %}never{% endif %}",
        );
        assert_eq!(step.render(&ctx).unwrap(), None);
        let step = DeployStepSpec::new(
            "x",
            DeployPhase::Provision,
            "Make {{ objects | length }}",
            "For `{{ env_name }}`.",
        )
        .with_script("for o in {% for o in objects %}{{ o | shell }} {% endfor %}; do :; done\n");
        let rendered = step.render(&ctx).unwrap().unwrap();
        assert_eq!(rendered.title, "Make 2");
        assert_eq!(rendered.instructions, "For `lh`.");
        assert_eq!(
            rendered.script.as_deref(),
            Some("for o in 'a' 'b' ; do :; done\n")
        );
    }
}
