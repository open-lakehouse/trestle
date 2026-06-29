//! The inlined baseline catalog, built purely in Rust (no I/O, no YAML).
//!
//! These are the common local-Lakehouse modules — a gateway, a relational store, an
//! object store, experiment tracking, a data catalog, a query engine, tracing,
//! and the Databricks app-runtime contract — transcribed from trestle's
//! `local-stack-*` components. They are the corpus the planner is validated against:
//! planning the default selection must re-derive the routes and materialize the
//! artifacts these components ship today (see the crate's golden tests).
//!
//! # How a module encodes its routing facts (without authoring routes)
//!
//! A module declares only *intent* on its endpoints; the planner assigns the actual
//! prefixes/rewrites. The facts the planner needs to reproduce a service-specific rewrite
//! are typed fields on the services a module emits:
//!
//! - [`ServiceSpec::base_path`](crate::ServiceSpec::base_path) — where the service serves
//!   itself (e.g. MLflow under `/mlflow`). The planner derives an API rewrite as
//!   `base_path + client_path`, and uses it as a prefixable UI's chosen base path.
//! - [`Endpoint::mount_prefix`](crate::Endpoint::mount_prefix) — an `Api` endpoint's client
//!   mount prefix (the endpoint's own `path` stays empty so the resolver does not double it).
//! - [`Endpoint::rewrite`](crate::Endpoint::rewrite) — the [`Rewrite`](crate::Rewrite)
//!   policy: `Inherit` (derive from `base_path`), `Passthrough` (forward unchanged, e.g.
//!   MLflow's `/api/2.0/otel`), or `To(path)` (an explicit upstream path).
//!
//! Most modules need no logic and are built as [`DataModule`](crate::DataModule)s (their
//! static services are returned verbatim). A module that varies its topology with a knob —
//! e.g. [`headwaters`], which drops its UI route when its UI knob is off — is a hand-written
//! type implementing [`Module`](crate::Module).
//!
//! # Compose fragments
//!
//! Each module is a self-contained directory under the crate-root `templates/` tree (a
//! sibling of `src/`): a selectable module's compose `services:` snippet lives at
//! `templates/modules/<name>/compose.yaml.jinja`, and the always-on gateway's two template
//! faces live together under `templates/gateway/` (`compose.yaml.jinja` for the envoy
//! module fragment, `bootstrap.yaml.jinja` for the aggregated Envoy config rendered by
//! [`crate::render_envoy`]). Each snippet is embedded via `include_str!` (relative paths
//! reach out of `src/catalog/` as `../../templates/…`) and carried on the module's
//! [`RenderSpec`]. The `.jinja` suffix marks a MiniJinja template: every fragment is rendered
//! against the typed [`RenderCtx`](crate::RenderCtx), so it reads plan-resolved values directly
//! (`{{ env.DATA_ROOT }}`, `{{ connections.object_store.0.credential.access_key_id }}`) and
//! branches on a resolved [`Connection`](crate::Connection) where it must. Plan-time values are
//! rendered concrete; SeaweedFS's bucket-init lines, for instance, iterate the provisioned
//! `objects` rather than splicing a planner-filled placeholder.

use std::sync::Arc;

use super::Catalog;
use crate::connection::{Connection, ConnectionField, ConnectionTemplate, ObjectStoreCredential};
use crate::endpoint::{Endpoint, Rewrite, RouteIntent, Scheme};
use crate::module::{
    ConnectionBinding, DataModule, Knob, KnobKind, Module, ModuleId, Provides, RenderSpec,
    ResolvedKnobs, ResourceDemand,
};
use crate::placement::Placement;
use crate::render::RenderFile;
use crate::role::{Role, ServiceSpec};

/// The well-known [`Provides::extras`](crate::Provides::extras) key by which a
/// resource provider names the compose service a *consumer* should gate its startup
/// on, and the condition to wait for — `"<service>:<condition>"`, e.g.
/// `"db:service_healthy"` (Postgres) or `"seaweedfs-init:service_completed_successfully"`
/// (SeaweedFS). For each demand, the planner reads the *chosen* provider's value and
/// resolves it into a typed [`DepGate`](crate::DepGate) it hands the consumer's render via
/// [`RenderCtx::dependencies`](crate::RenderCtx) — so a consumer never hard-codes which
/// backend's init it waits for (it follows whichever provider the planner picked).
pub const DEP_GATE_EXTRA: &str = "dep_gate";

