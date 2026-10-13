//! Phase 5 — **render**: render every module from its binding and assemble the compose head.

use std::collections::BTreeMap;

use super::bind::Bound;
use super::resolve::ResolvedGraph;
use super::{ComposeInclude, ConfigDecl, HeadFile, PlanCtx, PlanError, SecretDecl};
use crate::catalog::module::{ModuleId, RenderError};
use crate::render::RenderOutput;

/// Render every module in dependency order and assemble the [`HeadFile`]. A module with an
/// empty fragment (e.g. the env-only contract module) contributes no compose include.
pub(super) fn render(
    graph: &ResolvedGraph,
    bound: &Bound,
    ctx: &PlanCtx,
) -> Result<(Vec<(ModuleId, RenderOutput)>, HeadFile), PlanError> {
    let mut renders = Vec::with_capacity(graph.nodes.len());
    let mut includes = Vec::new();
    // Aggregated `configs:` declarations and the module each alias was first seen on, so a
    // second module reusing an alias is rejected rather than silently shadowing.
    let mut configs: Vec<ConfigDecl> = Vec::new();
    let mut secrets: Vec<SecretDecl> = Vec::new();
    let mut alias_owner: BTreeMap<String, ModuleId> = BTreeMap::new();
    for module in &graph.nodes {
        let render_ctx = bound.bindings[module.id()].render_ctx(&bound.injected[module.id()]);
        let mut out = module
            .render()
            .render(&render_ctx)
            .map_err(|source| PlanError::Render {
                module: module.id().0.clone(),
                source,
            })?;
        // Root each emitted file under the module's own directory (`modules/<id>/<path>`),
        // unless the module marked it `at_root` (operator-facing files live beside
        // `compose.yaml`). The rewritten path is the single source of truth the consumer
        // writes to and the compose `configs: file:` / `secrets: file:` references.
        for f in &mut out.files {
            if !f.at_root {
                f.path = format!("modules/{}/{}", module.id().as_str(), f.path);
            }
            if f.alias.is_some() && f.secret_alias.is_some() {
                return Err(PlanError::Render {
                    module: module.id().0.clone(),
                    source: RenderError(
                        "a rendered file cannot be both a config and a secret".into(),
                    ),
                });
            }
            if let Some(alias) = &f.alias {
                claim_alias(&mut alias_owner, alias, module.id())?;
                configs.push(ConfigDecl {
                    alias: alias.clone(),
                    path: f.path.clone(),
                });
            }
            if let Some(alias) = &f.secret_alias {
                claim_alias(&mut alias_owner, alias, module.id())?;
                secrets.push(SecretDecl {
                    alias: alias.clone(),
                    path: f.path.clone(),
                });
            }
        }
        if !out.fragment.trim().is_empty() {
            includes.push(ComposeInclude {
                module: module.id().clone(),
                fragment: out.fragment.clone(),
            });
        }
        renders.push((module.id().clone(), out));
    }

    // Deterministic `configs:` order regardless of module/graph order.
    configs.sort_by(|a, b| a.alias.cmp(&b.alias));
    secrets.sort_by(|a, b| a.alias.cmp(&b.alias));

    let head = HeadFile {
        name: ctx.env_name.clone(),
        env: bound.env.clone(),
        includes,
        configs,
        secrets,
    };
    Ok((renders, head))
}

/// Record `module` as the owner of the top-level `alias`, rejecting a second owner: config and
/// secret aliases share one namespace.
fn claim_alias(
    owner: &mut BTreeMap<String, ModuleId>,
    alias: &str,
    module: &ModuleId,
) -> Result<(), PlanError> {
    if let Some(first) = owner.get(alias) {
        return Err(PlanError::ConfigAliasCollision {
            alias: alias.to_string(),
            first: first.clone(),
            second: module.clone(),
        });
    }
    owner.insert(alias.to_string(), module.clone());
    Ok(())
}
