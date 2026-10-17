//! `plan.json`: the settled plan as a versioned, credential-free JSON document, for tools that
//! join an environment to something else (an architecture model, docs, a registry) without
//! running the planner.
//!
//! It is the [`PlanResultDto`](super::PlanResultDto) without file bodies, plus what a joining
//! tool needs that the browser doesn't: every container with its component, the client
//! environments per vantage, the file manifest, and provenance. The document records *planned
//! configuration*; whether the stack is healthy, or a test passed against it, is separate
//! evidence.
//!
//! Identical inputs give identical bytes: every map is ordered, nothing records a time or an
//! absolute path, and the version strings come from the caller ([`ExportMeta`]), so a test can
//! pin them. The schema is `schema/plan.v1.schema.json` in this crate.
//!
//! [`CatalogExport`] is the module catalog in the same envelope, for a tool that joins modules
//! rather than one environment (schema `schema/catalog.v1.schema.json`).

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::topology::{GraphDto, graph_dto};
use super::{
    CatalogDto, DeployStepDto, GatewayDto, catalog_dto_of, deploy_steps_dto, gateway_dto,
    services_by_module,
};
use crate::render::output::MaterializedOutput;
use crate::{Catalog, EnvManifest, HelperKind, InjectedEnv, Placement, Plan, PlanCtx, ServiceSpec};

/// The `format` every `plan.json` carries.
pub const PLAN_EXPORT_FORMAT: &str = "trestle-plan";

/// The `plan.json` schema version this crate writes. Bump it for any change a reader could
/// misread; adding an optional field is not one.
pub const PLAN_EXPORT_VERSION: u32 = 1;

/// The `format` every exported catalog carries.
pub const CATALOG_EXPORT_FORMAT: &str = "trestle-catalog";

/// The catalog schema version this crate writes, versioned like [`PLAN_EXPORT_VERSION`].
pub const CATALOG_EXPORT_VERSION: u32 = 1;

/// The exported plan (see the [module docs](self)).
#[derive(Debug, Clone, Serialize)]
pub struct PlanExport {
    /// Always [`PLAN_EXPORT_FORMAT`].
    pub format: &'static str,
    /// Always [`PLAN_EXPORT_VERSION`].
    pub version: u32,
    /// What the document describes: always [`PlanState::Planned`].
    pub state: PlanState,
    /// The tool that wrote the document.
    pub generator: Generator,
    /// The module catalog the plan was made from.
    pub catalog: CatalogIdentity,
    /// The recipe (the environment manifest) the plan was made from.
    pub recipe: RecipeIdentity,
    /// The plan context, as the manifest settled it. Secrets are never serialized.
    pub context: PlanCtx,
    /// The functional topology: components (with their model refs) and keyed edges.
    pub graph: GraphDto,
    /// Every container the deploy target runs, with the component it belongs to: each module's
    /// container-placed services, then its declared helpers. Hosted, host, and in-process
    /// services are graph nodes but not containers.
    pub containers: Vec<ContainerDto>,
    /// Each module's resolved services, keyed by module id.
    pub services: BTreeMap<String, Vec<ServiceSpec>>,
    /// The gateway layout: listeners, routes, and upstream clusters.
    pub gateway: GatewayDto,
    /// The variables a client loads to reach the stack, per vantage.
    pub client: ClientEnvDto,
    /// Every file the plan materializes, without contents.
    pub files: Vec<FileEntryDto>,
    /// The deployment steps, in order; their scripts are among [`files`](Self::files).
    pub deploy: Vec<DeployStepDto>,
}

/// What a [`PlanExport`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    /// The configuration the planner settled, before anything ran.
    Planned,
}

/// The tool that wrote a [`PlanExport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Generator {
    /// The tool's name (e.g. `"trestle"`).
    pub name: String,
    /// Its version.
    pub version: String,
    /// The source revision it was built from, when the build recorded one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Where a [`PlanExport`]'s modules came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogIdentity {
    /// The built-in catalog's label (e.g. `"olai-stack-topology@0.0.5"`).
    pub baseline: String,
    /// The module manifests merged over it, as the recipe lists them.
    pub manifests: Vec<String>,
    /// `sha256:<hex>` over the catalog's JSON projection ([`CatalogDto`]), so two exports can
    /// tell whether they planned from the same modules.
    pub digest: String,
}

/// The recipe a [`PlanExport`] was planned from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeIdentity {
    /// `sha256:<hex>` over the manifest's canonical JSON (parsed and re-serialized, so
    /// comments and formatting don't count; secrets are never part of it).
    pub digest: String,
}

/// The caller-supplied provenance of a [`PlanExport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportMeta {
    /// The tool writing the export.
    pub generator: Generator,
    /// The built-in catalog's label; [`ExportMeta::new`] uses this crate's name and version.
    pub baseline: String,
}

impl ExportMeta {
    /// Provenance for `generator`, labelling the baseline with this crate's version.
    pub fn new(generator: Generator) -> Self {
        ExportMeta {
            generator,
            baseline: concat!(env!("CARGO_PKG_NAME"), "@", env!("CARGO_PKG_VERSION")).into(),
        }
    }
}

/// One container in a [`PlanExport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerDto {
    /// The compose service name.
    pub name: String,
    /// The module (graph component) it belongs to.
    pub component: String,
    /// What it is to that component.
    pub kind: ContainerKind,
}

/// What a [`ContainerDto`] is to its component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerKind {
    /// One of the module's topology services.
    Service,
    /// A declared helper that runs once and exits. A helper may not run in every stack (see
    /// [`Provides::helpers`](crate::Provides::helpers)).
    Init,
    /// A declared helper that runs beside the module's services.
    Sidecar,
}