/// The render-env key carrying the stack's root data directory, injected into *every*
/// module's render env (see [`PlanCtx::data_root`](crate::PlanCtx::data_root)).
///
/// A module that persists state across `compose down`/`up` mounts its data under
/// `{{ env.DATA_ROOT }}/<module>` by convention, rather than hard-coding a `./.data/...` path
/// relative to the compose file. The value is resolved at *plan time* (baked into the rendered
/// fragment, like [`BASE_PATH`]), so relocating the whole stack's persistence is a single
/// [`PlanCtx::data_root`](crate::PlanCtx::data_root) knob — no per-fragment edit. A module with
/// no durable state simply ignores it.
///
/// [`BASE_PATH`]: crate::InjectedEnv
pub const DATA_ROOT_VAR: &str = "DATA_ROOT";

/// The default value injected for [`DATA_ROOT_VAR`]: `./.data`, relative to the compose
/// file's directory — matching where the fragments persisted data before the root was
/// centralized.
pub const DATA_ROOT_DEFAULT: &str = "./.data";

/// The inlined baseline catalog: all common local-Lakehouse modules.
///
/// The `object_store` role has two providers (SeaweedFS and Azurite); the catalog
/// default is SeaweedFS, so the baseline plans the S3 wiring out of the box. An
/// environment that prefers Azurite (e.g. for local UC credential vending) overrides
/// via [`PlanCtx::provider_preference`](crate::PlanCtx::provider_preference).
pub fn baseline_catalog() -> Catalog {
    Catalog::from_modules([
        envoy(),
        postgres(),
        seaweedfs(),
        azurite(),
        mlflow(),
        unity_catalog(),
        jaeger(),
        headwaters(),
        databricks_emulator_env(),
    ] as [Arc<dyn Module>; 9])
    .with_default_provider(Role::OBJECT_STORE, "seaweedfs")
    // No coordinate contracts: a provider vends a typed `Connection`, whose variant fields
    // are all mandatory, so completeness is a compile-time guarantee rather than a runtime
    // check.
}

/// The default lakehouse selection: the always-on gateway plus the default category
/// picks (a relational store and an object store), mirroring trestle's base
/// `always: [envoy]` + default `storage`/`metadata_db` choices. Other modules
/// (catalog, ml, query engine, observability) are opt-in.
pub fn baseline_selection() -> crate::plan_env::Selection {
    crate::plan_env::Selection::modules(["envoy", "postgres", "seaweedfs"])
}

/// Helper: a container-placed service.
fn container(service: &str) -> Placement {
    Placement::Container {
        service: service.to_string(),
    }
}

/// Helper: an [`Internal`](RouteIntent::Internal) endpoint (a database/inter-service port
/// the gateway never fronts), with the given scheme and ports.
fn internal_endpoint(
    id: &str,
    scheme: Scheme,
    internal_port: u16,
    host_port: Option<u16>,
) -> Endpoint {
    Endpoint {
        id: id.into(),
        scheme,
        internal_port,
        host_port,
        intent: RouteIntent::Internal,
        path: String::new(),
        mount_prefix: None,
        rewrite: Rewrite::Inherit,
    }
}

/// Helper: an HTTP [`Api`](RouteIntent::Api) endpoint mounted at `mount_prefix`, with the
/// given [`Rewrite`] policy. Its `path` stays empty so the resolver round-trips the prefix.
fn api_endpoint(id: &str, internal_port: u16, mount_prefix: &str, rewrite: Rewrite) -> Endpoint {
    Endpoint {
        id: id.into(),
        scheme: Scheme::Http,
        internal_port,
        host_port: None,
        intent: RouteIntent::Api,
        path: String::new(),
        mount_prefix: Some(mount_prefix.into()),
        rewrite,
    }
}

/// Helper: an HTTP [`UiPrefixable`](RouteIntent::UiPrefixable) endpoint (its base path is the
/// owning service's [`base_path`](ServiceSpec::base_path)).
fn ui_prefixable_endpoint(id: &str, internal_port: u16, host_port: Option<u16>) -> Endpoint {
    Endpoint {
        id: id.into(),
        scheme: Scheme::Http,
        internal_port,
        host_port,
        intent: RouteIntent::UiPrefixable,
        path: String::new(),
        mount_prefix: None,
        rewrite: Rewrite::Inherit,
    }
}

