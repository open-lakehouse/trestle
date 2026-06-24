//! Configuration types for code generation.
//!
//! Split out of [`super`] so the parent module focuses on generation orchestration.
//! Re-exported from [`super`] (and `crate`) so the public API path is unchanged.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// Protobuf runtime that the *generated* code is expected to consume.
///
/// This selects the ABI that emitted clients, handlers, and builders are shaped for —
/// i.e. how generated code reads and writes the consuming project's model types. It does
/// **not** affect how `olai-codegen` itself parses descriptors.
///
/// - [`Runtime::Prost`] (default): models are [prost](https://docs.rs/prost)-generated —
///   open enums are bare `i32`, singular message fields are `Option<Box<T>>`.
/// - [`Runtime::Buffa`]: models are [buffa](https://github.com/anthropics/buffa)-generated —
///   open enums are `EnumValue<E>`, singular message fields are `MessageField<T>`, and the
///   runtime provides native serde JSON (no separate `pbjson` layer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Runtime {
    /// prost-generated models (the historical default).
    #[default]
    Prost,
    /// buffa-generated models.
    Buffa,
}

/// Validated model import path derived from a `{service}` template string.
///
/// Constructed once from [`CodeGenConfig`] template fields. `resolve` performs the
/// `{service}` substitution and parses the result as a [`syn::Path`], catching
/// malformed templates at construction time rather than at code-generation time.
#[derive(Debug, Clone)]
pub struct ModelsPath {
    template: String,
}

impl ModelsPath {
    /// Build a `ModelsPath` from a template string containing `{service}` and,
    /// optionally, `{version}`.
    ///
    /// Performs a test substitution at construction to validate the template.
    pub fn new(template: &str) -> Result<Self> {
        let test = template.replace("{service}", "test").replace("{version}", "v1");
        syn::parse_str::<syn::Path>(&test).map_err(|e| Error::InvalidModelsPathTemplate {
            template: template.to_string(),
            source: e,
        })?;
        Ok(Self::from_template(template))
    }

    /// Wrap a template without the construction-time validation done by [`ModelsPath::new`].
    ///
    /// For callers that already validated the template (e.g. `generate_code` validates both
    /// templates up front, so per-service handlers needn't re-parse on every `models_path()`
    /// call). `resolve`/`try_resolve` still parse the final substituted path.
    pub(crate) fn from_template(template: &str) -> Self {
        Self {
            template: template.to_string(),
        }
    }

    /// Replace `{service}` with `service` and return the parsed [`syn::Path`], erroring if the
    /// substitution does not parse.
    ///
    /// `new` only proves the template parses for the literal `test`; a real service segment that
    /// isn't a valid path component (leading digit, hyphen, reserved word) can still fail here.
    /// Call this once per service up front (see [`ModelsPath::validate_for`]) so failures surface
    /// as a clean [`Error::InvalidModelsPathTemplate`] before generation rather than panicking.
    pub fn try_resolve(&self, service: &str, version: &str) -> Result<syn::Path> {
        let path = self.substitute(service, version);
        syn::parse_str(&path).map_err(|source| Error::InvalidModelsPathTemplate {
            template: path,
            source,
        })
    }

    /// Validate that `service` substitutes into a parseable path. Used up front, per service.
    ///
    /// Validates against the conventional `v1` version; the actual version segment is a plain
    /// path component derived from the proto package, so it cannot make a `{service}`-valid
    /// template fail to parse.
    pub fn validate_for(&self, service: &str) -> Result<()> {
        self.try_resolve(service, "v1").map(|_| ())
    }

    /// Replace `{service}`/`{version}` and return the parsed [`syn::Path`].
    ///
    /// # Panics
    ///
    /// Panics if the substituted path doesn't parse. Generation validates every service segment
    /// up front via [`ModelsPath::validate_for`], so this is unreachable on the generation path;
    /// prefer [`ModelsPath::try_resolve`] anywhere that hasn't already validated.
    pub fn resolve(&self, service: &str, version: &str) -> syn::Path {
        let path = self.substitute(service, version);
        syn::parse_str(&path)
            .unwrap_or_else(|e| panic!("Invalid models path `{path}` after substitution: {e}"))
    }

    /// Substitute the `{service}` and `{version}` placeholders. A template without `{version}`
    /// is left as-is for that placeholder, preserving any literal version it already encodes.
    fn substitute(&self, service: &str, version: &str) -> String {
        self.template
            .replace("{service}", service)
            .replace("{version}", version)
    }
}

