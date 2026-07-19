//! Lower a validated [`TrestleConfig`] to [`olai_codegen::CodeGenConfig`].
//!
//! This is the bridge between the user-facing nested config and the flat config
//! the generator consumes. Config supplies each crate's `src` root; this layer
//! appends the fixed convention subdirectory (`codegen` for handler/client glue,
//! `wasm` for browser bindings, `_gen` for the co-located model tree) and resolves
//! the result on disk (creating dirs if needed, since the generator owns
//! everything under them).

use std::fs;
use std::path::{Path, PathBuf};

use olai_codegen::{BindingsConfig, CodeGenConfig, CodeGenOutput, DEFAULT_TRANSPORT_TYPE_PATH};

use super::{GenerateConfig, MODELS_GEN_SUBDIR, Transport, TrestleConfig};
use crate::error::{Error, Result};

/// Convention subdir for handler/client/binding glue (mounted `mod codegen;`).
const CODEGEN_SUBDIR: &str = "codegen";
/// Convention subdir for the browser `#[wasm_bindgen]` bindings (mounted `mod wasm;`).
const WASM_SUBDIR: &str = "wasm";

impl TrestleConfig {
    /// Resolve output directories and lower to [`CodeGenConfig`].
    ///
    /// Call [`derive_defaults`](TrestleConfig::derive_defaults) and
    /// [`validate`](TrestleConfig::validate) first.
    pub fn to_codegen_config(&self) -> Result<CodeGenConfig> {
        self.generate.lower()
    }
}

impl GenerateConfig {
    fn lower(&self) -> Result<CodeGenConfig> {
        // Models + Axum extractors are co-located in `<models.dir>/_gen`. The
        // generator's `common` (extractor) output and the models `_gen` subdir are
        // the same directory, so `mod.rs` includes the buf-written model files as
        // plain siblings (`./<pkg>.rs`).
        let models_root = resolve_dir(&self.models.dir)?;
        let common = resolve_dir_joined(&self.models.dir, MODELS_GEN_SUBDIR)?;
        let models = Some(models_root);

        let server = self
            .servers
            .rest
            .then_some(self.server.output.as_deref())
            .flatten()
            .map(|src| resolve_dir_joined(src, CODEGEN_SUBDIR))
            .transpose()?;
        let client = self
            .clients
            .rust
            .as_ref()
            .map(|r| resolve_dir_joined(&r.output, CODEGEN_SUBDIR))
            .transpose()?;
        let python = self
            .clients
            .python
            .as_ref()
            .map(|p| resolve_dir_joined(&p.output, CODEGEN_SUBDIR))
            .transpose()?;
        let (node, node_ts, wasm) = match &self.clients.node {
            Some(n) => (
                n.napi
                    .as_ref()
                    .map(|b| resolve_dir_joined(&b.output, CODEGEN_SUBDIR))
                    .transpose()?,
                n.ts.as_ref()
                    .map(|b| resolve_dir_joined(&b.output, CODEGEN_SUBDIR))
                    .transpose()?,
                n.wasm
                    .as_ref()
                    .map(|b| resolve_dir_joined(&b.output, WASM_SUBDIR))
                    .transpose()?,
            ),
            None => (None, None, None),
        };

        let output = CodeGenOutput {
            common,
            models,
            models_subdir: MODELS_GEN_SUBDIR.to_string(),
            server,
            client,
            generate_resource_clients: self.server.resource_clients,
            python,
            node,
            node_ts,
            wasm,
            python_typings_filename: "client.pyi".to_string(),
        };

        let bindings = self.lower_bindings(&output)?;
        let transport_type_path = self.transport_type_path();

        Ok(CodeGenConfig {
            context_type_path: self
                .server
                .context_type
                .clone()
                .unwrap_or_else(|| "crate::api::RequestContext".to_string()),
            result_type_path: self
                .server
                .result_type
                .clone()
                .unwrap_or_else(|| "crate::Result".to_string()),
            models_path_template: self.models.path_template.clone().unwrap_or_default(),
            models_path_crate_template: self.models.path_crate_template.clone().unwrap_or_default(),
            output,
            generate_resource_enum: self.server.resource_enum,
            generate_store_integration: self.server.store_integration,
            error_type_path: self.server.error_type_path.clone(),
            generate_object_conversions: self.server.object_conversions,
            bindings,
            resource_store_crate_name: self
                .server
                .resource_store_crate_name
                .clone()
                .unwrap_or_else(|| "olai_store".to_string()),
            runtime: self.proto_lib.to_runtime(),
            transport_type_path,
            dual_transport: self
                .clients
                .rust
                .as_ref()
                .is_some_and(|r| r.dual_transport),
            client_protocols: self.client_protocols(),
            connect_client_path: self
                .clients
                .rust
                .as_ref()
                .and_then(|r| r.connect_client_path.clone()),
        })
    }

