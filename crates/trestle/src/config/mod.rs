//! Structured trestle project configuration (`trestle.yaml`).
//!
//! This is the canonical, *nested* config schema that replaced the historical
//! flat `generate:` block. It models the real shape of the codegen problem as a
//! small matrix —
//!
//! - **proto library**: [`ProtoLib::Prost`] | [`ProtoLib::Buffa`]
//! - **servers**: REST (Axum) and/or Connect RPC
//! - **clients**: Rust, Node (NAPI + TS, optionally WASM-browser), Python
//!
//! — and makes the cross-cell constraints explicit (Connect RPC and the WASM
//! browser client both require buffa). It is the single source of truth: it
//! lowers to [`olai_codegen::CodeGenConfig`] for `trestle generate` *and* emits
//! the project's `buf.gen.yaml` (see [`buf_gen`]).
//!
//! The file also carries project identity ([`ProjectMeta`]) — a stable root
//! `name` that most crate names derive from, plus a generated `id` used to
//! correlate CLI state across runs — and a schema [`version`](TrestleConfig::version).

mod buf_gen;
mod derive;
mod lower;
mod validate;

pub use buf_gen::emit_buf_gen;

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Current config schema version. Bumped on breaking schema changes so the
/// loader can detect (and eventually migrate) older shapes.
pub const CONFIG_VERSION: u32 = 1;

/// Top-level `trestle.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrestleConfig {
    /// Schema version. See [`CONFIG_VERSION`].
    #[serde(default = "default_version")]
    pub version: u32,

    /// Project identity + metadata.
    pub project: ProjectMeta,

    /// Code-generation configuration.
    pub generate: GenerateConfig,

    /// OpenAPI enrichment configuration (independent of codegen). Passed through
    /// untouched — `trestle enrich-openapi` owns this section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrich_openapi: Option<crate::cli::enrich_openapi::FileEnrichOpenApiConfig>,
}

fn default_version() -> u32 {
    CONFIG_VERSION
}

/// Project identity and free-form metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMeta {
    /// The project's **root name** — most crate/binding names derive from this
    /// (see [`derive`]). Required.
    pub name: String,

    /// Stable project identifier, generated once on first write (see
    /// [`TrestleConfig::ensure_id`]) and never hand-edited. Used to correlate
    /// CLI state / telemetry across invocations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The protobuf runtime the *generated* code consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtoLib {
    /// prost-generated models (historical default).
    #[default]
    Prost,
    /// buffa-generated models. Required for Connect RPC and the WASM browser client.
    Buffa,
}

impl ProtoLib {
    /// Lower to the codegen crate's [`Runtime`](olai_codegen::Runtime).
    pub fn to_runtime(self) -> olai_codegen::Runtime {
        match self {
            ProtoLib::Prost => olai_codegen::Runtime::Prost,
            ProtoLib::Buffa => olai_codegen::Runtime::Buffa,
        }
    }
}

/// Code-generation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateConfig {
    /// Protobuf runtime the generated code consumes.
    #[serde(default)]
    pub proto_lib: ProtoLib,

    /// Path to the compiled `FileDescriptorSet` — the foundational codegen input.
    /// Defaults to `api.bin`.
    #[serde(default = "default_descriptors")]
    pub descriptors: String,

    /// Which servers to emit.
    #[serde(default)]
    pub servers: Servers,

    /// Which clients to emit.
    #[serde(default)]
    pub clients: Clients,

    /// Shared TS/JS binding identity (used by `clients.node.ts` + `clients.node.wasm`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Bindings>,

    /// Generated model layout.
    pub models: Models,

    /// Server-side codegen knobs.
    #[serde(default)]
    pub server: Server,
}

fn default_descriptors() -> String {
    "api.bin".to_string()
}

/// Which servers to emit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Servers {
    /// Emit Axum REST handlers (via olai-codegen).
    #[serde(default)]
    pub rest: bool,
    /// Emit the Connect RPC facade (via buf plugins). Requires buffa.
    #[serde(default)]
    pub connect: bool,
}

/// Which clients to emit. Each field, when present, requests that client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clients {
    /// Rust client (olai-codegen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust: Option<RustClient>,
    /// Python (PyO3) client + `.pyi` typings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<PythonClient>,
    /// Node.js clients (NAPI / TypeScript / WASM browser).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeClient>,
}