/// Configuration for language-binding code generation (Python / Node.js).
#[derive(Debug, Clone)]
pub struct BindingsConfig {
    /// Name of the aggregate client struct (e.g. `"MyServiceClient"`).
    pub aggregate_client_name: String,
    /// Rust crate name used in `use` statements for the client crate
    /// (e.g. `"my_service_client"`).
    ///
    /// This must be set explicitly because the crate name may not match the
    /// snake_case form of `aggregate_client_name` (e.g. `"MyServiceClient"`
    /// snake_cases to `"my_service_client"`).
    pub client_crate_name: String,
    /// Fully-qualified Python error type (e.g. `"PyMyServiceError"`).
    pub py_error_type: String,
    /// Fully-qualified Python result alias (e.g. `"PyMyServiceResult"`).
    pub py_result_type: String,
    /// Name of the NAPI error extension trait (e.g. `"NapiErrorExt"`).
    pub napi_error_ext_trait: String,
    /// Optional substring filter for the Python typings package.
    ///
    /// When `Some(s)`, only messages/enums whose fully-qualified name contains
    /// `s` are included in the generated `.pyi` file.  When `None`, all
    /// reachable types are included.
    pub typings_package_filter: Option<String>,
    /// Base class name for TypeScript errors (e.g. `"MyServiceError"`).
    pub ts_error_base_class: String,
    /// Prefix used in native NAPI error messages (e.g. `"UC"`).
    pub ts_error_code_prefix: String,
}

/// Configuration for code generation, including import paths and output directories.
///
/// Construct this struct directly and set the fields you need.
#[derive(Debug, Clone)]
pub struct CodeGenConfig {
    /// Fully-qualified path to the request context type used in handler methods.
    ///
    /// Default: `"crate::api::RequestContext"`
    pub context_type_path: String,

    /// Fully-qualified path to the `Result` alias used in generated handler and client code.
    ///
    /// Default: `"crate::Result"`
    pub result_type_path: String,

    /// Template for the external model import path. `{service}` is replaced with the service's
    /// base path (e.g. `"catalogs"`).
    ///
    /// Example: `"my_common::models::{service}::v1"`
    pub models_path_template: String,

    /// Template for crate-local model import path. `{service}` is replaced with the service's
    /// base path.
    ///
    /// Default: `"crate::models::{service}::v1"`
    pub models_path_crate_template: String,

    /// Output directory configuration.
    pub output: CodeGenOutput,

    /// When `true`, generate `labels.rs` with `Resource` / `ObjectLabel` enums derived
    /// from `google.api.resource` annotations. Requires `output.models` to be `Some`.
    ///
    /// Store-specific output (`Label` impl, `RESOURCE_DESCRIPTORS`) is only emitted when
    /// `generate_store_integration` is also `true`.
    pub generate_resource_enum: bool,

    /// When `true` (and `generate_resource_enum` is set), emit the `olai_store` integration
    /// code in `labels.rs`:
    /// - `impl olai_store::Label for ObjectLabel`
    /// - `pub static RESOURCE_DESCRIPTORS: &[olai_store::ResourceTypeDescriptor<ObjectLabel>]`
    ///
    /// Set to `false` for crates that use the enums without a store dependency.
    pub generate_store_integration: bool,

    /// Fully-qualified path to the `Error` type used in generated `TryFrom<Resource>` impls.
    ///
    /// E.g. `"crate::Error"`. When `None`, `TryFrom` impls are not generated.
    pub error_type_path: Option<String>,

    /// When `true` and `generate_resource_enum` is set, emit `TryFrom<Object>`/`TryFrom<T>`
    /// and `ResourceExt` impl blocks in `labels.rs` for all resource types that have an
    /// `IDENTIFIER`-annotated field, plus a `qualified_name()` inherent method on each
    /// resource type.
    pub generate_object_conversions: bool,

    /// Configuration for language-binding generation. Required when `output.python`,
    /// `output.node`, or `output.node_ts` is `Some`.
    pub bindings: Option<BindingsConfig>,

    /// Relative path of the prost-generated `gen/` dir from the models subdirectory.
    /// Required when `output.models` is `Some`. E.g. `"../gen"`.
    pub models_gen_dir: Option<String>,

    /// Crate name for the resource store types used in generated `RESOURCE_DESCRIPTORS`.
    ///
    /// Default: `"olai_store"`
    pub resource_store_crate_name: String,

    /// Protobuf runtime the generated code is shaped for. See [`Runtime`].
    ///
    /// Defaults to [`Runtime::Prost`] for backward compatibility.
    pub runtime: Runtime,

    /// Fully-qualified path to the HTTP transport type that generated clients store and call.
    ///
    /// The transport must expose the surface generated client bodies use: per-verb builder
    /// methods (`get`/`post`/`patch`/`delete`/`put`) returning a request builder with
    /// `.json(..)`/`.query(..)`/`.send()`, whose response has `.status()`/`.bytes()`.
    /// [`olai_http::CloudClient`](https://docs.rs/olai-http) satisfies this.
    ///
    /// Defaults to `"olai_http::CloudClient"`, so native output is unchanged. A WASM/browser
    /// client points this at a lightweight reqwest-Fetch transport that has no `ring`/`tokio`
    /// dependency and lets the browser attach the session.
    pub transport_type_path: String,
}

/// Default transport type path: the cloud client used by native (non-WASM) generated code.
pub const DEFAULT_TRANSPORT_TYPE_PATH: &str = "olai_http::CloudClient";