/// The client environments of a [`PlanExport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientEnvDto {
    /// For a client on the host (through published ports).
    pub host: InjectedEnv,
    /// For a client on the stack's network.
    pub container: InjectedEnv,
}

/// One materialized file, without its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntryDto {
    /// Path relative to the environment root.
    pub path: String,
    /// Whether it holds credentials (written with restricted permissions, and git-ignored).
    pub sensitive: bool,
}

/// The module catalog with the provenance a [`PlanExport`] records, so a joining tool can tell
/// which modules a plan was made from.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogExport {
    /// Always [`CATALOG_EXPORT_FORMAT`].
    pub format: &'static str,
    /// Always [`CATALOG_EXPORT_VERSION`].
    pub version: u32,
    /// The tool that wrote the document.
    pub generator: Generator,
    /// The catalog's identity, as a plan made from it records it.
    pub catalog: CatalogIdentity,
    /// The modules, the default selection, and the deploy targets.
    #[serde(flatten)]
    pub projection: CatalogDto,
}

/// Export `catalog`, the baseline with the module `manifests` merged over it.
pub fn catalog_export(
    catalog: &Catalog,
    manifests: &[String],
    meta: &ExportMeta,
) -> Result<CatalogExport, serde_json::Error> {
    Ok(CatalogExport {
        format: CATALOG_EXPORT_FORMAT,
        version: CATALOG_EXPORT_VERSION,
        generator: meta.generator.clone(),
        catalog: catalog_identity(catalog, manifests, meta)?,
        projection: catalog_dto_of(catalog),
    })
}

/// Serialize `export` like [`plan_export_json`]: pretty-printed, with a trailing newline.
pub fn catalog_export_json(export: &CatalogExport) -> Result<String, serde_json::Error> {
    let mut text = serde_json::to_string_pretty(export)?;
    text.push('\n');
    Ok(text)
}

/// Export `plan`, planned from `manifest` against `catalog`, with `materialized` its rendered
/// output. Pass the same plan the caller writes to disk, so the export describes exactly those
/// files.
pub fn plan_export(
    plan: &Plan,
    materialized: &MaterializedOutput,
    manifest: &EnvManifest,
    catalog: &Catalog,
    meta: &ExportMeta,
) -> Result<PlanExport, serde_json::Error> {
    Ok(PlanExport {
        format: PLAN_EXPORT_FORMAT,
        version: PLAN_EXPORT_VERSION,
        state: PlanState::Planned,
        generator: meta.generator.clone(),
        catalog: catalog_identity(catalog, &manifest.catalog.manifests, meta)?,
        recipe: RecipeIdentity {
            digest: digest(manifest)?,
        },
        context: manifest.context.clone(),
        graph: graph_dto(plan),
        containers: containers(plan),
        services: services_by_module(plan),
        gateway: gateway_dto(&plan.gateway),
        client: ClientEnvDto {
            host: plan.client_env_host.clone(),
            container: plan.client_env_container.clone(),
        },
        files: materialized
            .files
            .iter()
            .map(|f| FileEntryDto {
                path: f.path.clone(),
                sensitive: f.sensitive,
            })
            .collect(),
        deploy: deploy_steps_dto(&materialized.deploy),
    })
}

/// The identity of `catalog`, assembled from the baseline and the module manifests
/// `manifests` lists: the label from `meta`, and a digest of its [`CatalogDto`] projection.
pub fn catalog_identity(
    catalog: &Catalog,
    manifests: &[String],
    meta: &ExportMeta,
) -> Result<CatalogIdentity, serde_json::Error> {
    let projection: CatalogDto = catalog_dto_of(catalog);
    Ok(CatalogIdentity {
        baseline: meta.baseline.clone(),
        manifests: manifests.to_vec(),
        digest: digest(&projection)?,
    })
}

/// Serialize `export` as `plan.json` text: pretty-printed, with a trailing newline.
pub fn plan_export_json(export: &PlanExport) -> Result<String, serde_json::Error> {
    let mut text = serde_json::to_string_pretty(export)?;
    text.push('\n');
    Ok(text)
}

/// Every container, in dependency order: each module's container-placed services, then its
/// helpers.
fn containers(plan: &Plan) -> Vec<ContainerDto> {
    let mut out = Vec::new();
    for module in &plan.graph.nodes {
        let component = module.id().as_str();
        for spec in plan.services.get(module.id()).into_iter().flatten() {
            if !matches!(spec.placement, Placement::Container { .. }) {
                continue;
            }
            out.push(ContainerDto {
                name: spec.name.clone(),
                component: component.to_string(),
                kind: ContainerKind::Service,
            });
        }
        // A hosted provider runs nothing, so it has no helpers to list.
        if module.hosted().is_some() {
            continue;
        }
        let Some(binding) = plan.bindings.get(module.id()) else {
            continue;
        };
        for helper in module.helpers_in(&binding.knobs).iter() {
            out.push(ContainerDto {
                name: helper.name.clone(),
                component: component.to_string(),
                kind: match helper.kind {
                    HelperKind::Init => ContainerKind::Init,
                    HelperKind::Sidecar => ContainerKind::Sidecar,
                },
            });
        }
    }
    out
}

/// `sha256:<hex>` over `value`'s JSON. Struct fields serialize in declaration order and every
/// map is a `BTreeMap`, so the JSON is canonical for a given value.
fn digest(value: &impl Serialize) -> Result<String, serde_json::Error> {
    let hash = Sha256::digest(serde_json::to_vec(value)?);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("sha256:{hex}"))
}