/// The HTTP transport a generated Rust client stores and calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// `olai_http::CloudClient` — cloud/server transport (default).
    #[default]
    Cloud,
    /// `olai_http_wasm::WasmClient` — browser-buildable transport. Requires buffa.
    Wasm,
}

/// Wire protocol a generated Rust client speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientProtocol {
    /// HTTP/JSON, routed from `google.api.http` annotations (default).
    #[default]
    Rest,
    /// ConnectRPC, layered over the connect-rust generated service client.
    Connect,
}

/// Rust client config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustClient {
    /// Client crate `src` root. The generated client lands in the `codegen/`
    /// subdirectory beneath it.
    pub output: String,
    /// HTTP transport selector. Ignored if `transport_type_path` is set, or when
    /// `protocols` is `connect`-only (Connect dispatch owns its own transport).
    #[serde(default)]
    pub transport: Transport,
    /// Fully-qualified custom transport type path; overrides `transport`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_type_path: Option<String>,
    /// Which wire protocol layer(s) the generated client speaks. Defaults to `["rest"]`. List both
    /// (`["rest", "connect"]`) to emit a REST client and a ConnectRPC client side by side.
    #[serde(default = "default_protocols")]
    pub protocols: Vec<ClientProtocol>,
    /// Import path of the connect-rust generated client module (e.g.
    /// `"my_proto::connect_gen::my::pkg::v1"`). Required when `protocols` includes `connect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_client_path: Option<String>,
}

fn default_protocols() -> Vec<ClientProtocol> {
    vec![ClientProtocol::Rest]
}

/// Python (PyO3) client config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PythonClient {
    /// Python client crate `src` root. The generated client lands in the
    /// `codegen/` subdirectory beneath it.
    pub output: String,
    /// Fully-qualified Python error type. Required.
    pub error_type: String,
    /// Fully-qualified Python result type. Required.
    pub result_type: String,
    /// Substring filter for which packages get a `.pyi` stub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typings_package_filter: Option<String>,
}

/// Node.js client config (NAPI / TypeScript / WASM browser variants).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeClient {
    /// NAPI-RS native bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub napi: Option<NapiBindings>,
    /// NAPI TypeScript client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<TsBindings>,
    /// WASM `#[wasm_bindgen]` browser bindings + `client.d.ts`. Requires buffa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<WasmBindings>,
}

/// NAPI-RS native binding config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NapiBindings {
    /// NAPI crate `src` root. Generated bindings land in the `codegen/`
    /// subdirectory beneath it.
    pub output: String,
    /// Fully-qualified error extension trait. Required.
    pub error_ext_trait: String,
}

/// NAPI TypeScript client config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TsBindings {
    /// TypeScript client `src` root. The generated client lands in the `codegen/`
    /// subdirectory beneath it.
    pub output: String,
}

/// WASM browser binding config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmBindings {
    /// Client crate `src` root. The generated `#[wasm_bindgen]` bindings land in
    /// the `wasm/` subdirectory beneath it.
    pub output: String,
}

/// Shared TS/JS binding identity. Derived from [`ProjectMeta::name`] when unset
/// (see [`derive`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    /// Aggregate client class name (`<Pascal>Client`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_client_name: Option<String>,
    /// Client crate name (`<kebab>-client`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_crate_name: Option<String>,
    /// JS/TS error base class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_base_class: Option<String>,
    /// JS/TS error-code prefix (`<SCREAMING_SNAKE>_`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code_prefix: Option<String>,
}

/// Generated model layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    /// Models module directory (e.g. `crates/common/src/models`). Trestle writes
    /// the generated tree into the fixed `_gen` subdirectory beneath this, and the
    /// buf proto plugin co-locates the compiled model files there too. The
    /// hand-owned `<dir>/mod.rs` re-exports `_gen`.
    pub dir: String,
    /// Models crate name. Derived from project name (`<snake>_common`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crate_name: Option<String>,
    /// External model import path template (`{service}`/`{version}` placeholders).
    /// Derived from `crate_name` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_template: Option<String>,
    /// Crate-local model import path template. Defaults to
    /// `crate::models::{service}::{version}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_crate_template: Option<String>,
}

