//! The serializable projection layer between the planner and JS.
//!
//! Several planner types are deliberately **not** `Serialize`:
//! [`ResolvedGraph`](olai_stack_topology::ResolvedGraph)'s nodes are `Arc<dyn Module>` trait
//! objects, and the gateway config types keep serialization at the artifact-renderer boundary.
//! This module defines plain `#[derive(Serialize)]` DTOs and projects the planner's output into
//! them, so the JS side never sees a trait object. It is pure and compiles on every target, so
//! the projection is unit-tested natively (no wasm runner) — only the thin `JsValue` wrappers in
//! [`bindings`](crate::bindings) are wasm-gated.

use std::collections::BTreeMap;

use olai_stack_topology::{
    Catalog, ClusterConfig, GatewayConfig, GatewayRoute, Knob, ListenerConfig, PlanCtx, PlanError,
    Role, Selection, ServiceSpec, baseline_catalog, baseline_selection,
};
use serde::Serialize;

pub use crate::topology::{
    CLIENTS_NODE_ID, ExposedDto, ExposedKind, GraphDto, GraphNodeDto, NodeKind, ProvisionedDto,
    SurfaceDto, SurfaceKind, TopologyEdgeDto, graph_dto,
};

/// The whole selectable catalog, projected for the picker UI: every module with the metadata a
/// user needs to choose it (name/summary/category), its relationships (requires/conflicts), the
/// capability it provides, and its tunable knobs.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogDto {
    /// Every module in the baseline catalog, in catalog order.
    pub modules: Vec<ModuleDto>,
    /// A sensible starting selection (the baseline lakehouse), so the UI can open pre-populated.
    pub default_selection: Selection,
}

/// One selectable module, projected from the [`Module`] trait (which is not `Serialize`).
#[derive(Debug, Clone, Serialize)]
pub struct ModuleDto {
    /// Stable module id (e.g. `"mlflow"`).
    pub id: String,
    /// Human-readable name for a card title; falls back to the id when unset.
    pub display_name: Option<String>,
    /// One-line summary for the card body.
    pub summary: Option<String>,
    /// Wizard category the module slots into (e.g. `"ml"`, `"storage"`), for grouping.
    pub category: Option<String>,
    /// The capability this module provides (e.g. `"experiment_tracking"`), for the
    /// capability-based picker.
    pub provider_of: Option<String>,
    /// Ids of modules this one requires (pulled in transitively when selected).
    pub requires: Vec<String>,
    /// Ids of modules this one cannot coexist with (selecting both is a plan error).
    pub conflicts_with: Vec<String>,
    /// The user-tunable config knobs this module exposes. `Knob` already derives `Serialize`, so
    /// it is reused verbatim.
    pub knobs: Vec<Knob>,
}

/// The result of planning a selection: the runtime topology for the diagram, the resolved
/// services, a projected gateway layout, and the non-sensitive materialized files.
#[derive(Debug, Clone, Serialize)]
pub struct PlanResultDto {
    /// The functional topology (see [`crate::topology`]) — the primary input to the React Flow
    /// "markitecture".
    pub graph: GraphDto,
    /// Each module's resolved services (`ServiceSpec` is `Serialize`), keyed by module id.
    pub services: BTreeMap<String, Vec<ServiceSpec>>,
    /// The gateway layout (listeners → routes, and upstream clusters), projected from the
    /// non-`Serialize` planner types.
    pub gateway: GatewayDto,
    /// Every non-sensitive file the plan materializes (compose fragments, configs, etc.).
    pub files: Vec<OutputFileDto>,
}

/// One materialized file safe to expose in the browser (sensitive paths are filtered out).
#[derive(Debug, Clone, Serialize)]
pub struct OutputFileDto {
    /// Path relative to the environment root (e.g. `compose.yaml`, `modules/postgres/compose.yaml`).
    pub path: String,
    /// The file's rendered contents.
    pub contents: String,
}

