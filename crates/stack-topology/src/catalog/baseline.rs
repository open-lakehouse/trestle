//! The inlined baseline catalog, built purely in Rust (no I/O, no YAML).
//!
//! These are the common local-Lakehouse modules — a gateway, a relational store, an
//! object store, experiment tracking, and a data catalog — transcribed from
//! trestle's `local-stack-*` components. They are the corpus the planner is validated
//! against: planning the default selection must re-derive the routes these components
//! ship today (see the crate's golden tests).
//!
//! # How a module encodes its routing facts (without authoring routes)
//!
//! A module declares only *intent* on its endpoints; the planner assigns the actual
//! prefixes/rewrites. The two facts the planner needs to reproduce a service-specific
//! rewrite are kept in [`Provides::extras`](crate::Provides::extras), not in authored
//! routes:
//!
//! - `base_path` — where the service serves itself (e.g. MLflow under `/mlflow`). The
//!   planner derives an API rewrite as `base_path + client_path`, and uses it as a
//!   prefixable UI's chosen base path.
//! - `rewrite:<client_prefix>` — an explicit per-route rewrite override for the rare
//!   case the derived rule is wrong (e.g. MLflow's `/api/2.0/otel`, which must strip
//!   to root rather than sit under `/mlflow`). An empty value means "rewrite to root".
//!
//! Extras are namespaced per module when surfaced, but the planner reads a module's
//! own extras directly while planning that module's services.

use super::Catalog;
use crate::endpoint::{Endpoint, RouteIntent, Scheme};
use crate::module::{Module, ModuleId, Provides};
use crate::placement::Placement;
use crate::role::{Role, ServiceSpec};

/// The well-known extras key naming where a service serves itself.
pub const BASE_PATH_EXTRA: &str = "base_path";
/// Prefix for the well-known extras keys overriding a route's rewrite, e.g.
/// `rewrite:/api/2.0/otel`.
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

/// The inlined baseline catalog: the common local-Lakehouse modules.
pub fn baseline_catalog() -> Catalog {
    Catalog::from_modules([envoy(), postgres(), seaweedfs(), mlflow(), unity_catalog()])
}

/// Helper: a container-placed service.
fn container(service: &str) -> Placement {
    Placement::Container {
        service: service.to_string(),
    }
}

/// `local-stack-envoy` — the single-port gateway. It has no surface endpoints of its
/// own (it *is* the surface); its listening port is supplied to the planner via
/// `TopologyCtx`, not as a routed endpoint.
fn envoy() -> Module {
    Module {
        id: ModuleId::from("local-stack-envoy"),
        display_name: Some("Envoy gateway".into()),
        summary: Some("Single-port gateway, Databricks-shaped URL rewrites.".into()),
        category: Some("gateway".into()),
        provider_of: Some("gateway".into()),
        requires: vec![],
        conflicts_with: vec![],
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
        provides: Provides::default(),
        knobs: vec![],
        render: Default::default(),
    }
}

/// `local-stack-postgres` — the relational store. Internal-only (a database port is
/// never on the gateway surface).
fn postgres() -> Module {
    Module {
        id: ModuleId::from("local-stack-postgres"),
        display_name: Some("Postgres".into()),
        summary: Some("Postgres 16; auto-creates DBs other modules declare.".into()),
        category: Some("metadata_db".into()),
        provider_of: Some("relational_db".into()),
        requires: vec![],
        conflicts_with: vec![],
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
        provides: Provides::default(),
        knobs: vec![],
        render: Default::default(),
    }
}

