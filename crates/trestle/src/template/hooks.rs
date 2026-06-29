//! Execute `post_init:` hooks declared in a template manifest.
//!
//! Hooks are simple shell commands run with `sh -c` in the freshly-rendered
//! destination directory. Hooks with `confirm: true` prompt the user; hooks with
//! a `when:` expression are skipped if it evaluates falsy.

use std::path::Path;
use std::process::Command;

use minijinja::Value;

use crate::error::{Error, Result};

use super::manifest::PostInitHook;
use super::render::Renderer;

/// Run each post-init hook in declaration order.
///
/// `non_interactive`: when true, hooks with `confirm: true` are *skipped* (we
/// never auto-run side-effecting hooks the user hasn't seen).
pub fn run_post_init(
    hooks: &[PostInitHook],
    dest: &Path,
    ctx: &Value,
    renderer: &Renderer,
    non_interactive: bool,
) -> Result<()> {
    for hook in hooks {
        if let Some(expr) = &hook.when
            && !renderer.eval_when(expr, ctx)?
        {
            continue;
        }

        let description = hook.description.clone().unwrap_or_else(|| hook.run.clone());

        if hook.confirm {
            if non_interactive {
                tracing::info!("skipping confirm-required hook: {description}");
                continue;
            }
            let ok = cliclack::confirm(format!("Run: {description}?"))
                .initial_value(true)
                .interact()
                .map_err(|e| Error::other(format!("hook prompt failed: {e}")))?;
            if !ok {
                continue;
            }
        }

        let rendered = renderer.render_str(&hook.run, ctx, &format!("post_init:{description}"))?;
        let status = Command::new("sh")
            .arg("-c")
            .arg(&rendered)
            .current_dir(dest)
            .status()
            .map_err(|e| Error::Hook(format!("failed to spawn `sh`: {e}")))?;
        if !status.success() {
            return Err(Error::Hook(format!(
                "hook exited with status {status}: {rendered}"
            )));
        }
    }
    Ok(())
}
