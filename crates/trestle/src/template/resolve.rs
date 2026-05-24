//! Resolve the active component set: apply `--profile`, `--with`, and `when:`
//! filters; load each component's manifest; transitively resolve `depends_on`;
//! topologically sort the result so context aggregation runs in dependency order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use minijinja::Value;

use crate::error::{Error, Result};

use super::loader::{LoadedComponent, load_local_component, load_shared_component};
use super::manifest::{Component, ComponentKind, Manifest};
use super::render::Renderer;

/// A resolved component, loaded from disk and tagged with its declared `kind`.
pub struct ResolvedComponent {
    pub kind: ComponentKind,
    pub loaded: LoadedComponent,
}

impl ResolvedComponent {
    pub fn name(&self) -> &str {
        &self.loaded.manifest.name
    }
}

/// Inputs to component resolution.
pub struct ResolveInput<'a> {
    pub template_root: &'a Path,
    pub manifest: &'a Manifest,
    pub vars: &'a Value,
    pub profile: Option<&'a str>,
    pub extra_with: &'a [String],
}

/// Resolve the active set of components for this scaffold run.
///
/// Order of inclusion:
/// 1. Components matching `--profile <name>` (if any), in profile-declaration order.
/// 2. Components matching `--with <name>` (if any), in CLI order, append-only.
/// 3. Components from the template manifest's `components:` block whose `when:`
///    expression evaluates truthy.
/// 4. Recursive `depends_on` closure for each of the above.
///
/// The returned vector is topologically sorted (dependencies first).
pub fn resolve_components(
    input: ResolveInput<'_>,
    renderer: &Renderer,
) -> Result<Vec<ResolvedComponent>> {
    let mut requested: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    // 1. profile
    if let Some(profile) = input.profile {
        if let Some(list) = input.manifest.profiles.get(profile) {
            for name in list {
                if seen.insert(name.clone()) {
                    requested.push(name.clone());
                }
            }
        } else if !profile.is_empty() {
            return Err(Error::ProfileNotFound {
                template: input.manifest.name.clone(),
                name: profile.to_string(),
            });
        }
    }

    // 2. --with
    for name in input.extra_with {
        if seen.insert(name.clone()) {
            requested.push(name.clone());
        }
    }

    // 3. manifest components matching `when:`
    for c in &input.manifest.components {
        let when_ok = match &c.when {
            Some(expr) => renderer.eval_when(expr, input.vars)?,
            None => true,
        };
        if !when_ok {
            continue;
        }
        if seen.insert(c.name.clone()) {
            requested.push(c.name.clone());
        }
    }

    // Map requested names → declared component (for kind/path); shared components are
    // recognised by name if they're in the embedded library, even if not declared
    // explicitly.
    let declared: BTreeMap<String, &Component> = input
        .manifest
        .components
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();

    // Load each requested component (and its dependency closure).
    let mut loaded: BTreeMap<String, ResolvedComponent> = BTreeMap::new();
    let mut order_hint: Vec<String> = Vec::new();

    fn load_one(
        name: &str,
        template_root: &Path,
        declared: &BTreeMap<String, &Component>,
        loaded: &mut BTreeMap<String, ResolvedComponent>,
        order_hint: &mut Vec<String>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        if loaded.contains_key(name) {
            return Ok(());
        }
        if stack.iter().any(|n| n == name) {
            return Err(Error::DependencyCycle(name.to_string()));
        }
        stack.push(name.to_string());

        let resolved = match declared.get(name) {
            Some(c) if matches!(c.kind, ComponentKind::Local) => {
                let path = c
                    .path
                    .clone()
                    .unwrap_or_else(|| format!("components/{name}"));
                let root = template_root.join(&path);
                let component = load_local_component(root)?;
                ResolvedComponent {
                    kind: ComponentKind::Local,
                    loaded: component,
                }
            }
            Some(c) if matches!(c.kind, ComponentKind::Shared) => {
                let component = load_shared_component(name)?;
                ResolvedComponent {
                    kind: ComponentKind::Shared,
                    loaded: component,
                }
            }
            None => {
                // Not declared: try shared library first, then fall back to a
                // local component directory of the same name.
                match load_shared_component(name) {
                    Ok(component) => ResolvedComponent {
                        kind: ComponentKind::Shared,
                        loaded: component,
                    },
                    Err(_) => {
                        let root = template_root.join(format!("components/{name}"));
                        if root.is_dir() {
                            let component = load_local_component(root)?;
                            ResolvedComponent {
                                kind: ComponentKind::Local,
                                loaded: component,
                            }
                        } else {
                            stack.pop();
                            return Err(Error::ComponentNotFound {
                                name: name.to_string(),
                            });
                        }
                    }
                }
            }
            Some(_) => unreachable!("kind pattern is exhaustive"),
        };

        let deps = resolved.loaded.manifest.depends_on.clone();
        loaded.insert(name.to_string(), resolved);

        for d in deps {
            load_one(&d, template_root, declared, loaded, order_hint, stack)?;
        }

        order_hint.push(name.to_string());
        stack.pop();
        Ok(())
    }

    let mut stack: Vec<String> = Vec::new();
    for name in &requested {
        load_one(
            name,
            input.template_root,
            &declared,
            &mut loaded,
            &mut order_hint,
            &mut stack,
        )?;
    }

    // Topological order: `order_hint` is post-order (deps emitted before their parents),
    // which is exactly what we want for context aggregation (deps' contributions
    // appear first).
    let mut out: Vec<ResolvedComponent> = Vec::with_capacity(order_hint.len());
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    for name in order_hint {
        if let Some(rc) = loaded.remove(&name) {
            if emitted.insert(name) {
                out.push(rc);
            }
        }
    }

    Ok(out)
}
