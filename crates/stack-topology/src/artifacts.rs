//! Pure renderers that turn an [`EnvironmentPlan`](crate::EnvironmentPlan) into the
//! concrete text artifacts a Lakehouse dev environment is made of: the Envoy gateway
//! bootstrap, the `.env` overlay, and the top-level compose file.
//!
//! These are the **stack-aggregated** outputs — the ones that, in trestle today, are
//! MiniJinja `{% for %}` loops over the aggregated `stack.*` lists. Here they are plain
//! functions over exactly what [`plan`](crate::plan) already computes ([`GatewayConfig`],
//! the aggregated env, and the [`HeadFile`]), so the crate is the single source of truth
//! and the consuming tool only writes the returned strings to disk. (Per-module config
//! files — including the Postgres init script — are now module
//! [`RenderFile`](crate::RenderFile)s on the plan's
//! [`renders`](crate::EnvironmentPlan::renders), not aggregated here.)
//!
//! Everything here is pure and string-only — no I/O. The output shape is fixed and
//! hand-written (not serialized via a YAML library) to stay reviewable and stable.
//! Gated behind the `render` feature so the pure addressing core need not pull it in.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde::Serialize;

use crate::plan_env::{EnvironmentPlan, GatewayConfig, HeadFile};
use crate::render::InjectedEnv;

/// The on-disk Envoy bootstrap template, embedded at compile time. Lives at
/// `templates/gateway/bootstrap.yaml.jinja` (a sibling of `src/`), alongside the envoy
/// module's compose fragment (`templates/gateway/compose.yaml.jinja`), so the gateway's two
/// template faces sit together and are reviewable as plain files rather than inline strings.
const ENVOY_TEMPLATE: &str = include_str!("../templates/gateway/bootstrap.yaml.jinja");

/// The flat, owned render context the Envoy template iterates. The ordering decisions
/// (longest-prefix-first routes, app cluster first) stay in Rust; the template only
/// renders what it is handed. The core plan types ([`GatewayConfig`] et al.) deliberately
/// do not derive `Serialize`, so this is the serialization boundary.
#[derive(Serialize)]
struct EnvoyCtx<'a> {
    /// Every listener to emit: the shared, path-multiplexed one first, then any dedicated
    /// listeners (one per fixed-path UI / gatewayed backend) on their own ports.
    listeners: Vec<ListenerCtx<'a>>,
    clusters: Vec<ClusterCtx<'a>>,
    /// The Envoy admin port (bound on both sides).
    admin_port: u16,
}

/// One Envoy listener: the port it binds and its route table. Only the shared listener
/// (the first one) carries the app catch-all, flagged by its own `app` field.
#[derive(Serialize)]
struct ListenerCtx<'a> {
    name: String,
    internal_port: u16,
    routes: Vec<RouteCtx<'a>>,
    /// Whether the app `/` catch-all should be appended to this listener's routes (the
    /// shared listener only).
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
/// Reproduces the Databricks-edge shape: the shared, path-multiplexed listener on the
/// gateway's internal port (each [`GatewayRoute`](crate::GatewayRoute) in order, with a
/// `regex_rewrite` block only when the route has a rewrite, and an optional `/` app catch-all
/// last), then one dedicated listener per fixed-path UI / gatewayed backend (serving `/` to
/// its own cluster on its own port), the upstream clusters, and the admin endpoint.
pub fn render_envoy(gateway: &GatewayConfig, opts: &EnvoyOpts) -> String {
    // Each planner listener becomes an Envoy listener. The shared listener is first and
    // carries the app catch-all; dedicated listeners (their own port, a single `/` route)
    // follow. An empty gateway still emits a valid, route-less shared listener.
    let mut listeners: Vec<ListenerCtx> = gateway
        .listeners
        .iter()
        .enumerate()
        .map(|(i, l)| ListenerCtx {
            name: if i == 0 {
                "gateway".to_string()
            } else {
                format!("dedicated_{}", l.host_port)
            },
            internal_port: l.internal_port,
            routes: l
                .routes
                .iter()
                .map(|r| RouteCtx {
                    prefix: &r.prefix,
                    cluster: &r.cluster,
                    rewrite: r.rewrite.as_deref(),
                })
                .collect(),
            // Only the shared listener (first) carries the app catch-all.
            app: i == 0 && opts.app.is_some(),
        })
        .collect();
    if listeners.is_empty() {
        // No listeners at all (empty plan) — emit a bare shared listener so the config is
        // still valid Envoy.
        listeners.push(ListenerCtx {
            name: "gateway".to_string(),
            internal_port: 10000,
            routes: Vec::new(),
            app: opts.app.is_some(),
        });
    }

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
        listeners,
        clusters,
        admin_port: gateway.admin_port,
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

/// Collect the compose variable names referenced as `${NAME}` (or `${NAME:-default}` /
/// `${NAME:?err}`) across `texts`. Only brace substitutions count — a brace-less shell `$VAR`
/// (e.g. `$POSTGRES_USER` inside an init heredoc) is a container-runtime read, not a compose
/// substitution the `.env` overlay must supply, so it is deliberately ignored.
fn referenced_env_vars<'a>(texts: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for text in texts {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                let mut j = i + 2;
                // A variable name: leading non-digit identifier char, then identifier chars.
                let start = j;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let name = &text[start..j];
                // Valid only if non-empty, not starting with a digit, and the braces close
                // (possibly after a `:-default` / `:?err` modifier).
                if !name.is_empty()
                    && !name.as_bytes()[0].is_ascii_digit()
                    && text[j..].find('}').is_some()
                {
                    refs.insert(name.to_string());
                }
                i = j;
            } else {
                i += 1;
            }
        }
    }
    refs
}

