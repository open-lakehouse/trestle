//! Pure dependency resolution: a module selection → a resolved, ordered graph.
//!
//! [`resolve`] is side-effect-free. It expands a selection over the
//! [`requires`](crate::Module::requires) edges in a catalog, errors on unknown ids,
//! conflicts, and cycles, and produces a [`ResolvedGraph`] whose `nodes` are
//! topologically ordered — dependencies before dependents, i.e. a valid Compose
//! `depends_on` / startup order. The graph is the resolver's natural working
//! representation; returning it (rather than a flat list) leaves the data shaped for
//! the planner and for a future node-diagram visualization at near-zero extra cost.
//!
//! This is the topology crate's port of hydrofoil's `env-modules` resolver, adapted
//! to this crate's [`Module`]/[`ModuleId`] and extended with
//! [`conflicts_with`](crate::Module::conflicts_with) validation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::module::{Module, ModuleId};

/// What can go wrong resolving a selection.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    /// A selected (or transitively required) id is not in the catalog.
    #[error("unknown module: {0}")]
    UnknownModule(ModuleId),
    /// Two modules in the resolved set declare a [`conflicts_with`] relationship.
    ///
    /// [`conflicts_with`]: crate::Module::conflicts_with
    #[error("conflicting modules selected: `{a}` conflicts with `{b}`")]
    Conflict {
        /// One side of the conflict (the lexicographically smaller id).
        a: ModuleId,
        /// The other side.
        b: ModuleId,
    },
    /// A dependency cycle was detected involving these modules (sorted).
    #[error("dependency cycle among modules: {0:?}")]
    Cycle(Vec<ModuleId>),
}

/// A directed dependency edge: `from` requires `to` (so `to` starts first).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// The dependent module.
    pub from: ModuleId,
    /// The module it depends on.
    pub to: ModuleId,
}

/// The resolved environment graph: the full set of modules to run (transitive
/// closure of the selection), topologically ordered, plus the dependency edges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraph {
    /// All modules to run, ordered dependencies-first (valid startup order).
    pub nodes: Vec<Module>,
    /// Dependency edges among `nodes`.
    pub edges: Vec<Edge>,
}

impl ResolvedGraph {
    /// Look up a resolved module by id.
    pub fn module(&self, id: &ModuleId) -> Option<&Module> {
        self.nodes.iter().find(|m| &m.id == id)
    }
}

/// Resolve a module selection into an ordered graph against the given catalog
/// modules.
///
/// Expands the transitive `requires` closure, then validates conflicts and
/// topologically sorts. Working over sorted sets/maps keeps the output deterministic
/// regardless of selection order.
pub fn resolve(selected: &[ModuleId], catalog: &[Module]) -> Result<ResolvedGraph, ResolveError> {
    resolve_with(selected, catalog)
}

/// Resolve against an explicit module list (the same as [`resolve`]; the second name
/// mirrors the hydrofoil API and reads well in tests).
pub fn resolve_with(
    selected: &[ModuleId],
    catalog: &[Module],
) -> Result<ResolvedGraph, ResolveError> {
    let by_id: BTreeMap<&str, &Module> = catalog.iter().map(|m| (m.id.as_str(), m)).collect();

    // Transitive closure of the selection over `requires` (BFS), erroring on any
    // unknown id encountered along the way.
    let mut included: BTreeSet<ModuleId> = BTreeSet::new();
    let mut queue: Vec<ModuleId> = selected.to_vec();
    while let Some(id) = queue.pop() {
        let module = by_id
            .get(id.as_str())
            .ok_or_else(|| ResolveError::UnknownModule(id.clone()))?;
        if included.insert(id.clone()) {
            for dep in &module.requires {
                queue.push(dep.clone());
            }
        }
    }

    check_conflicts(&included, &by_id)?;

    let edges = collect_edges(&included, &by_id);
    let nodes = topo_sort(&included, &edges)?
        .into_iter()
        .map(|id| (*by_id.get(id.as_str()).unwrap()).clone())
        .collect();

    Ok(ResolvedGraph { nodes, edges })
}

/// Reject the selection if any two included modules declare a `conflicts_with`
/// relationship (in either direction). Reports the pair with ids sorted so the error
/// is order-independent.
fn check_conflicts(
    included: &BTreeSet<ModuleId>,
    by_id: &BTreeMap<&str, &Module>,
) -> Result<(), ResolveError> {
    for id in included {
        let module = by_id.get(id.as_str()).unwrap();
        for other in &module.conflicts_with {
            if included.contains(other) {
                let (a, b) = if id <= other {
                    (id.clone(), other.clone())
                } else {
                    (other.clone(), id.clone())
                };
                return Err(ResolveError::Conflict { a, b });
            }
        }
    }
    Ok(())
}

/// Collect the dependency edges (`from` requires `to`) within the included set, in a
/// deterministic order.
fn collect_edges(included: &BTreeSet<ModuleId>, by_id: &BTreeMap<&str, &Module>) -> Vec<Edge> {
    let mut edges = Vec::new();
    for id in included {
        let module = by_id.get(id.as_str()).unwrap();
        for dep in &module.requires {
            if included.contains(dep) {
                edges.push(Edge {
                    from: id.clone(),
                    to: dep.clone(),
                });
            }
        }
    }
    edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    edges
}

