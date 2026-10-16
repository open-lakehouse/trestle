//! The serializable projection of a settled [`Plan`](crate::Plan) and of a [`Catalog`].
//!
//! Several planner types are deliberately **not** `Serialize`:
//! [`ResolvedGraph`](crate::ResolvedGraph)'s nodes are `Arc<dyn Module>` trait objects, and the
//! gateway config types keep serialization at the artifact-renderer boundary. This module
//! defines plain `#[derive(Serialize)]` DTOs and projects the planner's output into them, so a
//! consumer (the browser planner, the CLI's JSON output) never sees a trait object. It is pure
//! and compiles on every target.

pub mod export;
pub mod topology;

use std::collections::BTreeMap;

use crate::{
    Catalog, ClusterConfig, EnvManifest, GatewayConfig, GatewayRoute, HelperService, HostedKind,
    HostedResource, Knob, ListenerConfig, Plan, PlanCtx, PlanError, RenderError, Role, Selection,
    ServiceSpec, TargetId, baseline_catalog, baseline_selection, builtin_targets,
};
use serde::Serialize;

pub use self::topology::{
    CLIENTS_NODE_ID, ExposedDto, ExposedKind, GraphDto, GraphNodeDto, NodeKind, ProvisionedDto,
    ResourceNounDto, SurfaceDto, SurfaceKind, TopologyEdgeDto, graph_dto, resource_nouns,
};

/// The whole selectable catalog, projected for the picker UI: every module with the metadata a
/// user needs to choose it (name/summary/category), its relationships (requires/conflicts), the
/// capability it provides, and its tunable knobs.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogDto {
    /// Every module in the catalog, in catalog order.
    pub modules: Vec<ModuleDto>,
    /// A sensible starting selection (the baseline lakehouse), so the UI can open pre-populated.
    pub default_selection: Selection,
    /// The deploy targets an environment can run on, the default (Docker Compose) first.
    pub targets: Vec<TargetDto>,
}

/// One deploy target an environment can run on.
#[derive(Debug, Clone, Serialize)]
pub struct TargetDto {
    /// Stable target id (e.g. `"compose"`), what [`plan_result`] takes.
    pub id: String,
    /// Human-readable name (e.g. `"Docker Compose"`).
    pub display_name: String,
}

/// One selectable module, projected from the [`Module`] trait (which is not `Serialize`).
#[derive(Debug, Clone, Serialize)]
pub struct ModuleDto {
    /// Stable module id (e.g. `"mlflow"`).
    pub id: String,
    /// The implementation the module packages, as an architecture-model element id (see
    /// [`Module::model`](crate::Module::model)).
    pub model: Option<String>,
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
    /// The resource kinds this module provisions on request (e.g. `"relational_db"` for a
    /// database server), so the UI can offer to add an extra database or bucket to it.
    pub resource_kinds: Vec<String>,
    /// What this module calls each kind of resource it provisions (databases, buckets,
    /// containers), for labels.
    pub resource_nouns: Vec<ResourceNounDto>,
    /// The user-tunable config knobs this module exposes. `Knob` already derives `Serialize`, so
    /// it is reused verbatim.
    pub knobs: Vec<Knob>,
    /// The helper containers (init jobs, sidecars) the module may run beside its services.
    pub helpers: Vec<HelperService>,
    /// Set for a hosted provider (a managed database or cloud object store the stack uses but
    /// does not run): what it stands for and the configuration it uses until the selection's
    /// `hosted` entry overrides it.
    pub hosted: Option<HostedModuleDto>,
}

/// A hosted provider module's kind and default configuration (see [`ModuleDto::hosted`]).
#[derive(Debug, Clone, Serialize)]
pub struct HostedModuleDto {
    /// What it stands for (`"postgres"`, `"s3"`, `"azure_blob"`).
    pub kind: HostedKind,
    /// The configuration used when the selection gives none.
    pub default: HostedResource,
}

/// The result of planning a selection: the runtime topology for the diagram, the resolved
/// services, a projected gateway layout, and the non-sensitive materialized files.
#[derive(Debug, Clone, Serialize)]
pub struct PlanResultDto {
    /// The functional topology (see [`topology`]) — the primary input to the React Flow
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
    /// The authorization policy check, present only when a policy engine is selected.
    pub policy: Option<PolicyDto>,
}

/// The gateway's forward-auth wiring, projected from [`AuthConfig`](crate::AuthConfig).
#[derive(Debug, Clone, Serialize)]
pub struct AuthDto {
    /// The upstream cluster the `ext_authz` check calls.
    pub cluster: String,
    /// The client-facing prefix of the provider's login portal (not gated).
    pub portal_prefix: String,
}

