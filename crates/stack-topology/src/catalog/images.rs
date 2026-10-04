//! Central registry of default container image references for the baseline catalog.
//!
//! Each default is exposed as a module `image` / `init_image` knob so
//! environments can override a single service's image via `--set` or `env.toml` without
//! editing templates. Bump versions here when refreshing the baseline stack.

/// Envoy gateway proxy.
pub const ENVOY: &str = "envoyproxy/envoy:v1.34-latest";
/// Authelia forward-auth provider.
pub const AUTHELIA: &str = "ghcr.io/authelia/authelia:4.39";
/// Postgres relational database.
pub const POSTGRES: &str = "postgres:16";
/// RustFS S3-compatible object store with STS. Pinned: the STS shim's action list was probed
/// against this release's policy parser.
pub const RUSTFS: &str = "rustfs/rustfs:1.0.1";
/// One-shot S3 bucket initializer for RustFS.
pub const RUSTFS_INIT: &str = "amazon/aws-cli:latest";
/// The Python runtime for RustFS's STS policy shim.
pub const STS_SHIM: &str = "python:3.13-alpine";
/// One-shot that mints the gateway's local CA (needs `openssl` and the JDK's `keytool`).
/// Noble, not the floating tag: its Ubuntu 26.04 rebuild segfaults on 6.8 VM kernels.
pub const CERTS: &str = "eclipse-temurin:17-jdk-noble";
/// Azurite Azure Blob emulator.
pub const AZURITE: &str = "mcr.microsoft.com/azure-storage/azurite:3.35.0";
/// One-shot container initializer for Azurite.
pub const AZURITE_INIT: &str = "mcr.microsoft.com/azure-cli:2.87.0";
/// MLflow tracking server.
pub const MLFLOW: &str = "ghcr.io/mlflow/mlflow:v3.10.1-full";
/// Unity Catalog server.
pub const UNITY_CATALOG: &str = "unitycatalog/unitycatalog:v0.6.0";
/// Jaeger all-in-one tracing backend.
pub const JAEGER: &str = "cr.jaegertracing.io/jaegertracing/jaeger:2.14.1";
/// Headwaters lineage service (migrate + serve).
pub const HEADWATERS: &str = "ghcr.io/open-lakehouse/headwaters:0.0.7";
