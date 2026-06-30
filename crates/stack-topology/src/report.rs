//! A pure, at-a-glance summary of a planned environment's gateway layout: [`layout_report`].
//!
//! The [`Plan`] already holds every fact a reader needs — which gateway listener serves which
//! route, what upstream each route lands on, and where each service runs. This module renders
//! those facts as a terse Markdown report (two tables, no prose) so a human can see the deployed
//! shape of a compose environment at a glance. It is **pure** (a `&Plan -> String` function, no
//! I/O), so it runs in the browser too; [`Plan::materialize`](crate::Plan::materialize) also emits
//! it as a `LAYOUT.md` file at the project root.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::model::placement::Placement;
use crate::plan::Plan;

/// Render a compact Markdown summary of `plan`'s gateway layout: the gateway routes (which prefix
/// reaches which service, on which listener port) and the deployed services. Facts only — the
/// output is meant to read at a glance, not as prose.
///
/// The routes table mirrors Envoy match priority (most-specific prefix first); the services table
/// is in plan (dependency) order.
pub fn layout_report(plan: &Plan) -> String {
    let mut out = String::new();

    // Title: the env name and the stable, host-facing gateway port — the single most useful fact.
    let _ = writeln!(
        out,
        "# Environment `{}` — gateway http://localhost:{}",
        plan.head.name,
        plan.gateway_host_port(),
    );

    // Map a cluster name to a human "service @ host:port", so a route row says what it lands on.
    let clusters: BTreeMap<&str, String> = plan
        .gateway
        .clusters
        .iter()
        .map(|c| (c.name.as_str(), format!("{}:{}", c.host, c.port)))
        .collect();
    let shared_port = plan.gateway_host_port();

    // --- Gateway routes ---
    out.push_str("\n## Gateway routes\n\n");
    out.push_str("| Listener | Prefix | Service (cluster) | Upstream | Rewrite |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for listener in &plan.gateway.listeners {
        let kind = if listener.host_port == shared_port {
            format!("shared :{}", listener.host_port)
        } else {
            format!("dedicated :{}", listener.host_port)
        };
        for route in &listener.routes {
            let upstream = clusters
                .get(route.cluster.as_str())
                .map_or("—", String::as_str);
            let rewrite = route.rewrite.as_deref().unwrap_or("—");
            let _ = writeln!(
                out,
                "| {} | `{}` | `{}` | {} | {} |",
                kind, route.prefix, route.cluster, upstream, rewrite,
            );
        }
    }

    // --- Services ---
    out.push_str("\n## Services\n\n");
    out.push_str("| Module | Service | Role | Placement | Endpoints |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for (module, services) in &plan.services {
        for svc in services {
            let placement = match &svc.placement {
                Placement::InProcess => "in-process".to_string(),
                Placement::Host => "host".to_string(),
                Placement::Container { service } => format!("container `{service}`"),
            };
            let endpoints = svc
                .endpoints
                .iter()
                .map(|e| format!("{}:{}", e.id, e.internal_port))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "| `{}` | `{}` | `{}` | {} | {} |",
                module,
                svc.name,
                svc.role.as_str(),
                placement,
                if endpoints.is_empty() {
                    "—".into()
                } else {
                    endpoints
                },
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::baseline::baseline_catalog;
    use crate::plan::{PlanCtx, Selection};

    #[test]
    fn report_states_the_key_facts() {
        let plan = baseline_catalog()
            .plan(
                &Selection::modules(["envoy", "postgres", "seaweedfs", "unity-catalog", "mlflow"]),
                &PlanCtx::default(),
            )
            .unwrap();
        let report = layout_report(&plan);

        // Title carries the stable host port.
        assert!(report.contains("gateway http://localhost:9080"), "{report}");
        // The mlflow UI route and its cluster appear.
        assert!(report.contains("/mlflow"), "{report}");
        assert!(report.contains("mlflow"), "{report}");
        // Both section headers render.
        assert!(report.contains("## Gateway routes"));
        assert!(report.contains("## Services"));
        // A container placement is shown.
        assert!(report.contains("container `"), "{report}");
    }
}