/// The gateway's authorization-policy wiring, projected from
/// [`PolicyConfig`](crate::PolicyConfig).
#[derive(Debug, Clone, Serialize)]
pub struct PolicyDto {
    /// The upstream cluster the gRPC `ext_authz` check calls.
    pub cluster: String,
}

/// The gateway's emulated-hostname TLS listener, projected from
/// [`TlsListenerConfig`](crate::TlsListenerConfig).
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
    catalog_dto_of(&baseline_catalog())
}

/// Project `catalog` (the baseline, or the baseline merged with custom manifests) for a picker
/// or a JSON listing.
pub fn catalog_dto_of(catalog: &Catalog) -> CatalogDto {
    let modules = catalog
        .modules()
        .iter()
        .map(|m| ModuleDto {
            id: m.id().as_str().to_string(),
            model: m.model().map(str::to_string),
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
            resource_kinds: m.provides().resource_kinds.keys().cloned().collect(),
            resource_nouns: resource_nouns(m.as_ref()),
            knobs: m.knobs().to_vec(),
            helpers: m.provides().helpers.clone(),
            hosted: m.hosted().map(|kind| HostedModuleDto {
                kind,
                default: kind.default_resource(),
            }),
        })
        .collect();
    CatalogDto {
        modules,
        default_selection: baseline_selection(),
        targets: builtin_targets()
            .iter()
            .map(|t| TargetDto {
                id: t.id().0,
                display_name: t.display_name().to_string(),
            })
            .collect(),
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
        policy: gateway.policy.as_ref().map(|p| PolicyDto {
            cluster: p.cluster.clone(),
        }),
    }
}

/// Why [`plan_result`] failed: the selection didn't plan, or the plan didn't render.
#[derive(Debug, thiserror::Error)]
pub enum PlanResultError {
    /// Planning the selection failed.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// Rendering the plan's files failed.
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// Plan a selection against the baseline catalog and project the whole result into a
/// [`PlanResultDto`]. Pure — no I/O — so it runs identically native and on wasm32.
///
/// Plans for the deploy target `target` (Docker Compose when `None`) and otherwise uses
/// [`PlanCtx::default`] for the environment context (gateway service/ports, data root); the
/// planner itself prefers the providers the selection names. A future revision can accept a
/// partial `PlanCtx` from the caller.
pub fn plan_result(
    selection: &Selection,
    target: Option<&str>,
) -> Result<PlanResultDto, PlanResultError> {
    let catalog: Catalog = baseline_catalog();
    let ctx = PlanCtx {
        target: target.map(TargetId::from).unwrap_or_default(),
        ..PlanCtx::default()
    };
    let plan = catalog.plan(selection, &ctx)?;
    project_result(&plan)
}

/// Plan `manifest` with its whole [`PlanCtx`] (gateway ports, provider preferences, env name,
/// …) against `catalog`, and project the result as [`plan_result`] does. Where
/// [`plan_result`] defaults everything but the target, this honours a saved environment.
pub fn plan_result_for_manifest(
    manifest: &EnvManifest,
    catalog: &Catalog,
) -> Result<PlanResultDto, PlanResultError> {
    project_result(&manifest.plan(catalog)?)
}

fn project_result(plan: &Plan) -> Result<PlanResultDto, PlanResultError> {
    let materialized = plan.materialize()?;
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
        graph: graph_dto(plan),
        services: services_by_module(plan),
        gateway: gateway_dto(&plan.gateway),
        files,
    })
}

