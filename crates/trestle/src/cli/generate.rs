//! `trestle generate` — proto-driven code generation.
//!
//! Reads a YAML config (and/or CLI flags) and drives [`olai_codegen`] to emit
//! server/client/binding code from a compiled protobuf descriptor.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use olai_codegen::{
    BindingsConfig, CodeGenConfig, CodeGenOutput, Runtime, generate_code, parse_file_descriptor_set,
};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;

use crate::error::{Error, Result};

#[derive(Args, Clone)]
pub struct GenerateArgs {
    /// Path to a YAML config file; CLI flags override values from the file.
    #[clap(long, short = 'c')]
    pub config: Option<PathBuf>,

    #[clap(long, env = "UC_BUILD_OUTPUT_COMMON")]
    pub output_common: Option<String>,

    /// Parent models directory (e.g. `crates/common/src/models`). Generated files
    /// are written into `<output_models>/<models_subdir>/`.
    #[clap(long, env = "UC_BUILD_OUTPUT_MODELS")]
    pub output_models: Option<String>,

    /// Name of the generated subdirectory inside `output_models`. Defaults to `"_gen"`.
    #[clap(long, env = "UC_BUILD_MODELS_SUBDIR")]
    pub models_subdir: Option<String>,

    #[clap(long, env = "UC_BUILD_OUTPUT_SERVER")]
    pub output_server: Option<String>,

    #[clap(long, env = "UC_BUILD_OUTPUT_CLIENT")]
    pub output_client: Option<String>,

    #[clap(long, env = "UC_BUILD_OUTPUT_PYTHON")]
    pub output_python: Option<String>,

    #[clap(long, env = "UC_BUILD_OUTPUT_NODE")]
    pub output_node: Option<String>,

    #[clap(long, env = "UC_BUILD_OUTPUT_NODE_TS")]
    pub output_node_ts: Option<String>,

    #[clap(long, env = "UC_BUILD_OUTPUT_WASM")]
    pub output_wasm: Option<String>,

    #[clap(long, short, env = "UC_BUILD_DESCRIPTORS")]
    pub descriptors: Option<String>,

    /// Fully-qualified path to the request context type (e.g. `my_crate::Context`).
    #[clap(long, env = "UC_BUILD_CONTEXT_TYPE")]
    pub context_type: Option<String>,

    /// Fully-qualified path to the Result alias (e.g. `my_crate::Result`).
    #[clap(long, env = "UC_BUILD_RESULT_TYPE")]
    pub result_type: Option<String>,

    /// Template for the external model import path. Use `{service}` and `{version}` as placeholders.
    #[clap(long, env = "UC_BUILD_MODELS_PATH_TEMPLATE")]
    pub models_path_template: Option<String>,

    /// Template for the crate-local model import path. Use `{service}` and `{version}` as placeholders.
    #[clap(long, env = "UC_BUILD_MODELS_PATH_CRATE_TEMPLATE")]
    pub models_path_crate_template: Option<String>,

    /// Filename for the generated Python typings stub.
    #[clap(long, env = "UC_BUILD_PYTHON_TYPINGS_FILENAME")]
    pub python_typings_filename: Option<String>,
}

// ---------------------------------------------------------------------------
// Config file types
// ---------------------------------------------------------------------------

/// Top-level config file schema. Both `generate` and `enrich-openapi` share a
/// single file, each with its own optional section. CLI flags always override
/// config file values.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrestleConfig {
    /// Shared descriptor path used by both `generate` and `enrich_openapi`.
    pub descriptors: Option<String>,
    /// Path to buf.gen.yaml; used to auto-derive model path templates.
    pub buf_gen: Option<PathBuf>,
    pub generate: Option<FileGenerateConfig>,
    pub enrich_openapi: Option<crate::cli::enrich_openapi::FileEnrichOpenApiConfig>,
}

#[derive(Debug, serde::Deserialize)]
struct BufGenPlugin {
    remote: Option<String>,
    local: Option<String>,
    out: String,
}

#[derive(Debug, serde::Deserialize)]
struct BufGenConfig {
    plugins: Vec<BufGenPlugin>,
}