/// Helper: an HTTP [`Gatewayed`](RouteIntent::Gatewayed) endpoint (a whole-service backend,
/// e.g. an object store, reached through its own dedicated listener).
fn gatewayed_endpoint(id: &str, internal_port: u16) -> Endpoint {
    Endpoint {
        id: id.into(),
        scheme: Scheme::Http,
        internal_port,
        host_port: None,
        intent: RouteIntent::Gatewayed,
        path: String::new(),
        mount_prefix: None,
        rewrite: Rewrite::Inherit,
    }
}

/// Helper: a [`RenderSpec::Template`] (MiniJinja) carrying just a compose fragment (no extra
/// files). Every module's fragment is rendered against the typed [`RenderCtx`](crate::RenderCtx),
/// so it reads plan-resolved values (`{{ env.X }}`, `{{ connections.* }}`) directly.
fn template(text: &str) -> RenderSpec {
    RenderSpec::Template {
        fragment: text.to_string(),
        files: vec![],
    }
}

/// Helper: a [`RenderSpec::Template`] carrying a fragment plus mounted config files. Each
/// file's `path` is module-relative (the planner roots it under `modules/<id>/`), and each
/// file template is rendered against the same [`RenderCtx`](crate::RenderCtx) as the fragment.
fn template_with_files(text: &str, files: Vec<RenderFile>) -> RenderSpec {
    RenderSpec::Template {
        fragment: text.to_string(),
        files,
    }
}

/// `envoy` — the single-port gateway. It has no surface endpoints of its
/// own (it *is* the surface); its listening port is supplied to the planner via
/// `TopologyCtx`, not as a routed endpoint. Its rendered Envoy bootstrap config is a
/// planner-emitted artifact, not part of this fragment (which only declares the
/// container that mounts it).
fn envoy() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    provides.env_vars.insert("ENVOY_PORT".into(), "9080".into());
    Arc::new(DataModule {
        id: ModuleId::from("envoy"),
        display_name: Some("Envoy gateway".into()),
        summary: Some("Single-port gateway, Databricks-shaped URL rewrites.".into()),
        category: Some("gateway".into()),
        provider_of: Some("gateway".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "envoy".into(),
            role: Role::gateway(),
            placement: container("envoy"),
            endpoints: vec![internal_endpoint("http", Scheme::Http, 10000, Some(9080))],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![],
        render: template(include_str!("../../templates/gateway/compose.yaml.jinja")),
    })
}

