//! Resolve the active component set: evaluate each root's `components:` `when:`
//! expressions, load each component's manifest, transitively resolve
//! `depends_on`, and order the result dependencies-first.
//!
//! Resolution runs over **multiple roots**: one base template (e.g.
//! `_base/lakehouse/`) plus zero or more apps (e.g. `_apps/databricks-app-rust/`).
//! Each component is resolved against the root that declares it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use minijinja::Value;

use crate::error::{Error, Result};

use super::loader::{LoadedComponent, load_local_component};
use super::manifest::{Component, Manifest};
use super::render::Renderer;

/// A resolved component, loaded from disk.
pub struct ResolvedComponent {
    pub loaded: LoadedComponent,
}

impl ResolvedComponent {
    pub fn name(&self) -> &str {
        &self.loaded.manifest.name
    }
}

/// One template root (base or app) participating in a scaffold.
///
/// The base is always first; apps follow. Components declared in a root's
/// `components:` block are resolved against that root's directory.
#[derive(Clone, Copy)]
pub struct ScaffoldRoot<'a> {
    pub root: &'a Path,
    pub manifest: &'a Manifest,
}

/// Inputs to component resolution.
pub struct ResolveInput<'a> {
    /// Base + apps, in render order.
    pub roots: &'a [ScaffoldRoot<'a>],

    /// Variable context used to evaluate `when:` expressions.
    pub vars: &'a Value,
}

/// Resolve the active set of components for this scaffold run.
///
/// Every root's `components:` whose `when:` evaluates truthy (or has no `when:`)
/// is enabled, in declaration order, together with the transitive `depends_on`
/// closure. The returned vector is topologically sorted (dependencies first).
pub fn resolve_components(
    input: ResolveInput<'_>,
    renderer: &Renderer,
) -> Result<Vec<ResolvedComponent>> {
    let mut requested: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for root in input.roots {
        for c in &root.manifest.components {
            let when_ok = match &c.when {
                Some(expr) => renderer.eval_when(expr, input.vars)?,
                None => true,
            };
            if when_ok && seen.insert(c.name.clone()) {
                requested.push(c.name.clone());
            }
        }
    }

    // Build a name → (declared component, owning root) map for path lookups.
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
        if let Some(rc) = loaded.remove(&name)
            && emitted.insert(name)
        {
            out.push(rc);
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
    // A component is recorded in `loaded` before its dependencies are walked, so check
    // the in-progress stack first or a cycle would look like an already-loaded component.
    if stack.iter().any(|n| n == name) {
        return Err(Error::DependencyCycle(name.to_string()));
    }
    if loaded.contains_key(name) {
        return Ok(());
    }
    stack.push(name.to_string());

    let component = match declared.get(name) {
        Some((c, owner_root)) => {
            let path = c
                .path
                .clone()
                .unwrap_or_else(|| format!("components/{name}"));
            load_local_component(owner_root.join(path))?
        }
        None => {
            // Not declared (a `depends_on` target): look in each declaring root's
            // `components/<name>` directory.
            let found = declared
                .values()
                .map(|(_, root)| root.join(format!("components/{name}")))
                .find(|dir| dir.is_dir());
            match found {
                Some(dir) => load_local_component(dir)?,
                None => {
                    stack.pop();
                    return Err(Error::ComponentNotFound {
                        name: name.to_string(),
                    });
                }
            }
        }
    };
    let resolved = ResolvedComponent { loaded: component };

    let deps = resolved.loaded.manifest.depends_on.clone();
    loaded.insert(name.to_string(), resolved);

    for d in deps {
        load_one(&d, declared, loaded, order_hint, stack)?;
    }

    order_hint.push(name.to_string());
    stack.pop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Write `components/<name>/template.yaml` under `root`.
    fn component(root: &Path, name: &str, depends_on: &[&str]) {
        let dir = root.join("components").join(name);
        fs::create_dir_all(&dir).unwrap();
        let deps = depends_on.join(", ");
        fs::write(
            dir.join("template.yaml"),
            format!("name: {name}\ndepends_on: [{deps}]\n"),
        )
        .unwrap();
    }

    fn manifest(yaml: &str) -> Manifest {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn resolve(root: &Path, m: &Manifest, vars: serde_json::Value) -> Result<Vec<String>> {
        let roots = [ScaffoldRoot { root, manifest: m }];
        let vars = Value::from_serialize(&vars);
        let input = ResolveInput {
            roots: &roots,
            vars: &vars,
        };
        Ok(resolve_components(input, &Renderer::new())?
            .iter()
            .map(|c| c.name().to_string())
            .collect())
    }

    #[test]
    fn when_filters_components() {
        let tmp = tempfile::tempdir().unwrap();
        component(tmp.path(), "a", &[]);
        component(tmp.path(), "b", &[]);
        let m = manifest(
            "name: t\ncomponents:\n  - { name: a, when: with_a }\n  - { name: b, when: with_b }\n",
        );
        let names = resolve(
            tmp.path(),
            &m,
            serde_json::json!({ "with_a": true, "with_b": false }),
        )
        .unwrap();
        assert_eq!(names, ["a"]);
    }

    #[test]
    fn depends_on_is_pulled_in_dependencies_first() {
        let tmp = tempfile::tempdir().unwrap();
        component(tmp.path(), "app", &["lib"]);
        component(tmp.path(), "lib", &[]);
        let m = manifest("name: t\ncomponents:\n  - { name: app }\n");
        let names = resolve(tmp.path(), &m, serde_json::json!({})).unwrap();
        assert_eq!(names, ["lib", "app"]);
    }

    #[test]
    fn missing_component_errors() {
        let tmp = tempfile::tempdir().unwrap();
        component(tmp.path(), "app", &["ghost"]);
        let m = manifest("name: t\ncomponents:\n  - { name: app }\n");
        let err = resolve(tmp.path(), &m, serde_json::json!({})).unwrap_err();
        assert!(matches!(err, Error::ComponentNotFound { name } if name == "ghost"));
    }

    #[test]
    fn dependency_cycle_errors() {
        let tmp = tempfile::tempdir().unwrap();
        component(tmp.path(), "a", &["b"]);
        component(tmp.path(), "b", &["a"]);
        let m = manifest("name: t\ncomponents:\n  - { name: a }\n");
        let err = resolve(tmp.path(), &m, serde_json::json!({})).unwrap_err();
        assert!(matches!(err, Error::DependencyCycle(_)));
    }

    #[test]
    fn removed_manifest_keys_are_rejected() {
        for yaml in [
            "name: t\nalways: [x]\n",
            "name: t\nprofiles: { small: [x] }\n",
            "name: t\nlakehouse_requires: { hard: [x] }\n",
            "name: t\ncomponents:\n  - { name: x, kind: shared }\n",
        ] {
            assert!(serde_yaml::from_str::<Manifest>(yaml).is_err(), "{yaml}");
        }
        for yaml in [
            "name: x\nprovides: {}\n",
            "name: x\ncategory: storage\n",
            "name: x\nconflicts_with: [y]\n",
        ] {
            assert!(
                serde_yaml::from_str::<crate::template::ComponentManifest>(yaml).is_err(),
                "{yaml}"
            );
        }
    }
}