    /// Which client protocol layer(s) to emit for the generated Rust client.
    ///
    /// Maps the TOML `clients.rust.protocols` selection (defaulting to REST only) to the codegen
    /// [`ClientProtocols`](olai_codegen::ClientProtocols) set.
    fn client_protocols(&self) -> olai_codegen::ClientProtocols {
        match self.clients.rust.as_ref() {
            Some(rust) => olai_codegen::ClientProtocols {
                rest: rust
                    .protocols
                    .iter()
                    .any(|p| matches!(p, crate::config::ClientProtocol::Rest)),
                connect: rust
                    .protocols
                    .iter()
                    .any(|p| matches!(p, crate::config::ClientProtocol::Connect)),
            },
            None => olai_codegen::ClientProtocols::default(),
        }
    }

    /// The HTTP transport path for generated Rust clients. An explicit
    /// `transport_type_path` wins; otherwise the `transport` alias selects a
    /// built-in. Enabling the WASM browser client flips the default to the WASM
    /// transport so the generated client is dual-transport.
    fn transport_type_path(&self) -> String {
        if let Some(rust) = &self.clients.rust {
            if let Some(path) = &rust.transport_type_path {
                return path.clone();
            }
            if rust.transport == Transport::Wasm {
                return "olai_http_wasm::WasmClient".to_string();
            }
        }
        // A WASM browser client implies the wasm transport even without an
        // explicit Rust client transport selection.
        if self.clients.node.as_ref().is_some_and(|n| n.wasm.is_some()) {
            return "olai_http_wasm::WasmClient".to_string();
        }
        DEFAULT_TRANSPORT_TYPE_PATH.to_string()
    }

    fn lower_bindings(&self, output: &CodeGenOutput) -> Result<Option<BindingsConfig>> {
        let has_bindings_output = output.python.is_some()
            || output.node.is_some()
            || output.node_ts.is_some()
            || output.wasm.is_some();
        if !has_bindings_output {
            return Ok(None);
        }

        let b = self.bindings.as_ref();
        let py = self.clients.python.as_ref();
        let node = self.clients.node.as_ref();

        let require = |value: Option<String>, what: &str| -> Result<String> {
            match value {
                Some(v) if !v.trim().is_empty() => Ok(v),
                _ => Err(Error::other(format!(
                    "language bindings were requested but `{what}` is not set"
                ))),
            }
        };

        let (aggregate_client_name, client_crate_name, ts_error_base_class) =
            if output.node_ts.is_some() || output.wasm.is_some() {
                (
                    require(
                        b.and_then(|b| b.aggregate_client_name.clone()),
                        "bindings.aggregate_client_name",
                    )?,
                    require(
                        b.and_then(|b| b.client_crate_name.clone()),
                        "bindings.client_crate_name",
                    )?,
                    require(
                        b.and_then(|b| b.error_base_class.clone()),
                        "bindings.error_base_class",
                    )?,
                )
            } else {
                Default::default()
            };

        let (py_error_type, py_result_type) = if output.python.is_some() {
            (
                require(py.map(|c| c.error_type.clone()), "python.error_type")?,
                require(py.map(|c| c.result_type.clone()), "python.result_type")?,
            )
        } else {
            Default::default()
        };

        let napi_error_ext_trait = if output.node.is_some() {
            require(
                node.and_then(|n| n.napi.as_ref().map(|b| b.error_ext_trait.clone())),
                "node.napi.error_ext_trait",
            )?
        } else {
            String::new()
        };

        Ok(Some(BindingsConfig {
            aggregate_client_name,
            client_crate_name,
            py_error_type,
            py_result_type,
            napi_error_ext_trait,
            typings_package_filter: py.and_then(|c| c.typings_package_filter.clone()),
            ts_error_base_class,
            ts_error_code_prefix: b
                .and_then(|b| b.error_code_prefix.clone())
                .unwrap_or_default(),
        }))
    }
}

/// Create the directory if missing (the generator owns everything under it) and
/// canonicalize it — `fs::canonicalize` errors on a missing path.
fn resolve_dir(p: &str) -> Result<PathBuf> {
    fs::create_dir_all(p).map_err(|e| Error::io_at(p, e))?;
    fs::canonicalize(PathBuf::from(p)).map_err(|e| Error::io_at(p, e))
}

/// Resolve `<root>/<subdir>` — the crate `src` root joined with its convention
/// subdirectory, creating it if missing and canonicalizing.
fn resolve_dir_joined(root: &str, subdir: &str) -> Result<PathBuf> {
    let joined = Path::new(root).join(subdir);
    let joined = joined.to_string_lossy();
    resolve_dir(&joined)
}