/// `postgres` — the relational store. Internal-only (a database port is
/// never on the gateway surface).
fn postgres() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // The Postgres container reads these to initialize on first boot, so they stay in `.env`.
    // (Ports are written concretely in the fragment, so no `*_PORT` var is needed.)
    //
    // These are the *fixed* local-dev credentials, not an override surface: the connection URL
    // below bakes `postgres:postgres` concretely, and consumers (MLflow, UC, pgweb) embed that
    // resolved URL. Editing `POSTGRES_USER`/`POSTGRES_PASSWORD` in `.env` would re-credential the
    // container but NOT repoint those consumers, so they must move together. (A typed, mutable
    // relational credential on the connection is the future home for making this configurable.)
    for (k, v) in [
        ("POSTGRES_USER", "postgres"),
        ("POSTGRES_PASSWORD", "postgres"),
        ("POSTGRES_DB", "postgres"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }
    // Provisions `relational_db` resources; vends a typed relational connection. The
    // resource-kind key matches the service's role (`relational_db`) — one identity, so
    // role-exclusivity keys off the same name. The credential is folded into the URL,
    // concretely (matching the container's configured `postgres`/`postgres`) — no compose
    // `${VAR}` fallback for a consumer to resolve at run time.
    provides.resource_kinds.insert(
        Role::RELATIONAL_DB.into(),
        ConnectionTemplate(Connection::RelationalDb {
            url: "postgresql://postgres:postgres@db:5432/{name}".into(),
        }),
    );
    // A consumer that demands `relational_db` should wait for the `db` service to be healthy.
    provides
        .extras
        .insert(DEP_GATE_EXTRA.into(), "db:service_healthy".into());
    Arc::new(DataModule {
        id: ModuleId::from("postgres"),
        display_name: Some("Postgres".into()),
        summary: Some("Postgres 16; auto-creates DBs other modules declare.".into()),
        category: Some("metadata_db".into()),
        provider_of: Some("relational_db".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "db".into(),
            role: Role::relational_db(),
            placement: container("db"),
            endpoints: vec![internal_endpoint("sql", Scheme::Tcp, 5432, Some(5432))],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![],
        render: template_with_files(
            include_str!("../../templates/modules/postgres/compose.yaml.jinja"),
            vec![RenderFile {
                // Rendered from the databases the planner hands the provider via
                // `RenderCtx.objects`; co-located under `modules/postgres/` and mounted into
                // `/docker-entrypoint-initdb.d/` via the `postgres_init` config alias.
                path: "init-databases.sh".into(),
                contents: include_str!("../../templates/modules/postgres/init-databases.sh.jinja")
                    .into(),
                alias: Some("postgres_init".into()),
            }],
        ),
    })
}

/// `seaweedfs` — the S3-compatible object store. Reached directly by SDKs
/// at its host port (not multiplexed behind the gateway). Its compose fragment iterates the
/// buckets it provisions (`objects`) to build the one-shot init.
fn seaweedfs() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // No env vars are declared here: ports are written concretely in the fragment, and the S3
    // credentials (`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_DEFAULT_REGION`) are
    // derived from the typed S3 credential below (via `Connection::standard_env`), so they are
    // stated once and enter `.env` only when SeaweedFS is the chosen object_store — no `AWS_*`
    // leak under an Azure provider.
    //
    // The S3 flavour of the `object_store` role: the role-generic addressing
    // (`uri`/`bucket`/`endpoint`) plus an S3 credential a consumer may bind explicitly.
    provides.resource_kinds.insert(
        Role::OBJECT_STORE.into(),
        ConnectionTemplate(Connection::ObjectStore {
            uri: "s3://{name}".into(),
            bucket: "{name}".into(),
            // The in-network direct address. Because the `s3` endpoint is `Gatewayed`, the
            // planner rewrites this to the gateway origin (`http://<gateway>:<port>`) after
            // it allocates the store's dedicated listener, so consumers reach it via Envoy.
            endpoint: "http://seaweedfs:8333".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "seaweedfs".into(),
                secret_access_key: "seaweedfs".into(),
                region: "us-east-1".into(),
            },
        }),
    );
    // A consumer that demands `object_store` should wait for the one-shot bucket init to
    // finish (the buckets it needs exist only after `seaweedfs-init` completes).
    provides.extras.insert(
        DEP_GATE_EXTRA.into(),
        "seaweedfs-init:service_completed_successfully".into(),
    );
    Arc::new(DataModule {
        id: ModuleId::from("seaweedfs"),
        display_name: Some("SeaweedFS (local S3)".into()),
        summary: Some("Self-hosted S3-compatible object store.".into()),
        category: Some("storage".into()),
        provider_of: Some("object_store".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "seaweedfs".into(),
            role: Role::object_store(),
            placement: container("seaweedfs"),
            // No raw host port: the store is reached through the gateway on its own dedicated
            // listener (Gatewayed), not a direct compose `ports:` publish.
            endpoints: vec![gatewayed_endpoint("s3", 8333)],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![],
        render: template(include_str!(
            "../../templates/modules/seaweedfs/compose.yaml.jinja"
        )),
    })
}