/// Fixed subdirectory name for the generated model tree (under [`Models::dir`]).
/// Hidden-dir convention, not user-configurable.
pub(crate) const MODELS_GEN_SUBDIR: &str = "_gen";

/// Server-side codegen knobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// Server crate `src` root (required when `servers.rest` is set). Generated
    /// handler traits + routes land in the `codegen/` subdirectory beneath it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Request context type path. Defaults to `crate::api::RequestContext`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_type: Option<String>,
    /// Result alias path. Defaults to `crate::Result`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_type: Option<String>,
    /// Emit the `Resource` / `ObjectLabel` enums.
    #[serde(default)]
    pub resource_enum: bool,
    /// Emit `olai_store::Label` integration.
    #[serde(default)]
    pub store_integration: bool,
    /// Emit `TryFrom<Resource>` conversions + `qualified_name()`.
    #[serde(default)]
    pub object_conversions: bool,
    /// Emit resource-scoped clients in addition to per-service clients.
    #[serde(default)]
    pub resource_clients: bool,
    /// Fully-qualified error type path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_type_path: Option<String>,
    /// Resource store crate name. Defaults to `olai_store`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_store_crate_name: Option<String>,
}

impl TrestleConfig {
    /// Load and parse a `trestle.yaml`, rejecting unknown future schema versions.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
        let cfg: TrestleConfig =
            serde_yaml::from_str(&text).map_err(|e| Error::yaml_at(path, e))?;
        if cfg.version > CONFIG_VERSION {
            return Err(Error::other(format!(
                "trestle.yaml declares version {} but this CLI only understands up to {}; \
                 upgrade trestle",
                cfg.version, CONFIG_VERSION
            )));
        }
        Ok(cfg)
    }

    /// Generate [`ProjectMeta::id`] iff absent. Only the write path calls this —
    /// `generate` never mutates the file.
    pub fn ensure_id(&mut self) {
        if self.project.id.is_none() {
            self.project.id = Some(uuid::Uuid::new_v4().to_string());
        }
    }

    /// Serialize to `path`. Refuses to overwrite an existing file unless `force`.
    pub fn write(&self, path: &Path, force: bool) -> Result<()> {
        if path.exists() && !force {
            return Err(Error::other(format!(
                "{} already exists (use --force to overwrite)",
                path.display()
            )));
        }
        let body = serde_yaml::to_string(self).map_err(Error::PlainYaml)?;
        let doc = format!(
            "# Trestle project config. Generated/edited by `trestle config`.\n\
             # `trestle generate -c trestle.yaml` reads this; `buf.gen.yaml` is derived from it.\n\n\
             {body}"
        );
        fs::write(path, doc).map_err(|e| Error::io_at(path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_yaml() -> &'static str {
        "version: 1\n\
         project:\n  name: demo\n\
         generate:\n  models:\n    dir: crates/common/src/models\n"
    }

    #[test]
    fn parses_minimal_and_applies_defaults() {
        let cfg: TrestleConfig = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.project.name, "demo");
        assert_eq!(cfg.generate.descriptors, "api.bin");
        assert_eq!(cfg.generate.proto_lib, ProtoLib::Prost);
        assert_eq!(cfg.generate.models.dir, "crates/common/src/models");
        assert!(!cfg.generate.servers.rest);
    }

    #[test]
    fn future_version_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("trestle.yaml");
        fs::write(
            &p,
            "version: 9999\nproject:\n  name: demo\ngenerate:\n  models:\n    dir: x\n",
        )
        .unwrap();
        assert!(TrestleConfig::load(&p).is_err());
    }

    #[test]
    fn ensure_id_is_idempotent() {
        let mut cfg: TrestleConfig = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert!(cfg.project.id.is_none());
        cfg.ensure_id();
        let id = cfg.project.id.clone().unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        cfg.ensure_id();
        assert_eq!(cfg.project.id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn roundtrips_through_yaml() {
        let mut cfg: TrestleConfig = serde_yaml::from_str(minimal_yaml()).unwrap();
        cfg.derive_defaults();
        cfg.ensure_id();
        let body = serde_yaml::to_string(&cfg).unwrap();
        let back: TrestleConfig = serde_yaml::from_str(&body).unwrap();
        assert_eq!(back.project.id, cfg.project.id);
        assert_eq!(
            back.generate.models.crate_name.as_deref(),
            Some("demo_common")
        );
    }
}
