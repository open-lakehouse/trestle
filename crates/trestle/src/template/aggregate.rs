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
