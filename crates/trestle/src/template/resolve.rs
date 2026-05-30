//! Resolve the active component set: apply `--profile`, `--with`, the new
//! `--select`/`--app` channels, `when:` filters and `always:` baselines; load
//! each component's manifest; transitively resolve `depends_on`; topologically
//! sort the result so context aggregation runs in dependency order.
//!
//! Resolution now operates over **multiple roots** to support the base+apps
//! model: one base template (e.g. `_base/lakehouse/`) plus zero or more app
//! templates (e.g. `_apps/databricks-app-rust/`). Each root may contribute its
//! own `always:`, `lakehouse_requires:`, and (legacy) `components:` block, and
//! each local component is resolved against the root that declared it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

/// One template root (base or app) participating in a scaffold.
///
/// The base is always first; apps follow. Local components declared in a root's
/// `components:` block are resolved against that root's `components/` subtree.
#[derive(Clone, Copy)]
pub struct ScaffoldRoot<'a> {
    pub root: &'a Path,
    pub manifest: &'a Manifest,
}

/// Inputs to component resolution.
pub struct ResolveInput<'a> {
    /// Base + apps, in render order. The first entry is the base; the rest are
    /// apps. The resolver pulls `always:`, `lakehouse_requires:`, and the
    /// legacy `components:` block from every entry.
    pub roots: &'a [ScaffoldRoot<'a>],

    /// Variable + stack context used to evaluate `when:` expressions.
    pub vars: &'a Value,

    /// Component names explicitly selected by the user (wizard / `--select`).
    /// These layer on top of the baseline and `lakehouse_requires`.
    pub explicit_selections: &'a [String],

    /// Back-compat: `--profile <name>` against the *first* root's `profiles:` block.
    pub profile: Option<&'a str>,

    /// Back-compat: bare component names to enable from `--with`.
    pub extra_with: &'a [String],
}

/// Resolve the active set of components for this scaffold run.
///
/// Order of inclusion (deduped; first hit wins):
/// 1. Every root's `always:` list (the opinionated baseline).
/// 2. Explicit `--select` / wizard picks.
/// 3. App `lakehouse_requires.hard` and `.soft` lists, in app order.
/// 4. `--profile <name>` (against the first root's `profiles:` block).
/// 5. `--with <name>`.
/// 6. Each root's `components:` whose `when:` evaluates truthy.
/// 7. Recursive `depends_on` closure for everything above.
///
/// The returned vector is topologically sorted (dependencies first).
pub fn resolve_components(
    input: ResolveInput<'_>,
    renderer: &Renderer,
) -> Result<Vec<ResolvedComponent>> {
    let mut requested: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let push = |name: &str, requested: &mut Vec<String>, seen: &mut BTreeSet<String>| {
        if seen.insert(name.to_string()) {
            requested.push(name.to_string());
        }
    };

    // 1. always:
    for root in input.roots {
        for name in &root.manifest.always {
            push(name, &mut requested, &mut seen);
        }
    }

    // 2. explicit selections
    for name in input.explicit_selections {
        push(name, &mut requested, &mut seen);
    }

    // 3. lakehouse_requires from each app (skip the first root, which is the base)
    for root in input.roots.iter().skip(1) {
        for name in &root.manifest.lakehouse_requires.hard {
            push(name, &mut requested, &mut seen);
        }
        for name in &root.manifest.lakehouse_requires.soft {
            push(name, &mut requested, &mut seen);
        }
    }

    // 4. profile (against the base's profiles block) — legacy back-compat
    if let Some(profile) = input.profile {
        if let Some(first) = input.roots.first() {
            if let Some(list) = first.manifest.profiles.get(profile) {
                for name in list {
                    push(name, &mut requested, &mut seen);
                }
            } else if !profile.is_empty() {
                return Err(Error::ProfileNotFound {
                    template: first.manifest.name.clone(),
                    name: profile.to_string(),
                });
            }
        }
    }

    // 5. --with
    for name in input.extra_with {
        push(name, &mut requested, &mut seen);
    }

    // 6. Legacy `components:` blocks with `when:` from every root, in declaration order.
    for root in input.roots {
        for c in &root.manifest.components {
            let when_ok = match &c.when {
                Some(expr) => renderer.eval_when(expr, input.vars)?,
                None => true,
            };
            if !when_ok {
                continue;
            }
            push(&c.name, &mut requested, &mut seen);
        }
    }

    // Build a name → (declared component, owning root) map for local-path lookups.
    let mut declared: BTreeMap<String, (&Component, &Path)> = BTreeMap::new();
    for root in input.roots {
        for c in &root.manifest.components {
            declared.entry(c.name.clone()).or_insert((c, root.root));
        }
    }

    let mut loaded: BTreeMap<String, ResolvedComponent> = BTreeMap::new();
    let mut order_hint: Vec<String> = Vec::new();

    let mut stack: Vec<String> = Vec::new();
    for name in &requested {
        load_one(name, &declared, &mut loaded, &mut order_hint, &mut stack)?;
    }

    // Topological order: `order_hint` is post-order (deps emitted before parents).
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

fn load_one(
    name: &str,
    declared: &BTreeMap<String, (&Component, &Path)>,
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
        Some((c, owner_root)) if matches!(c.kind, ComponentKind::Local) => {
            let path = c
                .path
                .clone()
                .unwrap_or_else(|| format!("components/{name}"));
            let root: PathBuf = owner_root.join(&path);
            let component = load_local_component(root)?;
            ResolvedComponent {
                kind: ComponentKind::Local,
                loaded: component,
            }
        }
        Some((c, _owner_root)) if matches!(c.kind, ComponentKind::Shared) => {
            let component = load_shared_component(&c.name)?;
            ResolvedComponent {
                kind: ComponentKind::Shared,
                loaded: component,
            }
        }
        None => {
            // Not declared: try shared library first, then look in each
            // declared root's `components/<name>` directory.
            match load_shared_component(name) {
                Ok(component) => ResolvedComponent {
                    kind: ComponentKind::Shared,
                    loaded: component,
                },
                Err(_) => {
                    let mut found = None;
                    for (_, root) in declared.values() {
                        let root = root.join(format!("components/{name}"));
                        if root.is_dir() {
                            found = Some(load_local_component(root)?);
                            break;
                        }
                    }
                    match found {
                        Some(component) => ResolvedComponent {
                            kind: ComponentKind::Local,
                            loaded: component,
                        },
                        None => {
                            stack.pop();
                            return Err(Error::ComponentNotFound {
                                name: name.to_string(),
                            });
                        }
                    }
                }
            }
        }
        Some(_) => unreachable!("kind pattern is exhaustive"),
    };

    let deps = resolved.loaded.manifest.depends_on.clone();
    loaded.insert(name.to_string(), resolved);

    for d in deps {
        load_one(&d, declared, loaded, order_hint, stack)?;
    }

    order_hint.push(name.to_string());
    stack.pop();
    Ok(())
}
