//! Pure renderers that turn an [`EnvironmentPlan`](crate::EnvironmentPlan) into the
//! concrete text artifacts a Lakehouse dev environment is made of: the Envoy gateway
//! bootstrap, the Postgres init script, the `.env` overlay, and the top-level compose
//! file.
//!
//! These are the **stack-aggregated** outputs — the ones that, in trestle today, are
//! MiniJinja `{% for %}` loops over the aggregated `stack.*` lists. Here they are plain
//! functions over exactly what [`plan`](crate::plan) already computes
//! ([`GatewayConfig`], `postgres_databases`, the aggregated env, and the
//! [`HeadFile`]), so the crate is the single source of truth and the consuming tool
//! only writes the returned strings to disk.
//!
//! Everything here is pure and string-only — no I/O. The output shape is fixed and
//! hand-written (not serialized via a YAML library) to stay reviewable and stable.
//! Gated behind the `render` feature so the pure addressing core need not pull it in.

use std::fmt::Write as _;

use serde::Serialize;

use crate::plan_env::{EnvironmentPlan, GatewayConfig, HeadFile};
use crate::render::InjectedEnv;

/// The on-disk Envoy template, embedded at compile time. Lives in the crate-root
/// `templates/` directory (a sibling of `src/`) so it is reviewable as a plain file
/// rather than as inline string-building.
const ENVOY_TEMPLATE: &str = include_str!("../templates/envoy.yaml.jinja");

/// The flat, owned render context the Envoy template iterates. The ordering decisions
/// (longest-prefix-first routes, app cluster first) stay in Rust; the template only
/// renders what it is handed. The core plan types ([`GatewayConfig`] et al.) deliberately
/// do not derive `Serialize`, so this is the serialization boundary.
#[derive(Serialize)]
struct EnvoyCtx<'a> {
    internal_port: u16,
    routes: Vec<RouteCtx<'a>>,
    clusters: Vec<ClusterCtx<'a>>,
    app: bool,
}

#[derive(Serialize)]
struct RouteCtx<'a> {
    prefix: &'a str,
    cluster: &'a str,
    rewrite: Option<&'a str>,
}

#[derive(Serialize)]
struct ClusterCtx<'a> {
    name: &'a str,
    host: &'a str,
    port: u16,
}

/// Options the Envoy renderer needs beyond the [`GatewayConfig`] — chiefly the
/// optional default app upstream (the catch-all route + cluster) the gateway fronts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvoyOpts {
    /// The user's app upstream, if any: when set, a `/` catch-all route to an `app`
    /// cluster is emitted last, after all module routes.
    pub app: Option<AppUpstream>,
}

/// The default app upstream the gateway forwards unmatched requests to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppUpstream {
    /// The app's compose service / DNS name.
    pub service: String,
    /// The app's port.
    pub port: u16,
}

/// Render the Envoy bootstrap config for the plan's gateway.
///
/// Reproduces the Databricks-edge shape: one listener on the shared gateway's internal
/// port, an HTTP connection manager (`use_remote_address`, websocket upgrade), the
/// route table (each [`GatewayRoute`](crate::GatewayRoute) in order, with a
/// `regex_rewrite` block only when the route has a rewrite), an optional `/` app
/// catch-all last, the upstream clusters, and the admin endpoint.
pub fn render_envoy(gateway: &GatewayConfig, opts: &EnvoyOpts) -> String {
    // The shared listener (first) carries the multiplexed route table; dedicated
    // listeners (for fixed-path UIs) are not emitted here yet — the baseline has none.
    let shared = gateway.listeners.first();
    let internal_port = shared.map(|l| l.internal_port).unwrap_or(10000);

    let routes = shared
        .map(|l| l.routes.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|r| RouteCtx {
            prefix: &r.prefix,
            cluster: &r.cluster,
            rewrite: r.rewrite.as_deref(),
        })
        .collect();

    // The app cluster (when present) is emitted first, before the module clusters —
    // matching the route table, where its `/` catch-all comes last.
    let mut clusters = Vec::with_capacity(gateway.clusters.len() + 1);
    if let Some(app) = &opts.app {
        clusters.push(ClusterCtx {
            name: "app",
            host: &app.service,
            port: app.port,
        });
    }
    clusters.extend(gateway.clusters.iter().map(|c| ClusterCtx {
        name: &c.name,
        host: &c.host,
        port: c.port,
    }));

    let ctx = EnvoyCtx {
        internal_port,
        routes,
        clusters,
        app: opts.app.is_some(),
    };

    let mut env = minijinja::Environment::new();
    env.add_template("envoy", ENVOY_TEMPLATE)
        .expect("the embedded Envoy template is valid");
    env.get_template("envoy")
        .expect("the Envoy template was just added")
        .render(&ctx)
        .expect("rendering the Envoy template with a valid context cannot fail")
}

