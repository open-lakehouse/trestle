//! Pure dependency resolution: a module selection → a resolved, ordered graph.
//!
//! [`resolve`] is side-effect-free. It expands a selection over the
//! [`requires`](crate::Module::requires) edges in a catalog, errors on unknown ids,
//! conflicts, and cycles, and produces a [`ResolvedGraph`] whose `nodes` are
//! topologically ordered — dependencies before dependents, i.e. a valid Compose
//! `depends_on` / startup order. The graph is the resolver's natural working
//! representation; returning it (rather than a flat list) leaves the data shaped for
//! the planner and for a node-diagram visualization at near-zero extra cost.
//!
//! [`conflicts_with`](crate::Module::conflicts_with) is validated here, alongside the
//! dependency edges, so a selection that pulls in two mutually-exclusive modules fails at
//! resolution rather than surfacing later at render or launch.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::catalog::module::{Module, ModuleId};

/// Extra dependency edges supplied alongside a catalog: `consumer → [providers]`, ordered
/// like `requires` (each provider starts before the consumer). The planner passes the
/// demand→provider edges here so a resource provider is pulled into the graph and ordered
/// before its consumers, without mutating any module's own `requires`.
pub type ExtraEdges = BTreeMap<ModuleId, Vec<ModuleId>>;

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
///
/// `nodes` are `Arc<dyn Module>` (cloned cheaply from the catalog), so the graph is not
/// serializable or comparable — its modules may be hand-written logic types, not data.
#[derive(Clone)]
pub struct ResolvedGraph {
    /// All modules to run, ordered dependencies-first (valid startup order).
    pub nodes: Vec<Arc<dyn Module>>,
    /// Dependency edges among `nodes`.
    pub edges: Vec<Edge>,
}

impl std::fmt::Debug for ResolvedGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedGraph")
            .field("nodes", &crate::catalog::module::module_ids(&self.nodes))
            .field("edges", &self.edges)
            .finish()
    }
}

impl ResolvedGraph {
    /// Look up a resolved module by id.
    pub fn module(&self, id: &ModuleId) -> Option<&Arc<dyn Module>> {
        self.nodes.iter().find(|m| m.id() == id)
    }
}

/// Resolve a module selection into an ordered graph against the given catalog
/// modules, with no extra edges.
///
/// Expands the transitive `requires` closure, then validates conflicts and
/// topologically sorts. Working over sorted sets/maps keeps the output deterministic
/// regardless of selection order.
pub fn resolve(
    selected: &[ModuleId],
    catalog: &[Arc<dyn Module>],
) -> Result<ResolvedGraph, ResolveError> {
    resolve_with(selected, catalog, &ExtraEdges::new())
}

/// Resolve against an explicit module list plus `extra_edges` (`consumer → [providers]`)
/// that are treated exactly like `requires`: each provider is pulled into the closure and
/// ordered before its consumer. The planner uses this to thread demand→provider ordering
/// without mutating any module.
pub fn resolve_with(
    selected: &[ModuleId],
    catalog: &[Arc<dyn Module>],
    extra_edges: &ExtraEdges,
) -> Result<ResolvedGraph, ResolveError> {
    let by_id: BTreeMap<&str, &Arc<dyn Module>> =
        catalog.iter().map(|m| (m.id().as_str(), m)).collect();

    // The combined `requires` of a module: its own plus any `extra_edges` for it.
    let deps_of = |id: &ModuleId, module: &Arc<dyn Module>| -> Vec<ModuleId> {
        let mut deps: Vec<ModuleId> = module.requires().to_vec();
        if let Some(extra) = extra_edges.get(id) {
            for p in extra {
                if !deps.contains(p) {
                    deps.push(p.clone());
                }
            }
        }
        deps
    };

    // Transitive closure of the selection over `requires` + `extra_edges` (BFS), erroring
    // on any unknown id encountered along the way.
    let mut included: BTreeSet<ModuleId> = BTreeSet::new();
    let mut queue: Vec<ModuleId> = selected.to_vec();
    while let Some(id) = queue.pop() {
        let module = by_id
            .get(id.as_str())
            .ok_or_else(|| ResolveError::UnknownModule(id.clone()))?;
        if included.insert(id.clone()) {
            for dep in deps_of(&id, module) {
                queue.push(dep);
            }
        }
    }

    check_conflicts(&included, &by_id)?;

    let edges = collect_edges(&included, &by_id, extra_edges);
    let nodes = topo_sort(&included, &edges)?
        .into_iter()
        .map(|id| Arc::clone(*by_id.get(id.as_str()).unwrap()))
        .collect();

    Ok(ResolvedGraph { nodes, edges })
}

/// Reject the selection if any two included modules declare a `conflicts_with`
/// relationship (in either direction). Reports the pair with ids sorted so the error
/// is order-independent.
fn check_conflicts(
    included: &BTreeSet<ModuleId>,
    by_id: &BTreeMap<&str, &Arc<dyn Module>>,
) -> Result<(), ResolveError> {
    for id in included {
        let module = by_id.get(id.as_str()).unwrap();
        for other in module.conflicts_with() {
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

/// Collect the dependency edges (`from` requires `to`) within the included set — each
/// module's own `requires` plus any `extra_edges` for it — in a deterministic order.
fn collect_edges(
    included: &BTreeSet<ModuleId>,
    by_id: &BTreeMap<&str, &Arc<dyn Module>>,
    extra_edges: &ExtraEdges,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    for id in included {
        let module = by_id.get(id.as_str()).unwrap();
        let mut deps: Vec<ModuleId> = module.requires().to_vec();
        if let Some(extra) = extra_edges.get(id) {
            for p in extra {
                if !deps.contains(p) {
                    deps.push(p.clone());
                }
            }
        }
        for dep in deps {
            if included.contains(&dep) {
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
    use crate::catalog::module::DataModule;

    /// A bare module with just an id and `requires` — enough to exercise the graph.
    fn m(id: &str, requires: &[&str]) -> Arc<dyn Module> {
        m_conflicts(id, requires, &[])
    }

    /// A bare module with an id, `requires`, and `conflicts_with`.
    fn m_conflicts(id: &str, requires: &[&str], conflicts: &[&str]) -> Arc<dyn Module> {
        Arc::new(DataModule {
            id: id.into(),
            display_name: None,
            summary: None,
            category: None,
            provider_of: None,
            requires: requires.iter().map(|r| ModuleId::from(*r)).collect(),
            conflicts_with: conflicts.iter().map(|c| ModuleId::from(*c)).collect(),
            needs: Vec::new(),
            service_specs: Vec::new(),
            provides: Default::default(),
            knobs: Vec::new(),
            render: Default::default(),
        })
    }

    fn ids(g: &ResolvedGraph) -> Vec<String> {
        g.nodes.iter().map(|n| n.id().0.clone()).collect()
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
        // The graph is no longer `Eq` (its nodes are trait objects); resolution order
        // independence is what this asserts, so compare the ordered ids and edges.
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(a.edges, b.edges);
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
        let azurite = m_conflicts("azurite", &[], &["seaweedfs"]);
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