/// `local-stack-seaweedfs` — the S3-compatible object store. Its S3 API is an
/// [`Api`](RouteIntent::Api) endpoint served at root, so it fronts cleanly with no
/// rewrite.
fn seaweedfs() -> Module {
    let mut provides = Provides::default();
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), String::new());
    Module {
        id: ModuleId::from("local-stack-seaweedfs"),
        display_name: Some("SeaweedFS (local S3)".into()),
        summary: Some("Self-hosted S3-compatible object store.".into()),
        category: Some("storage".into()),
        provider_of: Some("object_store".into()),
        requires: vec![],
        conflicts_with: vec![],
        services: vec![ServiceSpec {
            name: "seaweedfs".into(),
            role: Role::new("object_store"),
            placement: container("seaweedfs"),
            endpoints: vec![Endpoint {
                id: "s3".into(),
                scheme: Scheme::Http,
                internal_port: 8333,
                host_port: Some(9000),
                // The S3 API is reached directly by SDKs at its host port today;
                // it is not multiplexed behind the shared gateway prefix.
                intent: RouteIntent::Internal,
                path: String::new(),
            }],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: Default::default(),
    }
}

/// `local-stack-mlflow` — experiment tracking. Fronts three ways behind the gateway:
/// the Databricks-shaped tracking API, the OTel ingest path, and the UI. It serves
/// itself under `/mlflow`, so the tracking API rewrites under that base; the OTel
/// path is the override exception (rewrites to root).
fn mlflow() -> Module {
    let mut provides = Provides::default();
    provides.postgres_databases.push("mlflow".into());
    provides.s3_buckets.push("mlflow".into());
    // The service serves itself under /mlflow.
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), "/mlflow".into());
    // Client mount prefixes for the two API endpoints (kept off `endpoint.path` so
    // the resolver's `join(prefix, path)` does not double the path).
    provides.extras.insert(
        format!("{API_PREFIX_EXTRA}tracking"),
        "/api/2.0/mlflow".into(),
    );
    provides
        .extras
        .insert(format!("{API_PREFIX_EXTRA}otel"), "/api/2.0/otel".into());
    // The OTel route is the exception: it must strip to root, not sit under /mlflow.
    provides.extras.insert(
        format!("{REWRITE_OVERRIDE_PREFIX}/api/2.0/otel"),
        String::new(),
    );

    Module {
        id: ModuleId::from("local-stack-mlflow"),
        display_name: Some("MLflow tracking".into()),
        summary: Some("Experiment + model tracking; Databricks-shaped URLs.".into()),
        category: Some("ml".into()),
        provider_of: Some("experiment_tracking".into()),
        requires: vec![
            ModuleId::from("local-stack-postgres"),
            ModuleId::from("local-stack-seaweedfs"),
            ModuleId::from("local-stack-envoy"),
        ],
        conflicts_with: vec![],
        services: vec![ServiceSpec {
            name: "mlflow".into(),
            role: Role::new("experiment_tracking"),
            placement: container("mlflow"),
            endpoints: vec![
                // Databricks-shaped tracking API. Its mount prefix is declared in
                // extras (`api_prefix:tracking`); `path` stays empty.
                Endpoint {
                    id: "tracking".into(),
                    scheme: Scheme::Http,
                    internal_port: 5000,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                },
                // OTel ingest. Mount prefix in extras (`api_prefix:otel`).
                Endpoint {
                    id: "otel".into(),
                    scheme: Scheme::Http,
                    internal_port: 5000,
                    host_port: None,
                    intent: RouteIntent::Api,
                    path: String::new(),
                },
                // The UI, served under the base path.
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
        render: Default::default(),
    }
}

/// `local-stack-unity-catalog` — the data catalog. Its REST API serves the
/// Databricks-shaped path at root, so `/api/2.1/unity-catalog` fronts with no
/// rewrite; a second `/unity-catalog` alias points at the same service.
fn unity_catalog() -> Module {
    let mut provides = Provides::default();
    provides.postgres_databases.push("unitycatalog".into());
    provides.s3_buckets.push("unity".into());
    // UC serves the Databricks-shaped path at its root → no base-path offset.
    provides
        .extras
        .insert(BASE_PATH_EXTRA.into(), String::new());
    // Client mount prefixes for the REST endpoint and its alias.
    provides.extras.insert(
        format!("{API_PREFIX_EXTRA}rest"),
        "/api/2.1/unity-catalog".into(),
    );
    provides.extras.insert(
        format!("{API_PREFIX_EXTRA}rest_alias"),
        "/unity-catalog".into(),
    );

    Module {
        id: ModuleId::from("local-stack-unity-catalog"),
        display_name: Some("Unity Catalog".into()),
        summary: Some("Databricks UC server; Databricks-shaped REST API.".into()),
        category: Some("catalog".into()),
        provider_of: Some("data_catalog".into()),
        requires: vec![
            ModuleId::from("local-stack-postgres"),
            ModuleId::from("local-stack-seaweedfs"),
            ModuleId::from("local-stack-envoy"),
        ],
        conflicts_with: vec![],
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
                // The shorter alias clients may also use.
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
        render: Default::default(),
    }
}