impl CodeGenConfig {
    /// Whether generated clients use the default cloud transport (`olai_http::CloudClient`).
    ///
    /// Controls emission of cloud-specific aggregate constructors (`new_unauthenticated`,
    /// `new_with_token`), which only make sense for `CloudClient`. A custom transport (e.g. the
    /// WASM/browser one) gets only the generic `new(transport, base_url)`.
    pub fn uses_default_transport(&self) -> bool {
        self.transport_type_path == DEFAULT_TRANSPORT_TYPE_PATH
    }

    /// Whether the generated client should carry **both** transports, selected at
    /// compile time by `cfg(target_arch = "wasm32")` — `CloudClient` for native
    /// targets, `olai_http_wasm::WasmClient` for the browser.
    ///
    /// Enabled when WASM bindings output is requested (`output.wasm`): a project
    /// that wants a browser client needs the same client crate to build both ways
    /// (native for server-side/tests, wasm32 for the frontend). Native-only
    /// projects leave `output.wasm` unset and keep the single
    /// [`transport_type_path`](Self::transport_type_path) transport with no WASM
    /// dependency.
    pub fn dual_transport(&self) -> bool {
        self.output.wasm.is_some()
    }

    /// Validate this config without running code generation.
    ///
    /// Checks that:
    /// - `models_path_template` and `models_path_crate_template` produce valid Rust paths after
    ///   `{service}` substitution.
    /// - `bindings` is `Some` whenever `output.python`, `output.node`, `output.node_ts`, or
    ///   `output.wasm` is `Some`.
    ///
    /// Call this at construction time to surface misconfiguration early, before generation runs.
    pub fn validate(&self) -> Result<()> {
        ModelsPath::new(&self.models_path_template)?;
        ModelsPath::new(&self.models_path_crate_template)?;
        if (self.output.python.is_some()
            || self.output.node.is_some()
            || self.output.node_ts.is_some()
            || self.output.wasm.is_some())
            && self.bindings.is_none()
        {
            return Err(Error::MissingBindingsConfig);
        }
        Ok(())
    }
}

/// Output directory configuration for code generation.
///
/// Only `common` is required. All other outputs are optional — set to `None` to skip that
/// output entirely. For example, a server-only crate can omit `client`, and a client-only
/// crate can omit `server`.
#[derive(Debug, Clone)]
pub struct CodeGenOutput {
    /// Output directory for common (shared extractor) code.
    pub common: PathBuf,
    /// Parent models directory (e.g. `crates/common/src/models`).
    ///
    /// When `Some`, the generator writes both `labels.rs` and `mod.rs` into a
    /// subdirectory named [`models_subdir`](CodeGenOutput::models_subdir) inside this path.
    /// The prost-generated `gen/` directory is expected to be a sibling of that subdirectory.
    pub models: Option<PathBuf>,
    /// Name of the generated subdirectory inside [`models`](CodeGenOutput::models).
    ///
    /// Defaults to `"_gen"`.
    pub models_subdir: String,
    /// Output directory for server-side handler and route code. Generation is skipped when `None`.
    pub server: Option<PathBuf>,
    /// Output directory for HTTP client code. Generation is skipped when `None`.
    pub client: Option<PathBuf>,
    /// Whether to also emit ergonomic resource-scoped clients (`<service>/resource.rs`) alongside
    /// the low-level per-service client and builders.
    ///
    /// A scoped client (e.g. `CatalogClient` bound to a name) captures the resource's name
    /// components and exposes `get`/`update`/`delete` (+ resource-targeted custom RPCs) returning
    /// the matching builder. Only emitted for resource-scoped services, and only when
    /// [`client`](CodeGenOutput::client) is `Some`. Defaults to `false`.
    pub generate_resource_clients: bool,
    /// Output directory for Python bindings. Generation is skipped when `None`.
    pub python: Option<PathBuf>,
    /// Output directory for Node.js NAPI bindings. Generation is skipped when `None`.
    pub node: Option<PathBuf>,
    /// Output directory for Node.js TypeScript client. Generation is skipped when `None`.
    pub node_ts: Option<PathBuf>,
    /// Output directory for WASM/browser `#[wasm_bindgen]` bindings + `.d.ts`. Generation is
    /// skipped when `None`.
    ///
    /// Emits a `#[wasm_bindgen]` wrapper layer over the generated (WASM-transport) clients plus a
    /// `client.d.ts` for JS/TS consumers. Request/response values cross the boundary as plain JS
    /// objects via `serde-wasm-bindgen`, so this pairs with `runtime: Buffa` (serde-native models)
    /// and `transport_type_path = "olai_http_wasm::WasmClient"`.
    pub wasm: Option<PathBuf>,
    /// Filename for the generated Python typings stub.
    ///
    /// Default: `"client.pyi"`
    pub python_typings_filename: String,
}

impl CodeGenOutput {
    /// Absolute path of the generated subdirectory (`models/models_subdir`), if `models` is set.
    pub fn models_subdir_path(&self) -> Option<PathBuf> {
        self.models.as_ref().map(|m| m.join(&self.models_subdir))
    }
}