/// Kahn topological sort: dependencies (edge `to`) ordered before dependents (edge
/// `from`). Returns the modules left in a cycle as a [`ResolveError::Cycle`].
fn topo_sort(included: &BTreeSet<ModuleId>, edges: &[Edge]) -> Result<Vec<ModuleId>, ResolveError> {
    // in_degree counts the unresolved dependencies of each node (its `requires`).
    let mut in_degree: BTreeMap<&ModuleId, usize> =
        included.iter().map(|id| (id, 0usize)).collect();
    // dependents[dep] = nodes that require `dep`, so we can decrement them once `dep`
    // is emitted.
    let mut dependents: BTreeMap<&ModuleId, Vec<&ModuleId>> = BTreeMap::new();
    for edge in edges {
        *in_degree.get_mut(&edge.from).unwrap() += 1;
        dependents.entry(&edge.to).or_default().push(&edge.from);
    }

    // Seed with dependency-free nodes (BTreeMap iteration → deterministic order).
    // Collected in reverse so the `Vec`-as-stack pops them in ascending id order.
    let mut ready: Vec<&ModuleId> = in_degree
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .rev()
        .collect();
    let mut ordered: Vec<ModuleId> = Vec::with_capacity(included.len());
    while let Some(id) = ready.pop() {
        ordered.push(id.clone());
        if let Some(deps) = dependents.get(id) {
            for &dependent in deps {
                let d = in_degree.get_mut(dependent).unwrap();
                *d -= 1;
                if *d == 0 {
                    ready.push(dependent);
                }
            }
        }
    }

    if ordered.len() != included.len() {
        // Whatever wasn't emitted is part of (or downstream of) a cycle.
        let emitted: BTreeSet<&ModuleId> = ordered.iter().collect();
        let mut stuck: Vec<ModuleId> = included
            .iter()
            .filter(|id| !emitted.contains(id))
            .cloned()
            .collect();
        stuck.sort();
        return Err(ResolveError::Cycle(stuck));
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::Module;

    /// A bare module with just an id and `requires` — enough to exercise the graph.
    fn m(id: &str, requires: &[&str]) -> Module {
        Module {
            id: id.into(),
            display_name: None,
            summary: None,
            category: None,
            provider_of: None,
            requires: requires.iter().map(|r| ModuleId::from(*r)).collect(),
            conflicts_with: Vec::new(),
            services: Vec::new(),
            provides: Default::default(),
            knobs: Vec::new(),
            render: Default::default(),
        }
    }

    fn ids(g: &ResolvedGraph) -> Vec<String> {
        g.nodes.iter().map(|n| n.id.0.clone()).collect()
    }

    #[test]
    fn pulls_in_transitive_requires() {
        let catalog = vec![
            m("mlflow", &["postgres", "seaweedfs", "envoy"]),
            m("postgres", &[]),
            m("seaweedfs", &[]),
            m("envoy", &[]),
        ];
        let g = resolve(&["mlflow".into()], &catalog).unwrap();
        for required in ["mlflow", "postgres", "seaweedfs", "envoy"] {
            assert!(
                ids(&g).contains(&required.to_string()),
                "missing {required}"
            );
        }
    }

    #[test]
    fn dependencies_ordered_before_dependents() {
        let catalog = vec![m("mlflow", &["postgres"]), m("postgres", &[])];
        let g = resolve(&["mlflow".into()], &catalog).unwrap();
        let order = ids(&g);
        let pg = order.iter().position(|x| x == "postgres").unwrap();
        let ml = order.iter().position(|x| x == "mlflow").unwrap();
        assert!(pg < ml, "postgres must come before mlflow: {order:?}");
    }

    #[test]
    fn order_is_deterministic_regardless_of_selection_order() {
        let catalog = vec![
            m("mlflow", &["postgres", "envoy"]),
            m("postgres", &[]),
            m("envoy", &[]),
        ];
        let a = resolve(&["mlflow".into(), "envoy".into()], &catalog).unwrap();
        let b = resolve(&["envoy".into(), "mlflow".into()], &catalog).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_module_errors() {
        let catalog = vec![m("postgres", &[])];
        let err = resolve(&["nope".into()], &catalog).unwrap_err();
        assert_eq!(err, ResolveError::UnknownModule("nope".into()));
    }

    #[test]
    fn missing_transitive_dependency_errors() {
        let catalog = vec![m("mlflow", &["postgres"])];
        let err = resolve(&["mlflow".into()], &catalog).unwrap_err();
        assert_eq!(err, ResolveError::UnknownModule("postgres".into()));
    }

    #[test]
    fn cycle_is_detected() {
        let catalog = vec![m("a", &["b"]), m("b", &["a"])];
        let err = resolve(&["a".into()], &catalog).unwrap_err();
        assert_eq!(err, ResolveError::Cycle(vec!["a".into(), "b".into()]));
    }

    #[test]
    fn conflict_is_rejected_with_sorted_pair() {
        let mut azurite = m("azurite", &[]);
        azurite.conflicts_with = vec!["seaweedfs".into()];
        let catalog = vec![azurite, m("seaweedfs", &[])];
        let err = resolve(&["azurite".into(), "seaweedfs".into()], &catalog).unwrap_err();
        assert_eq!(
            err,
            ResolveError::Conflict {
                a: "azurite".into(),
                b: "seaweedfs".into()
            }
        );
    }
}