/// Find the model-plugin's output path in a buf.gen.yaml file.
///
/// The relevant plugin depends on the runtime: prost projects look for the
/// `neoeinstein-prost` plugin (excluding the `-serde` / `-tonic` companions),
/// while buffa projects look for the `buffa` plugin. The plugin may be declared
/// as either `remote:` (BSR) or `local:`, so both fields are matched.
fn find_models_out(buf_gen_path: &Path, runtime: Runtime) -> Result<PathBuf> {
    let text = fs::read_to_string(buf_gen_path).map_err(|e| Error::io_at(buf_gen_path, e))?;
    let cfg: BufGenConfig =
        serde_yaml::from_str(&text).map_err(|e| Error::yaml_at(buf_gen_path, e))?;
    let matches = |p: &BufGenPlugin| {
        let names = [p.remote.as_deref(), p.local.as_deref()];
        names.into_iter().flatten().any(|name| match runtime {
            Runtime::Prost => {
                name.contains("prost") && !name.contains("serde") && !name.contains("tonic")
            }
            Runtime::Buffa => name.contains("buffa") && !name.contains("packaging"),
        })
    };
    let plugin = cfg.plugins.iter().find(|p| matches(p)).ok_or_else(|| {
        let kind = match runtime {
            Runtime::Prost => "prost",
            Runtime::Buffa => "buffa",
        };
        Error::other(format!("no {kind} plugin found in buf.gen.yaml"))
    })?;
    Ok(PathBuf::from(&plugin.out))
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilePythonConfig {
    pub output: Option<String>,
    pub error_type: Option<String>,
    pub result_type: Option<String>,
    pub typings_package_filter: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileNodeConfig {
    pub output: Option<String>,
    pub napi_error_ext_trait: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileTsConfig {
    pub output: Option<String>,
    pub aggregate_client_name: Option<String>,
    pub client_crate_name: Option<String>,
    pub error_base_class: Option<String>,
    pub error_code_prefix: Option<String>,
}

/// WASM/browser `#[wasm_bindgen]` bindings + `.d.ts`. Reuses the shared binding identifiers
/// (`aggregate_client_name`, `ts_error_base_class`, …) from the `typescript`/`node` config; only
/// the output directory is wasm-specific. Implies `transport: wasm` and pairs with `runtime: buffa`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileWasmConfig {
    pub output: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileGenerateConfig {
    pub output_common: Option<String>,
    pub output_models: Option<String>,
    pub models_subdir: Option<String>,
    pub output_server: Option<String>,
    pub output_client: Option<String>,
    pub context_type: Option<String>,
    pub result_type: Option<String>,
    pub models_path_template: Option<String>,
    pub models_path_crate_template: Option<String>,
    pub models_crate_name: Option<String>,
    pub python_typings_filename: Option<String>,
    pub generate_resource_enum: Option<bool>,
    pub generate_store_integration: Option<bool>,
    pub error_type_path: Option<String>,
    pub generate_object_conversions: Option<bool>,
    pub generate_resource_clients: Option<bool>,
    pub resource_store_crate_name: Option<String>,
    /// Protobuf runtime the generated code consumes: `"prost"` (default) or `"buffa"`.
    pub runtime: Option<String>,
    /// Friendly selector for the generated client's HTTP transport: `"cloud"` (default,
    /// `olai_http::CloudClient`) or `"wasm"` (`olai_http_wasm::WasmClient`, a browser-buildable
    /// client where the browser attaches the session). Ignored if `transport_type_path` is set.
    pub transport: Option<String>,
    /// Fully-qualified path to the HTTP transport type generated clients store and call. Overrides
    /// `transport`. Defaults to `"olai_http::CloudClient"`. Use this for a custom transport that
    /// exposes the verb-builder / `json`/`query`/`send` / `status`/`bytes` surface generated
    /// clients require.
    pub transport_type_path: Option<String>,
    pub python: Option<FilePythonConfig>,
    pub node: Option<FileNodeConfig>,
    pub typescript: Option<FileTsConfig>,
    pub wasm: Option<FileWasmConfig>,
}

pub(crate) fn load_trestle_config(path: &Path) -> Result<TrestleConfig> {
    // Tolerate the legacy filename `proto-gen.yaml` by silently falling back if
    // the caller passes that path; modern projects ship `trestle.yaml`.
    let text = fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
    serde_yaml::from_str(&text).map_err(|e| Error::yaml_at(path, e))
}

/// Compute a POSIX-style relative path from `base` to `target` (both absolute).
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

pub fn run(mut args: GenerateArgs) -> Result<()> {
    let mut file_cfg = FileGenerateConfig::default();
    let mut buf_gen_path: Option<PathBuf> = None;

    // Merge config file values (CLI flags win — only fill in fields not already set).
    if let Some(config_path) = args.config.clone() {
        let file = load_trestle_config(&config_path)?;

        if args.descriptors.is_none() {
            args.descriptors = file.descriptors.clone();
        }

        buf_gen_path = file.buf_gen.clone();

        let cfg = file.generate.unwrap_or_default();

        macro_rules! fill {
            ($field:ident) => {
                if args.$field.is_none() {
                    args.$field = cfg.$field.clone();
                }
            };
        }

        fill!(output_common);
        fill!(output_models);
        fill!(models_subdir);
        fill!(output_server);
        fill!(output_client);
        fill!(context_type);
        fill!(result_type);
        fill!(models_path_template);
        fill!(models_path_crate_template);
        fill!(python_typings_filename);

        if args.output_python.is_none() {
            args.output_python = cfg.python.as_ref().and_then(|c| c.output.clone());
        }
        if args.output_node.is_none() {
            args.output_node = cfg.node.as_ref().and_then(|c| c.output.clone());
        }
        if args.output_node_ts.is_none() {
            args.output_node_ts = cfg.typescript.as_ref().and_then(|c| c.output.clone());
        }
        if args.output_wasm.is_none() {
            args.output_wasm = cfg.wasm.as_ref().and_then(|c| c.output.clone());
        }

        file_cfg = cfg;
    }

    let runtime = match file_cfg.runtime.as_deref() {
        None | Some("prost") => Runtime::Prost,
        Some("buffa") => Runtime::Buffa,
        Some(other) => {
            return Err(Error::other(format!(
                "unknown runtime `{other}` in trestle.yaml (expected `prost` or `buffa`)"
            )));
        }
    };

    // Auto-derive path templates from buf.gen.yaml when not explicitly set. The
    // model plugin's `out` dir (prost or buffa, depending on the runtime) anchors
    // the relative import path the generated code uses to reach the models.
    let mut models_out_abs: Option<PathBuf> = None;
    if args.models_path_template.is_none() {
        if let Some(ref bgp) = buf_gen_path {
            let models_out = find_models_out(bgp, runtime)?;
            models_out_abs =
                Some(fs::canonicalize(&models_out).map_err(|e| Error::io_at(&models_out, e))?);
            let crate_name = file_cfg.models_crate_name.as_deref().ok_or_else(|| {
                Error::other("models_crate_name is required in generate config when buf_gen is set")
            })?;
            args.models_path_template =
                Some(format!("{crate_name}::models::{{service}}::{{version}}"));
        }
    }
    if args.models_path_crate_template.is_none() && buf_gen_path.is_some() {
        args.models_path_crate_template = Some("crate::models::{service}::{version}".to_string());
    }
    if models_out_abs.is_none() {
        if let Some(ref bgp) = buf_gen_path {
            let models_out = find_models_out(bgp, runtime)?;
            if models_out.exists() {
                models_out_abs =
                    Some(fs::canonicalize(&models_out).map_err(|e| Error::io_at(&models_out, e))?);
            }
        }
    }

    let descriptors = args
        .descriptors
        .as_deref()
        .ok_or_else(|| {
            Error::other(
                "required argument missing: --descriptors (or set it in the config file under \
                 generate.descriptors or top-level descriptors)",
            )
        })?
        .to_owned();
    let output_common = args
        .output_common
        .as_deref()
        .ok_or_else(|| {
            Error::other(
                "required argument missing: --output-common (or set it in the config file under \
                 generate.output_common)",
            )
        })?
        .to_owned();

    let descriptor_path =
        fs::canonicalize(PathBuf::from(&descriptors)).map_err(|e| Error::io_at(&descriptors, e))?;
    let descriptor_bytes =
        fs::read(&descriptor_path).map_err(|e| Error::io_at(&descriptor_path, e))?;
    let file_descriptor_set = FileDescriptorSet::parse_from_bytes(&descriptor_bytes)
        .map_err(|e| Error::other(format!("failed to parse descriptor: {e}")))?;

    let metadata = parse_file_descriptor_set(&file_descriptor_set)?;

    // Output directories may not exist yet on a freshly-scaffolded tree (the
    // generator owns everything under them), so create them before canonicalizing
    // — `fs::canonicalize` errors on a missing path.
    let resolve_dir = |p: &str| -> Result<PathBuf> {
        fs::create_dir_all(p).map_err(|e| Error::io_at(p, e))?;
        fs::canonicalize(PathBuf::from(p)).map_err(|e| Error::io_at(p, e))
    };

    let output_common = resolve_dir(&output_common)?;
    let output_models = args.output_models.as_deref().map(resolve_dir).transpose()?;
    let models_subdir = args
        .models_subdir
        .clone()
        .unwrap_or_else(|| "_gen".to_string());
    let output_server = args.output_server.as_deref().map(resolve_dir).transpose()?;
    let output_client = args.output_client.as_deref().map(resolve_dir).transpose()?;
    let output_python = args.output_python.as_deref().map(resolve_dir).transpose()?;
    let output_node = args.output_node.as_deref().map(resolve_dir).transpose()?;
    let output_node_ts = args
        .output_node_ts
        .as_deref()
        .map(resolve_dir)
        .transpose()?;
    let output_wasm = args.output_wasm.as_deref().map(resolve_dir).transpose()?;

    let python_typings_filename = args
        .python_typings_filename
        .clone()
        .unwrap_or_else(|| "client.pyi".to_string());

    let models_gen_dir: Option<String> = output_models
        .as_deref()
        .zip(models_out_abs.as_deref())
        .map(|(models_dir, prost_out)| {
            let subdir_path = models_dir.join(&models_subdir);
            relative_path(&subdir_path, prost_out)
        });

    let output = CodeGenOutput {
        common: output_common,
        models: output_models,
        models_subdir,
        server: output_server,
        client: output_client,
        python: output_python,
        node: output_node,
        node_ts: output_node_ts,
        wasm: output_wasm,
        python_typings_filename,
        generate_resource_clients: file_cfg.generate_resource_clients.unwrap_or(false),
    };

    let config = build_config(output, &args, &file_cfg, models_gen_dir)?;
    generate_code(&metadata, &config)?;

    Ok(())
}

/// Build a [`CodeGenConfig`] from parsed CLI arguments and config-file settings.
fn build_config(
    output: CodeGenOutput,
    args: &GenerateArgs,
    file_cfg: &FileGenerateConfig,
    models_gen_dir: Option<String>,
) -> Result<CodeGenConfig> {
    let has_bindings_output = output.python.is_some()
        || output.node.is_some()
        || output.node_ts.is_some()
        || output.wasm.is_some();

    // Several binding names become Rust identifiers (`format_ident!`) deep in the generators,
    // which panic on empty input. Validate up front and fail with an actionable error naming
    // the missing config key, rather than crashing mid-generation.
    let require = |value: Option<String>, key: &str, section: &str| -> Result<String> {
        match value {
            Some(v) if !v.trim().is_empty() => Ok(v),
            _ => Err(Error::other(format!(
                "language bindings output was requested but `{section}.{key}` is not set; \
                 binding names are required and cannot be empty"
            ))),
        }
    };

    let bindings = if has_bindings_output {
        let py = file_cfg.python.as_ref();
        let node = file_cfg.node.as_ref();
        let ts = file_cfg.typescript.as_ref();

        // The TS/JS binding names (aggregate client, client crate, error base
        // class) are shared by the NAPI TypeScript client and the WASM bindings —
        // both read them from the `typescript:` block. Require them when either is
        // requested.
        let (aggregate_client_name, client_crate_name, ts_error_base_class) =
            if output.node_ts.is_some() || output.wasm.is_some() {
                (
                    require(
                        ts.and_then(|c| c.aggregate_client_name.clone()),
                        "aggregate_client_name",
                        "typescript",
                    )?,
                    require(
                        ts.and_then(|c| c.client_crate_name.clone()),
                        "client_crate_name",
                        "typescript",
                    )?,
                    require(
                        ts.and_then(|c| c.error_base_class.clone()),
                        "error_base_class",
                        "typescript",
                    )?,
                )
            } else {
                Default::default()
            };

        let (py_error_type, py_result_type) = if output.python.is_some() {
            (
                require(
                    py.and_then(|c| c.error_type.clone()),
                    "error_type",
                    "python",
                )?,
                require(
                    py.and_then(|c| c.result_type.clone()),
                    "result_type",
                    "python",
                )?,
            )
        } else {
            Default::default()
        };

        let napi_error_ext_trait = if output.node.is_some() {
            require(
                node.and_then(|c| c.napi_error_ext_trait.clone()),
                "napi_error_ext_trait",
                "node",
            )?
        } else {
            String::new()
        };

        Some(BindingsConfig {
            aggregate_client_name,
            client_crate_name,
            py_error_type,
            py_result_type,
            napi_error_ext_trait,
            typings_package_filter: py.and_then(|c| c.typings_package_filter.clone()),
            ts_error_base_class,
            ts_error_code_prefix: ts
                .and_then(|c| c.error_code_prefix.clone())
                .unwrap_or_default(),
        })
    } else {
        None
    };

    let generate_resource_enum = file_cfg.generate_resource_enum.unwrap_or(false);
    let generate_store_integration = file_cfg.generate_store_integration.unwrap_or(false);
    let generate_object_conversions = file_cfg.generate_object_conversions.unwrap_or(false);

    let runtime = match file_cfg.runtime.as_deref() {
        None | Some("prost") => Runtime::Prost,
        Some("buffa") => Runtime::Buffa,
        Some(other) => {
            return Err(Error::other(format!(
                "unknown runtime `{other}` in trestle.yaml (expected `prost` or `buffa`)"
            )));
        }
    };

    // Resolve the HTTP transport for generated clients. `transport_type_path` (an explicit Rust
    // path) wins if set; otherwise the friendly `transport` alias selects a built-in. `wasm`
    // emits a browser-buildable client (no signing; the browser attaches the session).
    let transport_type_path = match (
        file_cfg.transport_type_path.as_deref(),
        file_cfg.transport.as_deref(),
    ) {
        (Some(path), _) => path.to_string(),
        (None, None | Some("cloud")) => olai_codegen::DEFAULT_TRANSPORT_TYPE_PATH.to_string(),
        (None, Some("wasm")) => "olai_http_wasm::WasmClient".to_string(),
        (None, Some(other)) => {
            return Err(Error::other(format!(
                "unknown transport `{other}` in trestle.yaml (expected `cloud` or `wasm`, \
                 or set `transport_type_path` to a custom path)"
            )));
        }
    };

    Ok(CodeGenConfig {
        context_type_path: args
            .context_type
            .clone()
            .unwrap_or_else(|| "crate::api::RequestContext".to_string()),
        result_type_path: args
            .result_type
            .clone()
            .unwrap_or_else(|| "crate::Result".to_string()),
        models_path_template: args.models_path_template.clone().unwrap_or_default(),
        models_path_crate_template: args.models_path_crate_template.clone().unwrap_or_default(),
        output,
        generate_resource_enum,
        generate_store_integration,
        error_type_path: file_cfg.error_type_path.clone(),
        generate_object_conversions,
        bindings,
        models_gen_dir,
        resource_store_crate_name: file_cfg
            .resource_store_crate_name
            .clone()
            .unwrap_or_else(|| "olai_store".to_string()),
        runtime,
        transport_type_path,
    })
}