/// The gateway layout, projected from the non-`Serialize` [`GatewayConfig`].
#[derive(Debug, Clone, Serialize)]
pub struct GatewayDto {
    /// The listeners (the shared, path-multiplexed one plus any dedicated ones).
    pub listeners: Vec<ListenerDto>,
    /// The upstream clusters the routes forward to.
    pub clusters: Vec<ClusterDto>,
    /// The Envoy admin port (published 1:1 on the host).
    pub admin_port: u16,
    /// Forward-auth, present only when the gateway's `auth` knob is on.
    pub auth: Option<AuthDto>,
    /// The in-network TLS listener for emulated public hostnames, present only when a module
    /// declares impersonated hosts.
    pub tls: Option<TlsDto>,
}

/// The gateway's forward-auth wiring, projected from [`AuthConfig`](olai_stack_topology::AuthConfig).
#[derive(Debug, Clone, Serialize)]
pub struct AuthDto {
    /// The upstream cluster the `ext_authz` check calls.
    pub cluster: String,
    /// The client-facing prefix of the provider's login portal (not gated).
    pub portal_prefix: String,
}

/// The gateway's emulated-hostname TLS listener, projected from
/// [`TlsListenerConfig`](olai_stack_topology::TlsListenerConfig).
#[derive(Debug, Clone, Serialize)]
pub struct TlsDto {
    /// The listener's in-network port (never host-published).
    pub port: u16,
    /// Every emulated hostname, sorted.
    pub hosts: Vec<String>,
    /// The hostnames each upstream cluster answers.
    pub virtual_hosts: Vec<VirtualHostDto>,
}

/// The hostnames one upstream cluster answers on the TLS listener.
#[derive(Debug, Clone, Serialize)]
pub struct VirtualHostDto {
    /// The upstream cluster.
    pub cluster: String,
    /// The exact hostnames routed to it.
    pub hosts: Vec<String>,
}

/// A gateway listener and its routes.
#[derive(Debug, Clone, Serialize)]
pub struct ListenerDto {
    /// The host-published port this listener binds.
    pub host_port: u16,
    /// The routes on this listener, most-specific-first.
    pub routes: Vec<RouteDto>,
}

/// One route on a listener.
#[derive(Debug, Clone, Serialize)]
pub struct RouteDto {
    /// The client-facing prefix matched.
    pub prefix: String,
    /// The upstream cluster forwarded to.
    pub cluster: String,
    /// The upstream rewrite, if the path is changed before forwarding.
    pub rewrite: Option<String>,
}

/// An upstream cluster.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterDto {
    /// The cluster's logical name (the service name).
    pub name: String,
    /// The upstream host (compose DNS name).
    pub host: String,
    /// The upstream port.
    pub port: u16,
}

/// Project the baseline catalog into a [`CatalogDto`] for the picker.
///
/// Reads each module through the [`Module`] trait accessors (the trait is not `Serialize`) and
/// reuses `Knob`'s own `Serialize` impl for the knob list.
pub fn catalog_dto() -> CatalogDto {
    let catalog = baseline_catalog();
    let modules = catalog
        .modules()
        .iter()
        .map(|m| ModuleDto {
            id: m.id().as_str().to_string(),
            display_name: m.display_name().map(str::to_string),
            summary: m.summary().map(str::to_string),
            category: m.category().map(str::to_string),
            provider_of: m.provider_of().map(str::to_string),
            requires: m
                .requires()
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
            conflicts_with: m
                .conflicts_with()
                .iter()
                .map(|c| c.as_str().to_string())
                .collect(),
            knobs: m.knobs().to_vec(),
        })
        .collect();
    CatalogDto {
        modules,
        default_selection: baseline_selection(),
    }
}

