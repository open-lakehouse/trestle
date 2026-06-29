//! The inlined baseline catalog, built purely in Rust (no I/O, no YAML).
//!
//! These are the common local-Lakehouse modules — a gateway, a relational store, an
//! object store, experiment tracking, a data catalog, a query engine, tracing,
//! notebooks, and the Databricks app-runtime contract — transcribed from trestle's
//! `local-stack-*` components. They are the corpus the planner is validated against:
//! planning the default selection must re-derive the routes and materialize the
//! artifacts these components ship today (see the crate's golden tests).
//!
//! # How a module encodes its routing facts (without authoring routes)
//!
//! A module declares only *intent* on its endpoints; the planner assigns the actual
//! prefixes/rewrites. The facts the planner needs to reproduce a service-specific
//! rewrite are kept in [`Provides::extras`](crate::Provides::extras), not in authored
//! routes:
//!
//! - `base_path` — where the service serves itself (e.g. MLflow under `/mlflow`). The
//!   planner derives an API rewrite as `base_path + client_path`, and uses it as a
//!   prefixable UI's chosen base path.
//! - `api_prefix:<endpoint_id>` — an API endpoint's client mount prefix (the endpoint's
//!   own `path` stays empty so the resolver does not double it).
//! - `rewrite:<client_prefix>` — an explicit per-route rewrite override for the rare
//!   case the derived rule is wrong (e.g. MLflow's `/api/2.0/otel`). An **empty** value
//!   forces passthrough (no rewrite emitted).
//!
//! # Compose fragments
//!
//! Each module's compose `services:` snippet lives as a sibling `.yaml` file in
//! `fragments/`, embedded via `include_str!` and carried on the module's
//! [`RenderSpec::Static`]. Snippets use only `${VAR}` substitution; the one
//! stack-dependent part — SeaweedFS's bucket-init lines — is a `${S3_BUCKET_MB_LINES}`
//! placeholder the planner fills from the aggregated `s3_buckets`.

use super::Catalog;
use crate::endpoint::{Endpoint, RouteIntent, Scheme};
use crate::module::{
    Injection, Module, ModuleId, Provides, RenderSpec, ResourceDemand, ResourceProvider,
};
use crate::placement::Placement;
use crate::role::{Role, ServiceSpec};

/// The well-known extras key naming where a service serves itself.
pub const BASE_PATH_EXTRA: &str = "base_path";
/// Prefix for the well-known extras keys overriding a route's rewrite, e.g.
/// `rewrite:/api/2.0/otel`. An empty value forces passthrough.
pub const REWRITE_OVERRIDE_PREFIX: &str = "rewrite:";
/// Prefix for the well-known extras key declaring an API endpoint's client mount
/// prefix, keyed by endpoint id (e.g. `api_prefix:tracking` → `/api/2.0/mlflow`).
///
/// The mount prefix lives here rather than on the endpoint's own `path` because the
/// [`address`](crate::address) resolver composes a gateway URL as
/// `join(AssignedRoute.prefix, endpoint.path)`: the planner puts the full client
/// mount in `AssignedRoute.prefix`, so the endpoint's `path` must stay empty or the
/// path would double. An [`Api`](crate::RouteIntent::Api) endpoint therefore declares
/// its mount here and leaves `path` empty.
pub const API_PREFIX_EXTRA: &str = "api_prefix:";

/// The placeholder SeaweedFS's fragment uses for the planner-injected per-bucket
/// `aws s3 mb` lines.
pub const S3_BUCKET_MB_LINES_VAR: &str = "S3_BUCKET_MB_LINES";
/// The placeholder Azurite's fragment uses for the planner-injected per-container
/// `az storage container create` lines.
pub const AZURE_CONTAINER_CREATE_LINES_VAR: &str = "AZURE_CONTAINER_CREATE_LINES";

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
        trino(),
        jaeger(),
        notebooks(),
        databricks_emulator_env(),
    ])
    .with_default_provider("object_store", "local-stack-seaweedfs")
}

/// The default lakehouse selection: the always-on gateway plus the default category
/// picks (a relational store and an object store), mirroring trestle's base
/// `always: [envoy]` + default `storage`/`metadata_db` choices. Other modules
/// (catalog, ml, query engine, observability, notebooks) are opt-in.
pub fn baseline_selection() -> crate::plan_env::Selection {
    crate::plan_env::Selection::modules([
        "local-stack-envoy",
        "local-stack-postgres",
        "local-stack-seaweedfs",
    ])
}

