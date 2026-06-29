//! Pre-render wiring summary.
//!
//! Given the resolved component set (split into baseline / lakehouse / app-runtime
//! contributions) plus the aggregated [`StackContext`], the preview emits a
//! human-readable block of "this is what you're about to scaffold" — routes,
//! databases, S3 buckets, env vars, ports, and per-component `wire_help`.

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::error::Result;
use crate::template::aggregate::StackContext;
use crate::template::loader::LoadedTemplate;
use crate::template::resolve::ResolvedComponent;

/// Build a textual wiring preview suitable for printing to the terminal (or
/// embedding inside a `cliclack::note(...)`).
pub fn render_text(
    base: &LoadedTemplate,
    apps: &[LoadedTemplate],
    components: &[ResolvedComponent],
    stack: &StackContext,
) -> Result<String> {
    let mut out = String::new();

    // Group components by their role in the scaffold.
    let baseline_names: BTreeSet<String> = base.manifest.always.iter().cloned().collect();
    let app_hard_names: BTreeSet<String> = apps
        .iter()
        .flat_map(|a| a.manifest.lakehouse_requires.hard.iter().cloned())
        .collect();
    let app_soft_names: BTreeSet<String> = apps
        .iter()
        .flat_map(|a| a.manifest.lakehouse_requires.soft.iter().cloned())
        .collect();

    let (baseline, rest): (Vec<&ResolvedComponent>, Vec<&ResolvedComponent>) = components
        .iter()
        .partition(|c| baseline_names.contains(&c.loaded.manifest.name));

    writeln!(out, "Baseline (always-on):").ok();
    if baseline.is_empty() {
        writeln!(out, "  (none)").ok();
    } else {
        for rc in &baseline {
            writeln!(out, "  - {}", rc.loaded.manifest.name).ok();
        }
    }

    writeln!(
        out,
        "\nLakehouse components ({}):",
        rest.iter()
            .filter(|c| !app_hard_names.contains(&c.loaded.manifest.name)
                && !app_soft_names.contains(&c.loaded.manifest.name))
            .count()
    )
    .ok();
    for rc in &rest {
        if app_hard_names.contains(&rc.loaded.manifest.name)
            || app_soft_names.contains(&rc.loaded.manifest.name)
        {
            continue;
        }
        let label = component_label(rc);
        writeln!(out, "  - {label}").ok();
    }

    if !apps.is_empty() {
        writeln!(out, "\nApps ({}):", apps.len()).ok();
        for app in apps {
            let label = app
                .manifest
                .display_name
                .as_deref()
                .unwrap_or(&app.manifest.name);
            writeln!(out, "  - {} ({label})", app.manifest.name).ok();
            for name in &app.manifest.lakehouse_requires.hard {
                writeln!(out, "    └─ pulls in: {name} (via lakehouse_requires.hard)").ok();
            }
            for name in &app.manifest.lakehouse_requires.soft {
                writeln!(
                    out,
                    "    └─ pre-selected: {name} (via lakehouse_requires.soft)"
                )
                .ok();
            }
        }
    }

    if !stack.envoy_routes.is_empty() {
        writeln!(out, "\nEnvoy routes:").ok();
        for r in &stack.envoy_routes {
            if r.rewrite.is_empty() {
                writeln!(out, "  {:<28} → {}", r.prefix, r.cluster).ok();
            } else {
                writeln!(
                    out,
                    "  {:<28} → {} (rewrite {})",
                    r.prefix, r.cluster, r.rewrite
                )
                .ok();
            }
        }
    }

    if !stack.postgres_databases.is_empty() {
        writeln!(
            out,
            "Postgres dbs:    {}",
            stack.postgres_databases.join(", ")
        )
        .ok();
    }
    if !stack.s3_buckets.is_empty() {
        writeln!(out, "S3 buckets:      {}", stack.s3_buckets.join(", ")).ok();
    }
    if !stack.ports.is_empty() {
        let ports: Vec<String> = stack
            .ports
            .iter()
            .map(|p| format!("{}={}", p.name, p.default))
            .collect();
        writeln!(out, "Ports:           {}", ports.join(", ")).ok();
    }
    if !stack.env_vars.is_empty() {
        let mut names: Vec<&str> = stack.env_vars.keys().map(String::as_str).collect();
        names.sort();
        writeln!(out, "Env vars set:    {}", names.join(", ")).ok();
    }

    // Per-component wire_help (group all hints below their component).
    let hinted: Vec<&ResolvedComponent> = components
        .iter()
        .filter(|c| !c.loaded.manifest.wire_help.is_empty())
        .collect();
    if !hinted.is_empty() {
        writeln!(out, "\nWiring:").ok();
        for rc in hinted {
            let m = &rc.loaded.manifest;
            writeln!(out, "  {}:", component_label(rc)).ok();
            for h in &m.wire_help {
                let prefix = match (h.env.as_deref(), h.url.as_deref()) {
                    (Some(e), _) => format!("env {e}"),
                    (_, Some(u)) => format!("url {u}"),
                    _ => "    ".to_string(),
                };
                if let Some(note) = &h.note {
                    writeln!(out, "    {prefix} — {note}").ok();
                } else {
                    writeln!(out, "    {prefix}").ok();
                }
            }
        }
    }

    Ok(out)
}

fn component_label(rc: &ResolvedComponent) -> String {
    let m = &rc.loaded.manifest;
    let display = m.display_name.as_deref().unwrap_or(&m.name);
    if display == m.name {
        m.name.clone()
    } else {
        format!("{} ({display})", m.name)
    }
}