/// Project a [`GatewayConfig`] into a [`GatewayDto`].
fn gateway_dto(gateway: &GatewayConfig) -> GatewayDto {
    GatewayDto {
        listeners: gateway
            .listeners
            .iter()
            .map(|l: &ListenerConfig| ListenerDto {
                host_port: l.host_port,
                routes: l
                    .routes
                    .iter()
                    .map(|r: &GatewayRoute| RouteDto {
                        prefix: r.prefix.clone(),
                        cluster: r.cluster.clone(),
                        rewrite: r.rewrite.clone(),
                    })
                    .collect(),
            })
            .collect(),
        clusters: gateway
            .clusters
            .iter()
            .map(|c: &ClusterConfig| ClusterDto {
                name: c.name.clone(),
                host: c.host.clone(),
                port: c.port,
            })
            .collect(),
        admin_port: gateway.admin_port,
        auth: gateway.auth.as_ref().map(|a| AuthDto {
            cluster: a.cluster.clone(),
            portal_prefix: a.portal_prefix.clone(),
        }),
        tls: gateway.tls.as_ref().map(|t| TlsDto {
            port: t.port,
            hosts: t.hosts.clone(),
            virtual_hosts: t
                .virtual_hosts
                .iter()
                .map(|v| VirtualHostDto {
                    cluster: v.cluster.clone(),
                    hosts: v.hosts.clone(),
                })
                .collect(),
        }),
    }
}

/// Plan a selection against the baseline catalog and project the whole result into a
/// [`PlanResultDto`]. Pure — no I/O — so it runs identically native and on wasm32.
///
/// Uses [`PlanCtx::default`] for the environment context (gateway service/ports, data root); a
/// future revision can accept a partial `PlanCtx` from the caller.
pub fn plan_result(selection: &Selection) -> Result<PlanResultDto, PlanError> {
    let catalog: Catalog = baseline_catalog();
    let ctx = PlanCtx::default();
    let plan = catalog.plan(selection, &ctx)?;
    let materialized = plan.materialize();
    // Omit secrets and LAYOUT.md — the topology diagram is the richer layout view.
    let files = materialized
        .files
        .into_iter()
        .filter(|f| !f.sensitive && f.path != "LAYOUT.md")
        .map(|f| OutputFileDto {
            path: f.path,
            contents: f.contents,
        })
        .collect();
    Ok(PlanResultDto {
        graph: graph_dto(&plan),
        services: plan
            .services
            .iter()
            .map(|(id, specs)| (id.as_str().to_string(), specs.clone()))
            .collect(),
        gateway: gateway_dto(&plan.gateway),
        files,
    })
}