/// Render the `.env` overlay: `KEY=value` lines in deterministic (key) order, emitting only
/// keys some rendered artifact references as `${KEY}` (see [`referenced_env_vars`]). Keys the
/// templates consumed concretely at plan time (e.g. `BASE_PATH`, `DATA_ROOT`) are referenced
/// by nothing and so are naturally omitted.
fn render_env(env: &InjectedEnv, referenced: &BTreeSet<String>) -> String {
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
        if referenced.contains(k) {
            let _ = writeln!(out, "{k}={v}");
        }
    }
    out
}

/// Render the top-level `compose.yaml`: the project name, a top-level `configs:` block
/// declaring every module-announced config file (alias → host file), and an `include:` per
/// module fragment, in dependency order.
///
/// Each module lives in its own directory: its fragment is `./modules/<id>/compose.yaml` and
/// its config files sit beside it (`./modules/<id>/<file>`). `project_directory: .` keeps a
/// fragment's relative references resolving against this top-level file, so the `configs:`
/// `file:` paths below line up with what the consumer writes.
pub fn render_compose(head: &HeadFile) -> String {
    let mut out = String::new();
    push_lines(
        &mut out,
        &[
            "# Top-level compose entrypoint. Every backing service is pulled in via `include:`.",
            "# The file list is owned by the planner, not edited by hand.",
            &format!("name: {}", head.name),
        ],
    );
    if !head.configs.is_empty() {
        push_lines(&mut out, &["", "configs:"]);
        for c in &head.configs {
            push_lines(
                &mut out,
                &[
                    &format!("  {}:", c.alias),
                    &format!("    file: ./{}", c.path),
                ],
            );
        }
    }
    push_lines(&mut out, &["", "include:"]);
    for inc in &head.includes {
        push_lines(
            &mut out,
            &[
                // Each module's fragment is co-located in its own directory.
                &format!("  - path: ./modules/{}/compose.yaml", inc.module.as_str()),
                "    project_directory: .",
            ],
        );
    }
    out
}

/// The compose `configs:` alias the gateway's Envoy bootstrap is mounted under. The bootstrap
/// is produced by the dedicated [`render_envoy`] renderer (not a module [`RenderFile`]), so the
/// planner does not see it; [`render_all`] declares it as a synthetic config when the gateway
/// is present.
const ENVOY_CONFIG_ALIAS: &str = "envoy_config";

/// The per-module-rooted host path the Envoy bootstrap is written to.
const ENVOY_CONFIG_PATH: &str = "modules/envoy/envoy.yaml";

/// The stack-aggregated artifacts for a plan, rendered together.
pub struct Artifacts {
    /// The Envoy bootstrap config. Written to [`ENVOY_CONFIG_PATH`](self) (`modules/envoy/envoy.yaml`)
    /// and mounted by the gateway fragment via the `envoy_config` config alias.
    pub envoy: String,
    /// The `.env` overlay.
    pub env: String,
    /// The top-level `compose.yaml`.
    pub compose: String,
}

/// Render the stack-aggregated artifacts for `plan`. Per-module compose fragments and their
/// mounted config files live on the plan's [`renders`](crate::EnvironmentPlan::renders); this
/// adds the Envoy bootstrap (a dedicated renderer), the audited `.env`, and the top-level
/// `compose.yaml`.
pub fn render_all(plan: &EnvironmentPlan, opts: &EnvoyOpts) -> Artifacts {
    let envoy = render_envoy(&plan.gateway, opts);

    // The `.env` carries only keys some rendered artifact still references as `${KEY}`. Scan
    // every module fragment, every mounted file's contents, and the Envoy bootstrap.
    let referenced = referenced_env_vars(
        plan.renders
            .iter()
            .flat_map(|(_, out)| {
                std::iter::once(out.fragment.as_str())
                    .chain(out.files.iter().map(|f| f.contents.as_str()))
            })
            .chain(std::iter::once(envoy.as_str())),
    );

    // The Envoy bootstrap is a dedicated-renderer artifact, so add its `configs:` declaration
    // here rather than from a module's `RenderFile`. Detect the gateway the same way the
    // planner does — by the `gateway` role on a resolved service, not a module-id string — so a
    // differently-named gateway module is still covered.
    let gateway_present = plan
        .graph
        .nodes
        .iter()
        .any(|m| m.services.iter().any(|s| s.role == crate::Role::gateway()));
    let mut head = plan.head.clone();
    // Skip if a module already declared this alias (the planner would have surfaced any
    // cross-module collision); guarding here keeps the synthetic push from emitting a
    // duplicate top-level key.
    let already_declared = head.configs.iter().any(|c| c.alias == ENVOY_CONFIG_ALIAS);
    if gateway_present && !already_declared {
        head.configs.push(crate::plan_env::ConfigDecl {
            alias: ENVOY_CONFIG_ALIAS.into(),
            path: ENVOY_CONFIG_PATH.into(),
        });
        head.configs.sort_by(|a, b| a.alias.cmp(&b.alias));
    }

    Artifacts {
        envoy,
        env: render_env(&plan.env, &referenced),
        compose: render_compose(&head),
    }
}