/// Helper: a container-placed service.
fn container(service: &str) -> Placement {
    Placement::Container {
        service: service.to_string(),
    }
}

/// Helper: a `RenderSpec::Static` carrying just a compose fragment (no extra files).
fn fragment(text: &str) -> RenderSpec {
    RenderSpec::Static {
        fragment: text.to_string(),
        files: vec![],
    }
}

/// `local-stack-envoy` — the single-port gateway. It has no surface endpoints of its
/// own (it *is* the surface); its listening port is supplied to the planner via
/// `TopologyCtx`, not as a routed endpoint. Its rendered Envoy bootstrap config is a
/// planner-emitted artifact, not part of this fragment (which only declares the
/// container that mounts it).
fn envoy() -> Module {
    let mut provides = Provides::default();
    provides.env_vars.insert("ENVOY_PORT".into(), "9080".into());
    Module {
        id: ModuleId::from("local-stack-envoy"),
        display_name: Some("Envoy gateway".into()),
        summary: Some("Single-port gateway, Databricks-shaped URL rewrites.".into()),
        category: Some("gateway".into()),
        provider_of: Some("gateway".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "envoy".into(),
            role: Role::new("gateway"),
            placement: container("envoy"),
            endpoints: vec![Endpoint {
                id: "http".into(),
                scheme: Scheme::Http,
                internal_port: 10000,
                host_port: Some(9080),
                intent: RouteIntent::Internal,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/envoy.yaml")),
    }
}

/// `local-stack-postgres` — the relational store. Internal-only (a database port is
/// never on the gateway surface).
fn postgres() -> Module {
    let mut provides = Provides::default();
    for (k, v) in [
        ("POSTGRES_USER", "postgres"),
        ("POSTGRES_PASSWORD", "postgres"),
        ("POSTGRES_DB", "postgres"),
        ("POSTGRES_PORT", "5432"),
        ("PGWEB_PORT", "8081"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }
    // Provisions `postgres_database` resources; vends a connection-string coordinate.
    let mut pg = ResourceProvider::default();
    pg.coordinates.insert(
        "url".into(),
        "postgresql://${POSTGRES_USER:-postgres}:${POSTGRES_PASSWORD:-postgres}@db:5432/{name}"
            .into(),
    );
    provides
        .resource_kinds
        .insert("postgres_database".into(), pg);
    Module {
        id: ModuleId::from("local-stack-postgres"),
        display_name: Some("Postgres".into()),
        summary: Some("Postgres 16; auto-creates DBs other modules declare.".into()),
        category: Some("metadata_db".into()),
        provider_of: Some("relational_db".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "db".into(),
            role: Role::new("relational_db"),
            placement: container("db"),
            endpoints: vec![Endpoint {
                id: "sql".into(),
                scheme: Scheme::Tcp,
                internal_port: 5432,
                host_port: Some(5432),
                intent: RouteIntent::Internal,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/postgres.yaml")),
    }
}

/// `local-stack-seaweedfs` — the S3-compatible object store. Reached directly by SDKs
/// at its host port (not multiplexed behind the gateway). Its compose fragment carries
/// a `${S3_BUCKET_MB_LINES}` placeholder the planner fills from the aggregated bucket
/// list.
fn seaweedfs() -> Module {
    let mut provides = Provides::default();
    for (k, v) in [
        ("AWS_ACCESS_KEY_ID", "seaweedfs"),
        ("AWS_SECRET_ACCESS_KEY", "seaweedfs"),
        ("AWS_DEFAULT_REGION", "us-east-1"),
        ("SEAWEEDFS_S3_PORT", "9000"),
        ("SEAWEEDFS_MASTER_PORT", "9333"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }
    // The S3 flavour of the `object_store` role. Fills the role's coordinate contract:
    // `artifacts_uri` (the `s3://` destination), `endpoint`, and the `provider_kind` tag.
    let mut object_store = ResourceProvider {
        provider_kind: Some("s3".into()),
        ..Default::default()
    };
    object_store
        .coordinates
        .insert("artifacts_uri".into(), "s3://{name}".into());
    object_store
        .coordinates
        .insert("bucket".into(), "{name}".into());
    object_store
        .coordinates
        .insert("endpoint".into(), "http://seaweedfs:8333".into());
    provides
        .resource_kinds
        .insert("object_store".into(), object_store);
    Module {
        id: ModuleId::from("local-stack-seaweedfs"),
        display_name: Some("SeaweedFS (local S3)".into()),
        summary: Some("Self-hosted S3-compatible object store.".into()),
        category: Some("storage".into()),
        provider_of: Some("object_store".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "seaweedfs".into(),
            role: Role::new("object_store"),
            placement: container("seaweedfs"),
            endpoints: vec![Endpoint {
                id: "s3".into(),
                scheme: Scheme::Http,
                internal_port: 8333,
                host_port: Some(9000),
                intent: RouteIntent::Internal,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/seaweedfs.yaml")),
    }
}

/// `local-stack-azurite` — the Azure Blob flavour of the `object_store` role, an
/// alternative to SeaweedFS. Preferred in environments that need Azure-shaped storage
/// (e.g. for local Unity Catalog credential vending). When chosen, it provisions the
/// demanded containers; its `provides.env_vars` carry the Azure connection string and
/// SeaweedFS is not deployed, so no `AWS_*` keys enter the stack.
fn azurite() -> Module {
    const CONN: &str = "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://azurite:10000/devstoreaccount1;";
    let mut provides = Provides::default();
    provides
        .env_vars
        .insert("AZURE_STORAGE_CONNECTION_STRING".into(), CONN.into());
    provides
        .env_vars
        .insert("AZURITE_BLOB_PORT".into(), "10000".into());

    // The Azure flavour of `object_store`: same contract coordinate names as SeaweedFS,
    // filled with the `wasbs://` shape and an `azure_blob` provider_kind tag.
    let mut object_store = ResourceProvider {
        provider_kind: Some("azure_blob".into()),
        ..Default::default()
    };
    object_store.coordinates.insert(
        "artifacts_uri".into(),
        "wasbs://{name}@devstoreaccount1.blob.core.windows.net".into(),
    );
    object_store
        .coordinates
        .insert("bucket".into(), "{name}".into());
    object_store.coordinates.insert(
        "endpoint".into(),
        "http://azurite:10000/devstoreaccount1".into(),
    );
    provides
        .resource_kinds
        .insert("object_store".into(), object_store);

    Module {
        id: ModuleId::from("local-stack-azurite"),
        display_name: Some("Azurite (local Azure Blob)".into()),
        summary: Some("Azure Blob emulator; object store with Azure-shaped wiring.".into()),
        category: Some("storage".into()),
        provider_of: Some("object_store".into()),
        requires: vec![],
        // The two object-store providers are mutually exclusive in one environment.
        conflicts_with: vec![ModuleId::from("local-stack-seaweedfs")],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "azurite".into(),
            role: Role::new("object_store"),
            placement: container("azurite"),
            endpoints: vec![Endpoint {
                id: "blob".into(),
                scheme: Scheme::Http,
                internal_port: 10000,
                host_port: Some(10000),
                intent: RouteIntent::Internal,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/azurite.yaml")),
    }
}

/// `local-stack-mlflow` — experiment tracking. Fronts three ways behind the gateway:
/// the Databricks-shaped tracking API, the OTel ingest path, and the UI. It serves
/// itself under `/mlflow`, so the tracking API rewrites under that base; the OTel path
/// is the override exception (passes through unchanged).
fn mlflow() -> Module {
    let mut provides = Provides::default();
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), "/mlflow".into());
    provides.extras.insert(
        format!("{API_PREFIX_EXTRA}tracking"),
        "/api/2.0/mlflow".into(),
    );
    provides
        .extras
        .insert(format!("{API_PREFIX_EXTRA}otel"), "/api/2.0/otel".into());
    // The OTel route passes through unchanged (empty override == no rewrite).
    provides.extras.insert(
        format!("{REWRITE_OVERRIDE_PREFIX}/api/2.0/otel"),
        String::new(),
    );
    for (k, v) in [
        (
            "MLFLOW_TRACKING_URI",
            "http://localhost:${ENVOY_PORT:-9080}",
        ),
        (
            "MLFLOW_S3_ENDPOINT_URL",
            "http://localhost:${SEAWEEDFS_S3_PORT:-9000}",
        ),
        ("MLFLOW_EXPERIMENT_NAME", "local-dev"),
    ] {
        provides.env_vars.insert(k.into(), v.into());
    }

    Module {
        id: ModuleId::from("local-stack-mlflow"),
        display_name: Some("MLflow tracking".into()),
        summary: Some("Experiment + model tracking; Databricks-shaped URLs.".into()),
        category: Some("ml".into()),
        provider_of: Some("experiment_tracking".into()),
        // Only the gateway is a hard module dependency; the relational store and
        // object store arrive via resource demands (auto-provisioned).
        requires: vec![ModuleId::from("local-stack-envoy")],
        conflicts_with: vec![],
        // MLflow's fragment hard-codes how it reaches Postgres/S3, so these demands
        // exist to *provision* the `mlflow` database and bucket (no coordinate
        // injection needed back).
        needs: vec![
            ResourceDemand {
                resource: "postgres_database".into(),
                name: "mlflow".into(),
                provider: None,
                inject: vec![],
            },
            ResourceDemand {
                resource: "object_store".into(),
                name: "mlflow".into(),
                provider: None,
                inject: vec![],
            },
        ],
        services: vec![ServiceSpec {
            name: "mlflow".into(),
            role: Role::new("experiment_tracking"),
            placement: container("mlflow"),
            endpoints: vec![
                Endpoint {
                    id: "tracking".into(),
                    scheme: Scheme::Http,
                    internal_port: 5000,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                },
                Endpoint {
                    id: "otel".into(),
                    scheme: Scheme::Http,
                    internal_port: 5000,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                },
                Endpoint {
                    id: "ui".into(),
                    scheme: Scheme::Http,
                    internal_port: 5000,
                    host_port: None,
                    intent: RouteIntent::UiPrefixable,
                    path: String::new(),
                },
            ],
            depends_on: vec!["db".into(), "seaweedfs".into()],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/mlflow.yaml")),
    }
}

/// `local-stack-unity-catalog` — the data catalog. Its REST API serves the
/// Databricks-shaped path at root, so `/api/2.1/unity-catalog` fronts with no rewrite;
/// a second `/unity-catalog` alias points at the same service.
fn unity_catalog() -> Module {
    let mut provides = Provides::default();
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), String::new());
    provides.extras.insert(
        format!("{API_PREFIX_EXTRA}rest"),
        "/api/2.1/unity-catalog".into(),
    );
    provides.extras.insert(
        format!("{API_PREFIX_EXTRA}rest_alias"),
        "/unity-catalog".into(),
    );
    provides
        .env_vars
        .insert("UC_IMAGE".into(), "unitycatalog/unitycatalog:latest".into());
    // `UC_DATABASE_URL` is no longer hard-coded: it's the connection-string coordinate
    // the relational-DB provider renders for UC's demanded `unitycatalog` database.

    Module {
        id: ModuleId::from("local-stack-unity-catalog"),
        display_name: Some("Unity Catalog".into()),
        summary: Some("Databricks UC server; Databricks-shaped REST API.".into()),
        category: Some("catalog".into()),
        provider_of: Some("data_catalog".into()),
        // Only the gateway is a hard module dependency; Postgres + S3 arrive as demands.
        requires: vec![ModuleId::from("local-stack-envoy")],
        conflicts_with: vec![],
        needs: vec![
            ResourceDemand {
                resource: "postgres_database".into(),
                name: "unitycatalog".into(),
                provider: None,
                // The provider's connection-string coordinate lands in UC's env as
                // `UC_DATABASE_URL`, which its fragment reads.
                inject: vec![Injection {
                    key: "UC_DATABASE_URL".into(),
                    coordinate: "url".into(),
                }],
            },
            ResourceDemand {
                resource: "object_store".into(),
                name: "unity".into(),
                provider: None,
                inject: vec![],
            },
        ],
        services: vec![ServiceSpec {
            name: "unitycatalog".into(),
            role: Role::new("data_catalog"),
            placement: container("unitycatalog"),
            endpoints: vec![
                Endpoint {
                    id: "rest".into(),
                    scheme: Scheme::Http,
                    internal_port: 8080,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                },
                Endpoint {
                    id: "rest_alias".into(),
                    scheme: Scheme::Http,
                    internal_port: 8080,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                },
            ],
            depends_on: vec!["db".into(), "seaweedfs".into()],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/unity-catalog.yaml")),
    }
}

/// `local-stack-trino` — distributed SQL engine, fronted at `/trino` (a prefixable UI).
fn trino() -> Module {
    let mut provides = Provides::default();
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), "/trino".into());
    provides.env_vars.insert("TRINO_PORT".into(), "8080".into());
    Module {
        id: ModuleId::from("local-stack-trino"),
        display_name: Some("Trino".into()),
        summary: Some("Distributed SQL engine (Iceberg, Delta, Hive, JDBC, S3).".into()),
        category: Some("query_engine".into()),
        provider_of: Some("sql_engine".into()),
        requires: vec![ModuleId::from("local-stack-envoy")],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "trino".into(),
            role: Role::new("sql_engine"),
            placement: container("trino"),
            endpoints: vec![Endpoint {
                id: "ui".into(),
                scheme: Scheme::Http,
                internal_port: 8080,
                host_port: Some(8080),
                intent: RouteIntent::UiPrefixable,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/trino.yaml")),
    }
}

/// `local-stack-jaeger` — all-in-one OTLP tracing backend, UI fronted at `/jaeger`.
fn jaeger() -> Module {
    let mut provides = Provides::default();
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), "/jaeger".into());
    provides.env_vars.insert(
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(),
        "http://localhost:4317".into(),
    );
    provides
        .env_vars
        .insert("JAEGER_UI_PORT".into(), "16686".into());
    Module {
        id: ModuleId::from("local-stack-jaeger"),
        display_name: Some("Jaeger tracing".into()),
        summary: Some("All-in-one OTLP tracing backend with the Jaeger UI.".into()),
        category: Some("observability".into()),
        provider_of: Some("tracing".into()),
        requires: vec![ModuleId::from("local-stack-envoy")],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "jaeger".into(),
            role: Role::new("tracing"),
            placement: container("jaeger"),
            endpoints: vec![
                Endpoint {
                    id: "ui".into(),
                    scheme: Scheme::Http,
                    internal_port: 16686,
                    host_port: Some(16686),
                    intent: RouteIntent::UiPrefixable,
                    path: String::new(),
                },
                // OTLP/gRPC ingest, reached directly (not gatewayed).
                Endpoint {
                    id: "otlp_grpc".into(),
                    scheme: Scheme::Grpc,
                    internal_port: 4317,
                    host_port: Some(4317),
                    intent: RouteIntent::Internal,
                    path: String::new(),
                },
            ],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/jaeger.yaml")),
    }
}

/// `local-stack-notebooks` — Marimo notebook server, fronted at `/notebooks`.
fn notebooks() -> Module {
    let mut provides = Provides::default();
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), "/notebooks".into());
    provides
        .env_vars
        .insert("NOTEBOOKS_PORT".into(), "8082".into());
    Module {
        id: ModuleId::from("local-stack-notebooks"),
        display_name: Some("Marimo notebooks".into()),
        summary: Some("Notebooks behind the gateway at /notebooks.".into()),
        category: Some("notebooks".into()),
        provider_of: Some("notebook_server".into()),
        requires: vec![ModuleId::from("local-stack-envoy")],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: "notebooks".into(),
            role: Role::new("notebook_server"),
            placement: container("notebooks"),
            endpoints: vec![Endpoint {
                id: "ui".into(),
                scheme: Scheme::Http,
                internal_port: 8080,
                host_port: Some(8082),
                intent: RouteIntent::UiPrefixable,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: fragment(include_str!("fragments/notebooks.yaml")),
    }
}

/// `databricks-emulator-env` — env-only module: the `DATABRICKS_*` contract Databricks
/// Apps inject, so app code reads the same names locally. No services, no fragment.
fn databricks_emulator_env() -> Module {
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
    Module {
        id: ModuleId::from("databricks-emulator-env"),
        display_name: Some("Databricks app runtime contract".into()),
        summary: Some("DATABRICKS_HOST / TOKEN / forwarded-user env vars apps expect.".into()),
        category: Some("app_runtime".into()),
        provider_of: Some("databricks_apps_contract".into()),
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![],
        provides,
        knobs: vec![],
        render: RenderSpec::default(),
    }
}