/// Well-known role strings, re-exported as a convenience for the JS side's role→theme map. Kept
/// here so a single Rust source lists them (they mirror [`Role`]'s consts).
pub const KNOWN_ROLES: &[&str] = &[
    Role::OBJECT_STORE,
    Role::RELATIONAL_DB,
    Role::DATA_CATALOG,
    Role::GATEWAY,
    Role::SQL_ENGINE,
    Role::EXPERIMENT_TRACKING,
    Role::TRACING,
    Role::LINEAGE,
    Role::AUTH,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_dto_projects_all_baseline_modules_with_knobs() {
        let dto = catalog_dto();
        // The baseline ships the common lakehouse modules.
        let ids: Vec<&str> = dto.modules.iter().map(|m| m.id.as_str()).collect();
        for expected in ["envoy", "postgres", "rustfs", "mlflow", "unity-catalog"] {
            assert!(
                ids.contains(&expected),
                "catalog missing {expected}: {ids:?}"
            );
        }
        // The gateway (envoy) module exposes at least the `auth` knob — proves knobs project.
        let envoy = dto.modules.iter().find(|m| m.id == "envoy").unwrap();
        assert!(
            envoy.knobs.iter().any(|k| k.key == "auth"),
            "envoy should expose an `auth` knob: {:?}",
            envoy.knobs
        );
        // The default selection is non-empty (opens the UI pre-populated).
        assert!(!dto.default_selection.modules.is_empty());
    }

    #[test]
    fn capability_provider_is_projected() {
        let dto = catalog_dto();
        let mlflow = dto.modules.iter().find(|m| m.id == "mlflow").unwrap();
        assert_eq!(mlflow.provider_of.as_deref(), Some("experiment_tracking"));
    }

    #[test]
    fn plan_result_projects_graph_edges_and_files() {
        // Selecting mlflow pulls in its transitive requires (postgres, an object store, envoy).
        let selection = Selection::modules(["mlflow"]);
        let result = plan_result(&selection).expect("plan should succeed");

        let node_ids: Vec<&str> = result.graph.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(
            node_ids.contains(&"mlflow"),
            "graph missing mlflow: {node_ids:?}"
        );
        assert!(
            node_ids.contains(&"postgres"),
            "graph missing postgres: {node_ids:?}"
        );

        // mlflow uses its backing capabilities (the full topology is pinned in `topology`'s
        // tests).
        assert!(
            result.graph.edges.iter().any(|e| matches!(
                e,
                TopologyEdgeDto::Uses { from, to, .. } if from == "mlflow" && to == "postgres"
            )),
            "expected mlflow to use postgres: {:?}",
            result.graph.edges
        );

        // A node carries the role of its primary service (mlflow = experiment_tracking).
        let mlflow_node = result
            .graph
            .nodes
            .iter()
            .find(|n| n.id == "mlflow")
            .unwrap();
        assert_eq!(mlflow_node.role.as_deref(), Some("experiment_tracking"));

        let paths: Vec<&str> = result.files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.contains(&"compose.yaml"),
            "compose.yaml should be present: {paths:?}"
        );
        assert!(
            paths.contains(&"modules/mlflow/compose.yaml"),
            "module compose fragment should be present: {paths:?}"
        );
        assert!(
            !paths.contains(&"LAYOUT.md"),
            "LAYOUT.md should be omitted in favor of the topology diagram: {paths:?}"
        );
        assert!(
            !paths.contains(&".env"),
            "sensitive .env should be filtered: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("/secrets/")),
            "secret files should be filtered: {paths:?}"
        );
    }

    /// The selection the `node/stack-wasm` fixtures are captured from.
    fn fixture_selection() -> Selection {
        Selection::modules([
            "envoy",
            "postgres",
            "rustfs",
            "unity-catalog",
            "mlflow",
            "jaeger",
        ])
    }

    #[test]
    #[ignore = "manual: regenerate node/stack-wasm fixture files"]
    fn dump_fixture_files_json() {
        let selection = fixture_selection();
        let result = plan_result(&selection).expect("plan should succeed");
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&result.files).expect("serialize files")
        );
    }

    #[test]
    #[ignore = "manual: regenerate node/stack-wasm FIXTURE_CATALOG + FIXTURE_PLAN"]
    fn dump_fixture_plan_json() {
        // A borrowing struct, not `serde_json::json!`: `json!` sorts keys, while this keeps
        // the DTOs' declaration order (what the wasm build emits).
        #[derive(Serialize)]
        struct Dump<'a> {
            catalog: CatalogDto,
            graph: &'a GraphDto,
            services: &'a BTreeMap<String, Vec<ServiceSpec>>,
            gateway: &'a GatewayDto,
        }
        let result = plan_result(&fixture_selection()).expect("plan should succeed");
        let dump = Dump {
            catalog: catalog_dto(),
            graph: &result.graph,
            services: &result.services,
            gateway: &result.gateway,
        };
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&dump).expect("serialize plan")
        );
    }

    #[test]
    fn invalid_selection_surfaces_a_plan_error() {
        // An unknown module id cannot be resolved against the catalog, so planning fails — the
        // error path the JS wrapper turns into a rejected promise / inline error.
        let selection = Selection::modules(["not-a-real-module"]);
        assert!(
            plan_result(&selection).is_err(),
            "an unknown module should be a plan error"
        );
    }
}
