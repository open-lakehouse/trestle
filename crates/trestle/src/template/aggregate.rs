//! Aggregate `provides:` declarations from the active component set into a single
//! `stack.*` context exposed to renderers.
//!
//! Each component contributes typed fragments (envoy routes, postgres databases,
//! S3 buckets, env vars, compose include paths, ports, free-form extras). The
//! [`aggregate_stack_context`] function walks the active components in
//! dependency-resolved order and concatenates the fragments, deduplicating where
//! semantically appropriate (e.g. postgres database names) but preserving order
//! everywhere else (e.g. envoy routes — order determines match priority).
//!
//! See the design note in `docs/templates.md` (added by `docs-and-workspace`) for
//! the rationale; the key idea is that this avoids any YAML-merge logic at all.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::manifest::{ComponentManifest, EnvoyCluster, EnvoyRoute, PortDecl, Provides};

/// The aggregated template context, exposed in renderers as `stack`.
#[derive(Debug, Default, Clone, Serialize)]
pub struct StackContext {
    pub components: Vec<String>,
    pub compose_includes: Vec<String>,
    pub postgres_databases: Vec<String>,
    pub s3_buckets: Vec<String>,
    pub envoy_routes: Vec<EnvoyRouteCtx>,
    pub envoy_clusters: Vec<EnvoyClusterCtx>,
    pub env_vars: BTreeMap<String, String>,
    pub ports: Vec<PortCtx>,
    /// Free-form per-component extras, flattened from every active component's
    /// [`Provides::extras`](crate::template::Provides) block.
    ///
    /// Each key is namespaced as `<component_name>__<key>` (component name and
    /// the original extras key, joined by a double underscore) so contributions
    /// from different components never collide. For example, a component named
    /// `metastore` declaring `extras: { schema: bronze }` appears here as:
    ///
    /// ```text
    /// { "metastore__schema": "bronze" }
    /// ```
    ///
    /// and is read in a template as `{{ stack.extras["metastore__schema"] }}`.
    pub extras: BTreeMap<String, serde_yaml::Value>,
}

/// Public re-export of the [`Provides::envoy_routes`] shape, flattened so MiniJinja
/// templates always see `r.rewrite` as a (possibly empty) string instead of
/// `Option<String>`.
#[derive(Debug, Clone, Serialize)]
pub struct EnvoyRouteCtx {
    pub prefix: String,
    pub cluster: String,
    pub rewrite: String,
}

/// Public re-export of [`Provides::envoy_clusters`].
#[derive(Debug, Clone, Serialize)]
pub struct EnvoyClusterCtx {
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// Public re-export of [`Provides::ports`].
#[derive(Debug, Clone, Serialize)]
pub struct PortCtx {
    pub name: String,
    pub default: u16,
    pub internal_only: bool,
}

/// A host-port claimed by more than one component.
///
/// Two components may declare ports under different [`names`](PortDecl::name) yet
/// map to the same host port number; deduping by name alone (see
/// [`aggregate_stack_context`]) does not catch that, and the conflict only
/// surfaces at `docker compose up`. [`port_collisions`] reports these so callers
/// can fail fast with an actionable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortCollision {
    /// The host port claimed more than once.
    pub port: u16,
    /// The component + port-name pairs that claim it, in aggregation order.
    pub claimants: Vec<(String, String)>,
}

/// Detect host-port collisions across the active components.
///
/// Only host-published ports participate: [`internal_only`](PortDecl::internal_only)
/// ports are container-network-local and never bind a host port, so they cannot
/// collide. Returns one [`PortCollision`] per contended host port (empty when the
/// stack is conflict-free).
pub fn port_collisions(components: &[&ComponentManifest]) -> Vec<PortCollision> {
    // port number -> claimants, preserving first-seen order of the ports.
    let mut by_port: BTreeMap<u16, Vec<(String, String)>> = BTreeMap::new();
    let mut seen_components: BTreeSet<String> = BTreeSet::new();
    for c in components {
        // Mirror aggregate's component dedupe so a component listed twice (e.g.
        // pulled in via two dependency paths) isn't counted as self-colliding.
        if !seen_components.insert(c.name.clone()) {
            continue;
        }
        for p in &c.provides.ports {
            if p.internal_only {
                continue;
            }
            by_port
                .entry(p.default)
                .or_default()
                .push((c.name.clone(), p.name.clone()));
        }
    }
    by_port
        .into_iter()
        .filter(|(_, claimants)| claimants.len() > 1)
        .map(|(port, claimants)| PortCollision { port, claimants })
        .collect()
}

