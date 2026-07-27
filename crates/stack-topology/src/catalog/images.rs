//! Central registry of default container image references for the baseline catalog.
//!
//! Each default is exposed as a module `image` / `init_image` / `pgweb_image` knob so
//! environments can override a single service's image via `--set` or `env.toml` without
//! editing templates. Bump versions here when refreshing the baseline stack.

/// Envoy gateway proxy.
pub const ENVOY: &str = "envoyproxy/envoy:v1.34-latest";
/// Authelia forward-auth provider.
pub const AUTHELIA: &str = "ghcr.io/authelia/authelia:4.39";
/// Postgres relational database.
pub const POSTGRES: &str = "postgres:16";
/// pgweb database browser (optional profile).
pub const PGWEB: &str = "sosedoff/pgweb:latest";
/// SeaweedFS S3-compatible object store.
pub const SEAWEEDFS: &str = "chrislusf/seaweedfs:latest";
/// One-shot S3 bucket initializer for SeaweedFS.
pub const SEAWEEDFS_INIT: &str = "amazon/aws-cli:latest";
/// Azurite Azure Blob emulator.
pub const AZURITE: &str = "mcr.microsoft.com/azure-storage/azurite:3.35.0";
/// One-shot container initializer for Azurite.
pub const AZURITE_INIT: &str = "mcr.microsoft.com/azure-cli:2.87.0";
/// MLflow tracking server.
pub const MLFLOW: &str = "ghcr.io/mlflow/mlflow:v3.10.1-full";
/// Unity Catalog server.
pub const UNITY_CATALOG: &str = "unitycatalog/unitycatalog:main-2f2e32d";
/// Jaeger all-in-one tracing backend.
pub const JAEGER: &str = "cr.jaegertracing.io/jaegertracing/jaeger:2.14.1";
/// Headwaters lineage service (migrate + serve).
pub const HEADWATERS: &str = "ghcr.io/open-lakehouse/headwaters:0.0.7";