/// `azurite` — the Azure Blob flavour of the `object_store` role, an
/// alternative to SeaweedFS. Preferred in environments that need Azure-shaped storage
/// (e.g. for local Unity Catalog credential vending). When chosen, it provisions the
/// demanded containers and vends the Azure connection string as an object_store
/// coordinate; SeaweedFS is not deployed, so no `AWS_*` keys enter the stack.
fn azurite() -> Arc<dyn Module> {
    const CONN: &str = "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://azurite:10000/devstoreaccount1;";
    let mut provides = Provides::default();
    // `AZURE_STORAGE_CONNECTION_STRING` is not hand-listed: the planner derives it from the
    // typed Azure credential below (via `Connection::standard_env`), so it is stated once and
    // enters `.env` only when Azurite is the chosen object_store. The blob port is the
    // emulator's fixed 10000, rendered directly in the fragment — no env var.

    // The Azure flavour of `object_store`: the same role-generic addressing as SeaweedFS
    // (`uri`/`bucket`/`endpoint`), filled with the `wasbs://` shape, plus an Azure
    // connection-string credential.
    provides.resource_kinds.insert(
        Role::OBJECT_STORE.into(),
        ConnectionTemplate(Connection::ObjectStore {
            uri: "wasbs://{name}@devstoreaccount1.blob.core.windows.net".into(),
            bucket: "{name}".into(),
            // In-network direct address; the planner rewrites the origin to the gateway's
            // dedicated listener (keeping the `/devstoreaccount1` path) since `blob` is
            // `Gatewayed`. The `BlobEndpoint=` inside the connection string is rewritten too.
            endpoint: "http://azurite:10000/devstoreaccount1".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: CONN.into(),
            },
        }),
    );
    // A consumer that demands `object_store` waits for the one-shot container init to finish.
    provides.extras.insert(
        DEP_GATE_EXTRA.into(),
        "azurite-init:service_completed_successfully".into(),
    );

    Arc::new(DataModule {
        id: ModuleId::from("azurite"),
        display_name: Some("Azurite (local Azure Blob)".into()),
        summary: Some("Azure Blob emulator; object store with Azure-shaped wiring.".into()),
        category: Some("storage".into()),
        provider_of: Some("object_store".into()),
        requires: vec![],
        // Same-role exclusivity is enforced by the planner (two unpinned object_store
        // providers in one environment is a `ConflictingRoleProviders` error), so no
        // hand-listed `conflicts_with` is needed here.
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "azurite".into(),
            role: Role::object_store(),
            placement: container("azurite"),
            // No raw host port: reached through the gateway's dedicated listener.
            endpoints: vec![gatewayed_endpoint("blob", 10000)],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![],
        render: template(include_str!(
            "../../templates/modules/azurite/compose.yaml.jinja"
        )),
    })
}