/// Append each line followed by `\n`. Used by the hand-built (non-templated) artifact
/// renderers below — the Postgres init script, the `.env` overlay, and the top-level
/// compose file, whose shapes are small, line-oriented, and not worth a template.
fn push_lines(out: &mut String, lines: &[&str]) {
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
}

/// Render the Postgres `init-databases.sh` that creates each database the stack needs.
pub fn render_postgres_init(databases: &[String]) -> String {
    let mut out = String::new();
    push_lines(
        &mut out,
        &[
            "#!/bin/bash",
            "# Runs once on first Postgres startup (empty data dir). Creates each database",
            "# declared by a module under `provides.postgres_databases`.",
            "set -euo pipefail",
            "",
            "psql -v ON_ERROR_STOP=1 --username \"$POSTGRES_USER\" --dbname \"$POSTGRES_DB\" <<-SQL",
        ],
    );
    for db in databases {
        push_lines(
            &mut out,
            &[
                &format!("    CREATE DATABASE {db};"),
                &format!("    GRANT ALL PRIVILEGES ON DATABASE {db} TO $POSTGRES_USER;"),
            ],
        );
    }
    out.push_str("SQL\n");
    out
}

/// Render the `.env` overlay: `KEY=value` lines in deterministic (key) order.
pub fn render_env(env: &InjectedEnv) -> String {
    let mut out = String::new();
    push_lines(
        &mut out,
        &[
            "# Local environment overlay. Copy to `.env.local` and adjust as needed.",
            "# The variables below mirror the env vars Databricks Apps inject, so app code",
            "# reads the same names locally and on Databricks.",
        ],
    );
    for (k, v) in env.iter() {
        let _ = writeln!(out, "{k}={v}");
    }
    out
}

/// Render the top-level `compose.yaml`: the project name plus an `include:` per module
/// fragment, in dependency order.
pub fn render_compose(head: &HeadFile) -> String {
    let mut out = String::new();
    push_lines(
        &mut out,
        &[
            "# Top-level compose entrypoint. Every backing service is pulled in via `include:`.",
            "# The file list is owned by the planner, not edited by hand.",
            &format!("name: {}", head.name),
            "",
            "include:",
        ],
    );
    for inc in &head.includes {
        push_lines(
            &mut out,
            &[
                &format!(
                    "  - path: ./docker/compose/{}.yaml",
                    compose_path_name(inc.module.as_str())
                ),
                "    project_directory: .",
            ],
        );
    }
    out
}

/// The compose fragment filename a module's include points at (the module id with the
/// `local-stack-` prefix stripped, matching trestle's `docker/compose/<name>.yaml`).
fn compose_path_name(module_id: &str) -> &str {
    module_id.strip_prefix("local-stack-").unwrap_or(module_id)
}

/// The four stack-aggregated artifacts for a plan, rendered together.
pub struct Artifacts {
    /// The Envoy bootstrap config (`docker/envoy/envoy.yaml`).
    pub envoy: String,
    /// The Postgres init script (`docker/postgres/init-databases.sh`).
    pub postgres_init: String,
    /// The `.env` overlay.
    pub env: String,
    /// The top-level `compose.yaml`.
    pub compose: String,
}

/// Render all four stack-aggregated artifacts for `plan`. Per-module compose fragments
/// live on the plan's [`renders`](crate::EnvironmentPlan::renders).
pub fn render_all(plan: &EnvironmentPlan, opts: &EnvoyOpts) -> Artifacts {
    Artifacts {
        envoy: render_envoy(&plan.gateway, opts),
        postgres_init: render_postgres_init(&plan.postgres_databases),
        env: render_env(&plan.env),
        compose: render_compose(&plan.head),
    }
}