/// Aggregate `provides:` blocks from a list of (already topologically sorted)
/// component manifests into a single [`StackContext`].
pub fn aggregate_stack_context(components: &[&ComponentManifest]) -> StackContext {
    let mut out = StackContext::default();

    // Track seen names to dedupe order-preserving lists.
    let mut seen_components: BTreeSet<String> = BTreeSet::new();
    let mut seen_includes: BTreeSet<String> = BTreeSet::new();
    let mut seen_dbs: BTreeSet<String> = BTreeSet::new();
    let mut seen_buckets: BTreeSet<String> = BTreeSet::new();
    let mut seen_clusters: BTreeSet<String> = BTreeSet::new();
    let mut seen_ports: BTreeSet<String> = BTreeSet::new();

    for c in components {
        if !seen_components.insert(c.name.clone()) {
            continue;
        }
        out.components.push(c.name.clone());
        absorb(&mut out, &c.provides, &c.name);

        for s in &c.provides.compose_includes {
            if seen_includes.insert(s.clone()) {
                out.compose_includes.push(s.clone());
            }
        }
        for db in &c.provides.postgres_databases {
            if seen_dbs.insert(db.clone()) {
                out.postgres_databases.push(db.clone());
            }
        }
        for b in &c.provides.s3_buckets {
            if seen_buckets.insert(b.clone()) {
                out.s3_buckets.push(b.clone());
            }
        }
        for r in &c.provides.envoy_routes {
            out.envoy_routes.push(flatten_route(r));
        }
        for cl in &c.provides.envoy_clusters {
            if seen_clusters.insert(cl.name.clone()) {
                out.envoy_clusters.push(flatten_cluster(cl));
            }
        }
        for p in &c.provides.ports {
            if seen_ports.insert(p.name.clone()) {
                out.ports.push(flatten_port(p));
            }
        }
        // env_vars: last writer wins (sorted BTreeMap means the lexicographically
        // last contributor takes precedence, but that's almost never the desired
        // semantics — apply in topological order instead).
        for (k, v) in &c.provides.env_vars {
            out.env_vars.insert(k.clone(), v.clone());
        }
        for (k, v) in &c.provides.extras {
            out.extras.insert(format!("{}__{}", c.name, k), v.clone());
        }
    }

    out
}

fn absorb(_out: &mut StackContext, _provides: &Provides, _component: &str) {
    // Reserved hook for cross-key derivations.
}

fn flatten_route(r: &EnvoyRoute) -> EnvoyRouteCtx {
    EnvoyRouteCtx {
        prefix: r.prefix.clone(),
        cluster: r.cluster.clone(),
        rewrite: r.rewrite.clone().unwrap_or_default(),
    }
}

fn flatten_cluster(c: &EnvoyCluster) -> EnvoyClusterCtx {
    EnvoyClusterCtx {
        name: c.name.clone(),
        host: c.host.clone(),
        port: c.port,
    }
}

fn flatten_port(p: &PortDecl) -> PortCtx {
    PortCtx {
        name: p.name.clone(),
        default: p.default,
        internal_only: p.internal_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(yaml: &str) -> ComponentManifest {
        serde_yaml::from_str(yaml).expect("valid component manifest")
    }

    #[test]
    fn no_collision_when_ports_differ() {
        let a = manifest("name: a\nprovides:\n  ports:\n    - { name: a_http, default: 5432 }\n");
        let b = manifest("name: b\nprovides:\n  ports:\n    - { name: b_http, default: 8081 }\n");
        assert!(port_collisions(&[&a, &b]).is_empty());
    }

    #[test]
    fn detects_same_host_port_under_different_names() {
        let a = manifest("name: a\nprovides:\n  ports:\n    - { name: a_http, default: 8080 }\n");
        let b = manifest("name: b\nprovides:\n  ports:\n    - { name: b_http, default: 8080 }\n");
        let collisions = port_collisions(&[&a, &b]);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].port, 8080);
        assert_eq!(
            collisions[0].claimants,
            vec![
                ("a".to_string(), "a_http".to_string()),
                ("b".to_string(), "b_http".to_string()),
            ]
        );
    }

    #[test]
    fn internal_only_ports_never_collide() {
        let a = manifest(
            "name: a\nprovides:\n  ports:\n    - { name: a_int, default: 9000, internal_only: true }\n",
        );
        let b = manifest(
            "name: b\nprovides:\n  ports:\n    - { name: b_int, default: 9000, internal_only: true }\n",
        );
        assert!(port_collisions(&[&a, &b]).is_empty());
    }

    #[test]
    fn same_component_listed_twice_is_not_self_collision() {
        let a = manifest("name: a\nprovides:\n  ports:\n    - { name: a_http, default: 8080 }\n");
        assert!(port_collisions(&[&a, &a]).is_empty());
    }
}
