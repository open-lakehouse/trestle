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

use crate::plan_env::{EnvironmentPlan, GatewayConfig, HeadFile};
use crate::render::InjectedEnv;

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

    let mut out = String::new();
    out.push_str(ENVOY_HEADER);
    push_lines(
        &mut out,
        &[
            "static_resources:",
            "  listeners:",
            "    - name: gateway",
            "      address:",
            &format!("        socket_address: {{ address: 0.0.0.0, port_value: {internal_port} }}"),
            "      filter_chains:",
            "        - filters:",
            "            - name: envoy.filters.network.http_connection_manager",
            "              typed_config:",
            "                \"@type\": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager",
            "                codec_type: AUTO",
            "                stat_prefix: ingress_http",
            "                use_remote_address: true",
            "                upgrade_configs:",
            "                  - upgrade_type: websocket",
            "                route_config:",
            "                  name: local_route",
            "                  virtual_hosts:",
            "                    - name: all",
            "                      domains: [\"*\"]",
            "                      routes:",
        ],
    );

    if let Some(listener) = shared {
        for r in &listener.routes {
            push_lines(
                &mut out,
                &[
                    &format!(
                        "                        - match: {{ prefix: \"{}\" }}",
                        r.prefix
                    ),
                    "                          route:",
                    &format!("                            cluster: {}", r.cluster),
                ],
            );
            if let Some(rewrite) = &r.rewrite {
                push_lines(
                    &mut out,
                    &[
                        "                            regex_rewrite:",
                        "                              pattern:",
                        "                                google_re2: {}",
                        &format!(
                            "                                regex: \"^{}(.*)$\"",
                            r.prefix
                        ),
                        &format!("                              substitution: \"{rewrite}\\\\1\""),
                    ],
                );
            }
        }
    }

    if opts.app.is_some() {
        push_lines(
            &mut out,
            &[
                "                        # Default route → the user's app (catch-all, must be last).",
                "                        - match: { prefix: \"/\" }",
                "                          route:",
                "                            cluster: app",
            ],
        );
    }

    push_lines(
        &mut out,
        &[
            "                http_filters:",
            "                  - name: envoy.filters.http.router",
            "                    typed_config:",
            "                      \"@type\": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router",
            "  clusters:",
        ],
    );

    if let Some(app) = &opts.app {
        push_cluster(&mut out, "app", &app.service, app.port);
    }
    for c in &gateway.clusters {
        push_cluster(&mut out, &c.name, &c.host, c.port);
    }

    push_lines(
        &mut out,
        &[
            "",
            "admin:",
            "  address:",
            "    socket_address: { address: 0.0.0.0, port_value: 9901 }",
        ],
    );
    out
}

/// Append each line followed by `\n`.
fn push_lines(out: &mut String, lines: &[&str]) {
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
}

/// Append one Envoy `STRICT_DNS` cluster block.
fn push_cluster(out: &mut String, name: &str, host: &str, port: u16) {
    push_lines(
        out,
        &[
            &format!("    - name: {name}"),
            "      type: STRICT_DNS",
            "      connect_timeout: 2s",
            "      load_assignment:",
            &format!("        cluster_name: {name}"),
            "        endpoints:",
            "          - lb_endpoints:",
            "              - endpoint:",
            "                  address:",
            "                    socket_address:",
            &format!("                      address: {host}"),
            &format!("                      port_value: {port}"),
        ],
    );
}

/// The leading comment block on the rendered Envoy config.
const ENVOY_HEADER: &str = "# Envoy front-edge config, rendered from the planner's gateway config.\n\
# Route ordering matters: more-specific prefixes come before less-specific ones,\n\
# and the default app route (if any) is emitted last.\n\n";

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