/// Each module's resolved services, keyed by module id.
fn services_by_module(plan: &Plan) -> BTreeMap<String, Vec<ServiceSpec>> {
    plan.services
        .iter()
        .map(|(id, specs)| (id.as_str().to_string(), specs.clone()))
        .collect()
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
    Role::POLICY,
    Role::AGENT_PLATFORM,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_dto_projects_all_baseline_modules_with_knobs() {
        let dto = catalog_dto();
        // The baseline ships the common lakehouse modules.
        let ids: Vec<&str> = dto.modules.iter().map(|m| m.id.as_str()).collect();
        for expected in [
            "envoy",
            "postgres",
            "rustfs",
            "mlflow",
            "unity-catalog",
            "omnigent",
        ] {
            assert!(
                ids.contains(&expected),
                "catalog missing {expected}: {ids:?}"
            );
        }
        // The gateway (envoy) module exposes its image knob, flagged advanced — proves knobs
        // project with their tier.
        let envoy = dto.modules.iter().find(|m| m.id == "envoy").unwrap();
        assert!(
            envoy.knobs.iter().any(|k| k.key == "image" && k.advanced),
            "envoy should expose an advanced `image` knob: {:?}",
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
        let result = plan_result(&selection, None).expect("plan should succeed");

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
        Selection {
            gateway: crate::GatewaySettings::POLICY,
            ..Selection::modules(["postgres", "rustfs", "unity-catalog", "mlflow", "jaeger"])
        }
    }

    #[test]
    #[ignore = "manual: regenerate node/stack-wasm fixture files"]
    fn dump_fixture_files_json() {
        let selection = fixture_selection();
        let result = plan_result(&selection, None).expect("plan should succeed");
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
        let result = plan_result(&fixture_selection(), None).expect("plan should succeed");
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
    fn providers_name_their_resources_by_connection() {
        let catalog = catalog_dto();
        let nouns = |id: &str| -> Vec<(String, String)> {
            catalog
                .modules
                .iter()
                .find(|m| m.id == id)
                .expect("module in catalog")
                .resource_nouns
                .iter()
                .map(|n| (n.kind.clone(), n.plural.clone()))
                .collect()
        };
        assert_eq!(
            nouns("azurite"),
            [("object_store".into(), "containers".into())]
        );
        assert_eq!(nouns("rustfs"), [("object_store".into(), "buckets".into())]);
        assert_eq!(
            nouns("postgres"),
            [("relational_db".into(), "databases".into())]
        );
        assert!(nouns("mlflow").is_empty());
    }

    #[test]
    fn a_selected_provider_replaces_the_default() {
        let selection = Selection::modules(["envoy", "postgres", "azurite", "unity-catalog"]);
        let result = plan_result(&selection, None).expect("plan should succeed");
        let ids: Vec<&str> = result.graph.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"azurite"), "{ids:?}");
        assert!(!ids.contains(&"rustfs"), "{ids:?}");
    }

    #[test]
    fn catalog_lists_compose_as_the_default_target() {
        let targets = catalog_dto().targets;
        assert_eq!(targets[0].id, "compose");
        assert_eq!(targets[0].display_name, "Docker Compose");
        // Planning on it explicitly is the same as the default.
        let selection = Selection::modules(["mlflow"]);
        let explicit = plan_result(&selection, Some("compose")).unwrap();
        let default = plan_result(&selection, None).unwrap();
        assert_eq!(explicit.files.len(), default.files.len());
        assert!(plan_result(&selection, Some("helm")).is_err());
    }

    #[test]
    fn hosted_providers_are_external_nodes_grouped_by_cloud() {
        use crate::HostedConnection;
        let mut selection = Selection::modules(["envoy", "hosted-postgres", "aws-s3", "mlflow"]);
        selection.hosted.insert(
            "hosted-postgres".into(),
            HostedResource {
                cloud: Some("neon".into()),
                connection: HostedConnection::Postgres {
                    host: "db.example.com".into(),
                    port: 5432,
                    database: "{name}".into(),
                    username: "app".into(),
                    password: "${PG_PASSWORD}".into(),
                },
            },
        );
        let result = plan_result(&selection, None).expect("plan should succeed");
        let node = |id: &str| result.graph.nodes.iter().find(|n| n.id == id).unwrap();
        let pg = node("hosted-postgres");
        assert_eq!(pg.kind, NodeKind::External);
        assert_eq!(pg.cloud.as_deref(), Some("neon"));
        // Unconfigured, a hosted provider sits in its kind's default cloud.
        assert_eq!(node("aws-s3").cloud.as_deref(), Some("aws"));
        assert_eq!(
            pg.placement.as_deref(),
            Some("external:tcp://db.example.com:5432")
        );
        assert_eq!(node("mlflow").cloud, None);
        // Its consumers still use it, and nothing is rendered for it.
        assert!(
            result
                .graph
                .edges
                .iter()
                .any(|e| e.endpoints() == ("mlflow", "hosted-postgres"))
        );
        assert!(
            !result
                .files
                .iter()
                .any(|f| f.path.starts_with("modules/hosted-postgres/"))
        );
    }

    #[test]
    fn a_manifest_plans_with_its_whole_context() {
        let selection = Selection::modules(["unity-catalog"]);
        let ctx = PlanCtx {
            gateway_host_port: 8080,
            ..PlanCtx::default()
        };
        let manifest = EnvManifest::new(selection.clone(), ctx);
        let host_ports = |r: &PlanResultDto| -> Vec<u16> {
            r.gateway.listeners.iter().map(|l| l.host_port).collect()
        };
        let saved = plan_result_for_manifest(&manifest, &baseline_catalog()).unwrap();
        let defaulted = plan_result(&selection, None).unwrap();
        assert!(
            host_ports(&saved).contains(&8080),
            "{:?}",
            host_ports(&saved)
        );
        assert!(!host_ports(&defaulted).contains(&8080));
    }

    #[test]
    fn invalid_selection_surfaces_a_plan_error() {
        // An unknown module id cannot be resolved against the catalog, so planning fails — the
        // error path the JS wrapper turns into a rejected promise / inline error.
        let selection = Selection::modules(["not-a-real-module"]);
        assert!(
            plan_result(&selection, None).is_err(),
            "an unknown module should be a plan error"
        );
    }
}