/// `mlflow` — experiment tracking. Fronts three ways behind the gateway:
/// the Databricks-shaped tracking API, the OTel ingest path, and the UI. It serves
/// itself under `/mlflow`, so the tracking API rewrites under that base; the OTel path
/// is the override exception (passes through unchanged).
fn mlflow() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    // `MLFLOW_S3_ENDPOINT_URL` and the AWS_* credentials are no longer declared here — they
    // are injected from the object_store provider's coordinates (see `needs` below), so they
    // follow whichever provider the planner chose and carry the in-network endpoint.
    for (k, v) in [
        (
            "MLFLOW_TRACKING_URI",
            "http://localhost:${ENVOY_PORT:-9080}",
        ),
        ("MLFLOW_EXPERIMENT_NAME", "local-dev"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }

    Arc::new(DataModule {
        id: ModuleId::from("mlflow"),
        display_name: Some("MLflow tracking".into()),
        summary: Some("Experiment + model tracking; Databricks-shaped URLs.".into()),
        category: Some("ml".into()),
        provider_of: Some("experiment_tracking".into()),
        // Only the gateway is a hard module dependency; the relational store and
        // object store arrive via resource demands (auto-provisioned).
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        // The relational store and object store arrive as demands. The role-generic
        // coordinates (`url`, `uri`, `endpoint`) are injected so the fragment no longer
        // hard-codes the backend URL, the `s3://` destination, or the endpoint. The S3
        // credentials themselves are *not* injected here — they are the chosen S3 provider's
        // own `env_vars` contribution (absent under an Azure provider), so MLflow's fragment
        // reads them with `:-` fallbacks.
        needs: vec![
            ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "mlflow".into(),
                provider: None,
                bind: ConnectionBinding {
                    bind: vec![(ConnectionField::Url, "MLFLOW_BACKEND_STORE_URI".into())],
                },
            },
            ResourceDemand {
                resource: Role::OBJECT_STORE.into(),
                name: "mlflow".into(),
                provider: None,
                bind: ConnectionBinding {
                    bind: vec![
                        (ConnectionField::Uri, "MLFLOW_ARTIFACTS_DESTINATION".into()),
                        (ConnectionField::Endpoint, "MLFLOW_S3_ENDPOINT_URL".into()),
                    ],
                },
            },
        ],
        service_specs: vec![ServiceSpec {
            name: "mlflow".into(),
            role: Role::experiment_tracking(),
            placement: container("mlflow"),
            // It serves itself under `/mlflow`: the tracking API rewrites under that base
            // (`Inherit`), while the OTel ingest path passes through unchanged
            // (`Passthrough`), and the UI is fronted at the base path.
            base_path: "/mlflow".into(),
            endpoints: vec![
                api_endpoint("tracking", 5000, "/api/2.0/mlflow", Rewrite::Inherit),
                api_endpoint("otel", 5000, "/api/2.0/otel", Rewrite::Passthrough),
                ui_prefixable_endpoint("ui", 5000, None),
            ],
            // No hand-listed startup edges: MLflow's `depends_on` is demand-driven. The
            // planner injects the chosen relational-db / object-store providers' gates
            // (service + condition) into the fragment, so the wait follows whichever backend
            // it picked rather than naming `db`/`seaweedfs` here.
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: template(include_str!(
            "../../templates/modules/mlflow/compose.yaml.jinja"
        )),
    })
}

/// `unity-catalog` — the data catalog. Its REST API serves the
/// Databricks-shaped path at root, so `/api/2.1/unity-catalog` fronts with no rewrite;
/// a second `/unity-catalog` alias points at the same service.
fn unity_catalog() -> Arc<dyn Module> {
    let provides = Provides::default();
    // The image is pinned in the fragment; UC reads its backend URL and object-store endpoint
    // straight from the typed connections (no `${VAR}` round-trip), so this module injects no
    // env vars of its own.

    Arc::new(DataModule {
        id: ModuleId::from("unity-catalog"),
        display_name: Some("Unity Catalog".into()),
        summary: Some("Databricks UC server; Databricks-shaped REST API.".into()),
        category: Some("catalog".into()),
        provider_of: Some("data_catalog".into()),
        // Only the gateway is a hard module dependency; Postgres + S3 arrive as demands.
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        // The relational store and object store arrive as demands, but nothing is bound into
        // UC's env: its fragment reads the resolved connections directly
        // (`connections.relational_db.0.url`, `connections.object_store.0.endpoint`, and the
        // typed credential), so no role-generic coordinate is round-tripped through `.env`.
        needs: vec![
            ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "unitycatalog".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            },
            ResourceDemand {
                resource: Role::OBJECT_STORE.into(),
                name: "unity".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            },
        ],
        service_specs: vec![ServiceSpec {
            name: "unitycatalog".into(),
            role: Role::data_catalog(),
            placement: container("unitycatalog"),
            // UC serves its REST API at root (empty base_path), so each mount `Inherit`s to no
            // rewrite: `/api/2.1/unity-catalog` fronts unchanged, and `/unity-catalog` is a
            // second alias to the same service.
            base_path: String::new(),
            endpoints: vec![
                api_endpoint("rest", 8080, "/api/2.1/unity-catalog", Rewrite::Inherit),
                api_endpoint("rest_alias", 8080, "/unity-catalog", Rewrite::Inherit),
            ],
            // Demand-driven, like MLflow: the planner injects the chosen providers' gates
            // (see the fragment's `depends_on`), so nothing is hand-listed here.
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: template(include_str!(
            "../../templates/modules/unity-catalog/compose.yaml.jinja"
        )),
    })
}

/// `jaeger` — all-in-one OTLP tracing backend, UI fronted at `/jaeger`.
fn jaeger() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    provides.env_vars.insert(
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(),
        "http://localhost:4317".into(),
    );
    Arc::new(DataModule {
        id: ModuleId::from("jaeger"),
        display_name: Some("Jaeger tracing".into()),
        summary: Some("All-in-one OTLP tracing backend with the Jaeger UI.".into()),
        category: Some("observability".into()),
        provider_of: Some("tracing".into()),
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "jaeger".into(),
            role: Role::tracing(),
            placement: container("jaeger"),
            // The Jaeger UI is fronted at `/jaeger`.
            base_path: "/jaeger".into(),
            endpoints: vec![
                ui_prefixable_endpoint("ui", 16686, Some(16686)),
                // OTLP/gRPC ingest, reached directly (not gatewayed).
                internal_endpoint("otlp_grpc", Scheme::Grpc, 4317, Some(4317)),
            ],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: template(include_str!(
            "../../templates/modules/jaeger/compose.yaml.jinja"
        )),
    })
}

/// The knob key gating Headwaters' bundled lineage UI.
const HEADWATERS_SERVE_UI: &str = "HEADWATERS_SERVE_UI";

/// `headwaters` — an OpenLineage lineage service, and the baseline's one *logic* module:
/// its single endpoint's routing varies with a knob, so it implements [`Module`] by hand
/// rather than being a [`DataModule`].
///
/// Headwaters serves its whole surface — web UI, REST read API, and OpenLineage ingest —
/// under a single base path `/lineage`. With the UI on (the default), it fronts as one
/// [`UiPrefixable`](RouteIntent::UiPrefixable) endpoint at `/lineage`: the planner mounts it
/// on the shared listener, forwards the prefix upstream unchanged, and tells the service
/// that base path through the `BASE_PATH` render handshake (the generated `config.toml`'s
/// `ui.base_path`). With the UI **off** (`HEADWATERS_SERVE_UI=false`), there is no UI to
/// base, so the same endpoint fronts as a plain [`Api`](RouteIntent::Api) at `/lineage`
/// (passthrough, no base-path handshake) — the API stays reachable through the gateway while
/// the UI-specific wiring drops away. This per-knob switch is exactly the logic a
/// `DataModule` cannot express, which is why Headwaters is a hand-written module.
///
/// It demands only a `relational_db` (named `lineage`); Postgres provisions the database and
/// the planner injects the `db:service_healthy` gate.
struct Headwaters {
    needs: Vec<ResourceDemand>,
    knobs: Vec<Knob>,
    provides: Provides,
    render: RenderSpec,
    requires: Vec<ModuleId>,
}

impl Headwaters {
    const ID: &'static str = "headwaters";

    fn new() -> Self {
        Headwaters {
            // Only the gateway is a hard module dependency; the relational store arrives as a
            // demand (auto-provisioned), and its startup gate is injected by the planner.
            requires: vec![ModuleId::from("envoy")],
            // The DSN is read straight from the resolved connection in the fragment
            // (`connections.relational_db.0.url`), like Unity Catalog — so nothing is bound
            // into Headwaters' env here.
            needs: vec![ResourceDemand {
                resource: Role::RELATIONAL_DB.into(),
                name: "lineage".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            }],
            // The one user-tunable surface Headwaters exposes: whether to serve the bundled
            // lineage UI. The value lands in the module's `InjectedEnv` under
            // `HEADWATERS_SERVE_UI` (the generated `config.toml` reads it as `ui.serve`), and
            // it also drives `services()`'s route choice below.
            knobs: vec![Knob {
                key: HEADWATERS_SERVE_UI.into(),
                title: Some("Serve the lineage UI".into()),
                kind: KnobKind::Bool,
                default: Some("true".into()),
                required: false,
                help: Some(
                    "Serve the bundled lineage web UI. Turn off to run the service API-only \
                     (e.g. when embedding a custom UI built on the shipped components)."
                        .into(),
                ),
            }],
            provides: Provides::default(),
            // The service config is rendered to a mounted `config.toml` (so an environment's
            // effective Headwaters config is inspectable on disk) rather than threaded as
            // individual env vars; the DSN stays in `DATABASE_URL`, which Headwaters overlays
            // over the file so the secret never lands in the checked-in config.
            render: template_with_files(
                include_str!("../../templates/modules/headwaters/compose.yaml.jinja"),
                vec![RenderFile {
                    path: "config.toml".into(),
                    contents: include_str!("../../templates/modules/headwaters/config.toml.jinja")
                        .into(),
                    alias: Some("headwaters_config".into()),
                }],
            ),
        }
    }
}

impl Module for Headwaters {
    fn id(&self) -> &ModuleId {
        static ID: std::sync::OnceLock<ModuleId> = std::sync::OnceLock::new();
        ID.get_or_init(|| ModuleId::from(Headwaters::ID))
    }
    fn display_name(&self) -> Option<&str> {
        Some("Headwaters lineage")
    }
    fn summary(&self) -> Option<&str> {
        Some("OpenLineage lineage service; UI + read API under one prefix.")
    }
    fn category(&self) -> Option<&str> {
        Some("observability")
    }
    fn provider_of(&self) -> Option<&str> {
        Some("lineage")
    }
    fn requires(&self) -> &[ModuleId] {
        &self.requires
    }
    fn needs(&self) -> &[ResourceDemand] {
        &self.needs
    }
    fn knobs(&self) -> &[Knob] {
        &self.knobs
    }
    fn provides(&self) -> &Provides {
        &self.provides
    }
    fn render(&self) -> &RenderSpec {
        &self.render
    }
    fn services(&self, knobs: &ResolvedKnobs) -> Vec<ServiceSpec> {
        // The single endpoint serves the whole surface under `/lineage`. With the UI on it is
        // a prefixable UI based at `/lineage`; with it off it is a plain API mounted at
        // `/lineage` (passthrough), so the read API + ingest stay reachable through the
        // gateway without the UI base-path handshake.
        let serve_ui = knobs.bool(HEADWATERS_SERVE_UI, true);
        let (endpoint, base_path) = if serve_ui {
            (ui_prefixable_endpoint("ui", 8091, None), "/lineage".into())
        } else {
            (
                api_endpoint("ui", 8091, "/lineage", Rewrite::Passthrough),
                String::new(),
            )
        };
        vec![ServiceSpec {
            name: "headwaters".into(),
            role: Role::lineage(),
            placement: container("headwaters"),
            base_path,
            endpoints: vec![endpoint],
            // Demand-driven, like MLflow: the planner injects the chosen relational-db
            // provider's gate into the fragment's `depends_on`.
            depends_on: vec![],
        }]
    }
}

fn headwaters() -> Arc<dyn Module> {
    Arc::new(Headwaters::new())
}

/// `databricks-emulator-env` — env-only module: the `DATABRICKS_*` contract Databricks
/// Apps inject, so app code reads the same names locally. No services, no fragment.
fn databricks_emulator_env() -> Arc<dyn Module> {
    let mut provides = Provides::default();
    for (k, v) in [
        ("LOCAL_DEV", "1"),
        ("DATABRICKS_HOST", "http://localhost:${ENVOY_PORT:-9080}"),
        ("DATABRICKS_TOKEN", "local-dev-token-do-not-use-in-prod"),
        ("DATABRICKS_CLIENT_ID", "local-dev"),
        ("DATABRICKS_WORKSPACE_ID", "local"),
        ("DATABRICKS_APP_URL", "http://localhost:${ENVOY_PORT:-9080}"),
        ("DATABRICKS_APP_PORT", "8080"),
        ("DATABRICKS_FORWARDED_USER", "local-developer@example.com"),
        ("DATABRICKS_FORWARDED_EMAIL", "local-developer@example.com"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }
    Arc::new(DataModule {
        id: ModuleId::from("databricks-emulator-env"),
        display_name: Some("Databricks app runtime contract".into()),
        summary: Some("DATABRICKS_HOST / TOKEN / forwarded-user env vars apps expect.".into()),
        category: Some("app_runtime".into()),
        provider_of: Some("databricks_apps_contract".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![],
        provides,
        knobs: vec![],
        render: RenderSpec::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::RouteIntent;

    #[test]
    fn headwaters_endpoint_switches_intent_on_the_ui_knob() {
        let m = headwaters();

        // UI on (the default): one prefixable-UI endpoint based at `/lineage`.
        let mut on = ResolvedKnobs::new();
        on.set(HEADWATERS_SERVE_UI, "true");
        let svcs = m.services(&on);
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs[0].base_path, "/lineage");
        assert_eq!(svcs[0].endpoints[0].intent, RouteIntent::UiPrefixable);
        assert_eq!(svcs[0].endpoints[0].mount_prefix, None);

        // UI off: the same endpoint becomes a plain API mounted at `/lineage`, passthrough,
        // and the service no longer claims a base path.
        let mut off = ResolvedKnobs::new();
        off.set(HEADWATERS_SERVE_UI, "false");
        let svcs = m.services(&off);
        assert_eq!(svcs[0].base_path, "");
        assert_eq!(svcs[0].endpoints[0].intent, RouteIntent::Api);
        assert_eq!(
            svcs[0].endpoints[0].mount_prefix.as_deref(),
            Some("/lineage")
        );
        assert_eq!(svcs[0].endpoints[0].rewrite, Rewrite::Passthrough);

        // The default (no knob value resolved) keeps the UI on.
        let svcs = m.services(&ResolvedKnobs::new());
        assert_eq!(svcs[0].endpoints[0].intent, RouteIntent::UiPrefixable);
    }
}
