//! Lower a validated [`TrestleConfig`] to [`olai_codegen::CodeGenConfig`].
//!
//! This is the bridge between the user-facing nested config and the flat config
//! the generator consumes. It resolves output directories on disk (creating them
//! if needed, since the generator owns everything under them) and computes the
//! relative path from the models subdirectory to the generated models `gen/` dir.

use std::fs;
use std::path::{Path, PathBuf};

use olai_codegen::{BindingsConfig, CodeGenConfig, CodeGenOutput, DEFAULT_TRANSPORT_TYPE_PATH};

use super::{GenerateConfig, Transport, TrestleConfig};
use crate::error::{Error, Result};

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
        let common = resolve_dir(&self.models.common_output)?;
        let models = self
            .models
            .parent_output
            .as_deref()
            .map(resolve_dir)
            .transpose()?;
        let server = self
            .servers
            .rest
            .then_some(self.server.output.as_deref())
            .flatten()
            .map(resolve_dir)
            .transpose()?;
        let client = self
            .clients
            .rust
            .as_ref()
            .map(|r| resolve_dir(&r.output))
            .transpose()?;
        let python = self
            .clients
            .python
            .as_ref()
            .map(|p| resolve_dir(&p.output))
            .transpose()?;
        let (node, node_ts, wasm) = match &self.clients.node {
            Some(n) => (
                n.napi
                    .as_ref()
                    .map(|b| resolve_dir(&b.output))
                    .transpose()?,
                n.ts.as_ref().map(|b| resolve_dir(&b.output)).transpose()?,
                n.wasm
                    .as_ref()
                    .map(|b| resolve_dir(&b.output))
                    .transpose()?,
            ),
            None => (None, None, None),
        };

        // Relative path from the models subdirectory to the generated models dir
        // (`common_output`). The generator emits model imports relative to this.
        let models_gen_dir = models.as_deref().map(|models_dir| {
            let subdir_path = models_dir.join(&self.models.subdir);
            relative_path(&subdir_path, &common)
        });

        let output = CodeGenOutput {
            common,
            models,
            models_subdir: self.models.subdir.clone(),
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
            models_gen_dir,
            resource_store_crate_name: self
                .server
                .resource_store_crate_name
                .clone()
                .unwrap_or_else(|| "olai_store".to_string()),
            runtime: self.proto_lib.to_runtime(),
            transport_type_path,
        })
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

/// POSIX-style relative path from `base` to `target` (both absolute).
fn relative_path(base: &Path, target: &Path) -> String {
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();

    let common_len = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let up_count = base_components.len() - common_len;
    let mut parts: Vec<std::borrow::Cow<str>> = (0..up_count).map(|_| "..".into()).collect();
    for comp in &target_components[common_len..] {
        parts.push(comp.as_os_str().to_string_lossy());
    }

    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}
