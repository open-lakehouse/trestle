//! Configuration types for code generation.
//!
//! Split out of [`super`] so the parent module focuses on generation orchestration.
//! Re-exported from [`super`] (and `crate`) so the public API path is unchanged.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// Wire protocol a *generated* client speaks.
///
/// A single client layer. The set of layers to emit is [`ClientProtocols`]; a server that exposes
/// both REST and ConnectRPC for the same proto can emit both from one generation run.
///
/// - [`ClientProtocol::Rest`]: the HTTP/JSON low-level client ([`super::client`]) plus fluent
///   builders. Methods are routed from `google.api.http`; a method without an annotation has no
///   route and is skipped by the REST emitter.
/// - [`ClientProtocol::Connect`]: fluent builders over the **connect-rust** (BSR plugin) generated
///   `{Service}Client<T>`, dispatched via [`connectrpc`](https://docs.rs/connectrpc). No
///   `google.api.http` annotation is required — the whole request message is the body, and
///   required-vs-optional partitioning comes purely from `google.api.field_behavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientProtocol {
    /// HTTP/JSON client routed from `google.api.http` (the historical default).
    #[default]
    Rest,
    /// ConnectRPC client layered over the connect-rust generated service client.
    Connect,
}

impl ClientProtocol {
    /// The convention subdirectory name for this protocol's client when both are emitted side by
    /// side (`<service>/rest/`, `<service>/connect/`).
    pub(crate) fn subdir(self) -> &'static str {
        match self {
            ClientProtocol::Rest => "rest",
            ClientProtocol::Connect => "connect",
        }
    }
}

/// Which client protocol layer(s) to emit.
///
/// Analysis is protocol-agnostic (REST analysis is a superset of what ConnectRPC needs), so this is
/// purely an *emit-time* selection. Emitting both is supported: a server fronting the same proto
/// over REST and ConnectRPC can hand out either client.
///
/// File layout:
/// - exactly one protocol: emitted **flat** under `<service>/` (`<service>/client.rs`,
///   `<service>/builders.rs`) — unchanged from single-protocol output.
/// - both: each protocol gets its own convention subdir (`<service>/rest/`, `<service>/connect/`),
///   re-exported from `<service>/mod.rs`.
///
/// Defaults to REST only, so existing output is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientProtocols {
    /// Emit the REST (HTTP/JSON) client.
    pub rest: bool,
    /// Emit the ConnectRPC client.
    pub connect: bool,
}

impl Default for ClientProtocols {
    fn default() -> Self {
        Self {
            rest: true,
            connect: false,
        }
    }
}

impl ClientProtocols {
    /// Whether more than one protocol is selected (drives the subdir-vs-flat layout decision).
    pub(crate) fn is_multi(self) -> bool {
        self.rest && self.connect
    }

    /// The selected protocols, in deterministic order (REST before Connect).
    pub(crate) fn selected(self) -> Vec<ClientProtocol> {
        let mut v = Vec::new();
        if self.rest {
            v.push(ClientProtocol::Rest);
        }
        if self.connect {
            v.push(ClientProtocol::Connect);
        }
        v
    }
}

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
        let test = template
            .replace("{service}", "test")
            .replace("{version}", "v1");
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

    /// The template prefix before `{service}`, without the trailing `::` (`crate::models` for
    /// `crate::models::{service}::v1`). `None` when there is none.
    pub(crate) fn root(&self) -> Option<&str> {
        let (prefix, _) = self.template.split_once("{service}")?;
        let root = prefix.trim_end_matches(':');
        (!root.is_empty()).then_some(root)
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
    /// Base class name for TypeScript errors (e.g. `"MyServiceError"`).
    pub ts_error_base_class: String,
    /// Prefix used in native NAPI error messages (e.g. `"UC"`).
    pub ts_error_code_prefix: String,
    /// The Python API. Required with the `python` output; see [`PythonOptions`].
    pub python: Option<PythonOptions>,
}

/// Shape of the generated Python (PyO3) API.
///
/// The model classes are frozen, `eq`, with a Python-style `repr`, and only the models that a
/// request takes get a keyword constructor. Enums are `enum.StrEnum` classes (Python 3.11+) in
/// `<package>/_enums.py`, without the zero `*_UNSPECIFIED` member. The extension module is a
/// declarative `#[pymodule]` with the proto comments as docs, so PyO3's introspection
/// (`experimental-inspect`) writes the `.pyi`. The methods block on the Tokio runtime of the
/// generated `codegen/runtime.rs` (needs `tokio`), which the module also installs for the
/// `_async` methods. Client errors raise the PyO3 exception classes of the generated
/// `codegen/error.rs`, which needs [`RestClientOptions::error_module`].
#[derive(Debug, Clone)]
pub struct PythonOptions {
    /// The pure-Python package that gets `_enums.py` and the public modules.
    pub package: PyPackageConfig,
    /// Name of the base exception class (e.g. `MyServiceError`): the class with `message`,
    /// `code`, `status` and `details` that the subclasses per kind of HTTP error status extend.
    pub error_base_class: String,
    /// The extension module (e.g. `my_pkg._core`): the generated `#[pymodule]`. Classes of a
    /// package `a.<service>.v1` go in the submodule `<module>.<service>`, the root client and
    /// the exceptions in `<module>`.
    pub module: String,
    /// Every client method also gets a `<name>_async` twin returning an `asyncio.Future`. Needs
    /// `pyo3-async-runtimes` with `tokio-runtime`.
    pub async_methods: bool,
    /// Optional parameters are keyword-only; constructors take `REQUIRED` fields positionally.
    pub keyword_only: bool,
    /// Write `<package>/<service>.py` per submodule of [`module`](Self::module), re-exporting
    /// its classes and enums.
    pub public_modules: bool,
    /// The `User-Agent` product of the Python client (`<product>/<binding crate version>`).
    /// Needs the `reqwest` transport.
    pub user_agent: Option<String>,
}

/// Where the generated pure-Python files go.
#[derive(Debug, Clone)]
pub struct PyPackageConfig {
    /// Directory of the Python package (e.g. `python/my_pkg`).
    pub dir: PathBuf,
    /// Import name of the package (e.g. `my_pkg`).
    pub name: String,
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
    /// The clients use the `Result` of the generated error module instead when
    /// [`RestClientOptions::error_module`] is set.
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

    /// Force dual-transport client emission (the `cfg`-gated `CloudClient`/`WasmClient`
    /// `Transport` alias) even when no WASM `#[wasm_bindgen]` bindings output is
    /// configured. See [`dual_transport`](Self::dual_transport).
    ///
    /// Defaults to `false`. Setting `output.wasm` also turns dual-transport on;
    /// this flag exists so a project can build the same client crate for native
    /// and `wasm32` *without* pulling in the JS binding layer.
    pub dual_transport: bool,

    /// Which client protocol layer(s) to emit. See [`ClientProtocols`].
    ///
    /// Defaults to REST only, so existing HTTP/JSON output is unchanged.
    pub client_protocols: ClientProtocols,

    /// Import path of the **connect-rust** (BSR plugin) generated client module, used only when
    /// [`client_protocols`](Self::client_protocols) includes Connect.
    ///
    /// The generated Connect builder layer references the connect-rust client type as
    /// `<connect_client_path>::{Service}Client`. The path should resolve to the module that
    /// holds the per-service `{Service}Client<T>` structs (e.g.
    /// `"headwaters_proto::connect_gen::headwaters::read::v1"`).
    ///
    /// Required (non-`None`) whenever Connect is selected and client output is enabled; ignored for
    /// REST-only output.
    pub connect_client_path: Option<String>,

    /// Wire-format and error options of the REST client. See [`RestClientOptions`].
    pub rest_client: RestClientOptions,
}

/// Wire-format and error options of the generated REST client. Every option is off by default.
#[derive(Debug, Clone, Default)]
pub struct RestClientOptions {
    /// Write 64-bit integers as JSON numbers instead of proto3-JSON strings, and whole numbers
    /// of `google.protobuf.Value`/`Struct`/`ListValue` as integers (`1`, not `1.0`). Emits a
    /// `json_int64` module into the models `mod.rs` that the `trestle` CLI points buffa's helpers
    /// at (see [`CodeGenConfig::json_int64_module_path`]). Reading accepts both forms. Buffa only.
    pub json_int64_as_number: bool,

    /// Parse responses with buffa's `ignore_unknown_enum_values`: an unknown enum name becomes
    /// the zero value instead of failing the response. Buffa only.
    pub ignore_unknown_enum_values: bool,

    /// Generate the client error module (`error.rs`); the clients then return its `Result`
    /// instead of [`CodeGenConfig::result_type_path`]. When `None`, the crate supplies
    /// `crate::error::parse_error_response`.
    pub error_module: Option<ErrorModuleConfig>,

    /// RPCs (`package.Service.Method`) whose request body is sent form-encoded.
    pub form_encoded_methods: Vec<String>,

    /// RPCs (`package.Service.Method`) whose request body keeps the path fields, for APIs that
    /// want them in both places.
    pub path_fields_in_body_methods: Vec<String>,

    /// Proto packages whose query keys are the JSON (lowerCamelCase) names. Elsewhere a query
    /// key is the proto name, or the field's explicit `json_name`.
    pub json_query_packages: Vec<String>,

    /// The `User-Agent` product of the generated `reqwest` transport
    /// (`<product>/<client crate version>`); not sent in the browser. Needs that transport.
    pub user_agent: Option<String>,
}

/// Shape of the JSON error body that the generated error module reads, such as
/// `{"error_code": "NOT_FOUND", "message": "…", "details": […]}`. A body that is not such an
/// object becomes the message.
#[derive(Debug, Clone)]
pub struct ErrorModuleConfig {
    /// Key of the error code; its lowerCamelCase form is also read. Default `error_code`.
    pub code_field: String,
    /// Key of the message. Default `message`.
    pub message_field: String,
    /// Key of the details array. Default `details`.
    pub details_field: Option<String>,
    /// Fully-qualified proto enum (e.g. `.my.errors.v1.ErrorCode`) of the error codes. The
    /// error exposes the code as that enum; an unknown code stays a string.
    pub code_enum: Option<String>,
    /// More keys of the code, tried in order (e.g. OAuth's `error`).
    pub more_code_fields: Vec<String>,
    /// More keys of the message, tried in order (e.g. OAuth's `error_description`).
    pub more_message_fields: Vec<String>,
    /// Keys under which some answers nest the error object (`{"error": {"message": …}}`).
    pub envelope_fields: Vec<String>,
}

impl Default for ErrorModuleConfig {
    fn default() -> Self {
        Self {
            code_field: "error_code".into(),
            message_field: "message".into(),
            details_field: Some("details".into()),
            code_enum: None,
            more_code_fields: Vec::new(),
            more_message_fields: Vec::new(),
            envelope_fields: Vec::new(),
        }
    }
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

    /// Whether generated clients use the generated `reqwest` transport
    /// ([`REQWEST_TRANSPORT_TYPE_PATH`](super::REQWEST_TRANSPORT_TYPE_PATH)).
    pub fn uses_reqwest_transport(&self) -> bool {
        !self.dual_transport() && self.transport_type_path == super::REQWEST_TRANSPORT_TYPE_PATH
    }

    /// Whether the generated client also builds for `wasm32`, where the transport's futures are
    /// not `Send`, so the builders box them as `Local*` there.
    pub fn builds_for_wasm(&self) -> bool {
        self.dual_transport() || self.uses_reqwest_transport()
    }

    /// Whether the generated client should carry **both** transports, selected at
    /// compile time by `cfg(target_arch = "wasm32")` — `CloudClient` for native
    /// targets, `olai_http_wasm::WasmClient` for the browser.
    ///
    /// Enabled when WASM bindings output is requested (`output.wasm`), or when
    /// [`dual_transport`](Self::dual_transport) is set explicitly: a project that
    /// wants a browser client needs the same client crate to build both ways
    /// (native for server-side/tests, wasm32 for the frontend). Native-only
    /// projects leave both unset and keep the single
    /// [`transport_type_path`](Self::transport_type_path) transport with no WASM
    /// dependency.
    ///
    /// The explicit flag decouples the transport alias from the JS binding layer:
    /// `output.wasm` additionally emits `#[wasm_bindgen]` bindings, whereas
    /// `dual_transport = true` alone emits only the `cfg`-gated `Transport` alias.
    pub fn dual_transport(&self) -> bool {
        self.output.wasm.is_some() || self.dual_transport
    }

    /// The crate-local path of the `json_int64` module (e.g. `crate::models::json_int64`), for
    /// [`ModelRewrites::int64_module`](crate::postprocess::ModelRewrites::int64_module). `None`
    /// unless [`RestClientOptions::json_int64_as_number`] is set.
    pub fn json_int64_module_path(&self) -> Option<String> {
        if !self.rest_client.json_int64_as_number {
            return None;
        }
        let models_path = ModelsPath::from_template(&self.models_path_crate_template);
        Some(format!("{}::json_int64", models_path.root()?))
    }

    /// The crate-local path of the `json_value` module (e.g. `crate::models::json_value`), for
    /// [`ModelRewrites::json_value_module`](crate::postprocess::ModelRewrites::json_value_module).
    /// `None` unless [`RestClientOptions::json_int64_as_number`] is set.
    pub fn json_value_module_path(&self) -> Option<String> {
        if !self.rest_client.json_int64_as_number {
            return None;
        }
        let models_path = ModelsPath::from_template(&self.models_path_crate_template);
        Some(format!("{}::json_value", models_path.root()?))
    }

    /// The `Result` type of the generated clients: the generated error module's with
    /// [`RestClientOptions::error_module`], else [`result_type_path`](Self::result_type_path).
    pub(crate) fn client_result_path(&self) -> syn::Path {
        let path = match self.rest_client.error_module {
            Some(_) => format!("{}::Result", super::error_module::CLIENT_ERROR_MODULE),
            None => self.result_type_path.clone(),
        };
        syn::parse_str(&path).expect("valid result_type_path")
    }

    /// The module of `parse_error_response`: the generated error module with
    /// [`RestClientOptions::error_module`], else `crate::error`.
    pub(crate) fn client_error_module(&self) -> syn::Path {
        let path = match self.rest_client.error_module {
            Some(_) => super::error_module::CLIENT_ERROR_MODULE,
            None => "crate::error",
        };
        syn::parse_str(path).expect("valid module path")
    }

    /// The Python options of the bindings config. Only for the `python` output, which
    /// [`validate_options`](Self::validate_options) checks has them.
    pub(crate) fn python_options(&self) -> &PythonOptions {
        self.bindings
            .as_ref()
            .and_then(|b| b.python.as_ref())
            .expect("validated: the python output has `BindingsConfig::python`")
    }

    /// Check the options that need other options.
    pub(crate) fn validate_options(&self) -> Result<()> {
        let needs = |what: &str, requirement: &str| {
            Err(Error::Build(format!("`{what}` needs {requirement}")))
        };
        if self.output.server.is_some() && self.output.common.is_none() {
            return needs(
                "CodeGenOutput::server",
                "`CodeGenOutput::common` (the Axum extractors)",
            );
        }
        if self.rest_client.user_agent.is_some() && !self.uses_reqwest_transport() {
            return needs(
                "RestClientOptions::user_agent",
                "the generated `reqwest` transport",
            );
        }
        if self.output.python.is_some() {
            let Some(python) = self.bindings.as_ref().and_then(|b| b.python.as_ref()) else {
                return needs("CodeGenOutput::python", "`BindingsConfig::python`");
            };
            if self.rest_client.error_module.is_none() {
                return needs(
                    "CodeGenOutput::python",
                    "`RestClientOptions::error_module` (the Python exception classes raise its \
                     errors)",
                );
            }
            if python.user_agent.is_some() && !self.uses_reqwest_transport() {
                return needs(
                    "PythonOptions::user_agent",
                    "the generated `reqwest` transport",
                );
            }
        }
        if self.rest_client.json_int64_as_number
            && ModelsPath::from_template(&self.models_path_crate_template)
                .root()
                .is_none()
        {
            return needs(
                "RestClientOptions::json_int64_as_number",
                "a `models_path_crate_template` of the form `<models root>::{service}::...`",
            );
        }
        Ok(())
    }

    /// Validate this config without running code generation.
    ///
    /// Checks that:
    /// - `models_path_template` and `models_path_crate_template` produce valid Rust paths after
    ///   `{service}` substitution.
    /// - `bindings` is `Some` whenever `output.python`, `output.node`, `output.node_ts`, or
    ///   `output.wasm` is `Some`; `output.python` also needs `BindingsConfig::python` and
    ///   `RestClientOptions::error_module`.
    /// - the REST client and Python options that need other options have them.
    ///
    /// Call this at construction time to surface misconfiguration early, before generation runs.
    pub fn validate(&self) -> Result<()> {
        ModelsPath::new(&self.models_path_template)?;
        ModelsPath::new(&self.models_path_crate_template)?;
        let wants_bindings = self.output.python.is_some()
            || self.output.node.is_some()
            || self.output.node_ts.is_some()
            || self.output.wasm.is_some();
        if wants_bindings && self.bindings.is_none() {
            return Err(Error::MissingBindingsConfig);
        }
        // The Connect client layers over the connect-rust generated client, whose import path the
        // emitter needs. Only required when client output is actually requested.
        if self.client_protocols.connect
            && self.output.client.is_some()
            && self.connect_client_path.is_none()
        {
            return Err(Error::MissingConnectClientPath);
        }
        // Language bindings call into the REST aggregate client, so they need REST enabled.
        if wants_bindings && !self.client_protocols.rest {
            return Err(Error::BindingsRequireRest);
        }
        self.validate_options()
    }
}

/// Output directory configuration for code generation.
///
/// All outputs are optional — set to `None` to skip that output entirely. For example, a
/// server-only crate can omit `client`, and a client-only crate can omit `server` and
/// `common`.
#[derive(Debug, Clone)]
pub struct CodeGenOutput {
    /// Output directory for the Axum request extractors. Generation is skipped when `None`; the
    /// `server` output needs it.
    pub common: Option<PathBuf>,
    /// Parent models directory (e.g. `crates/common/src/models`).
    ///
    /// When `Some`, the generator writes `labels.rs`, `pyo3_impls.rs`, and `mod.rs`
    /// into a subdirectory named [`models_subdir`](CodeGenOutput::models_subdir)
    /// inside this path. The buf proto plugin co-locates the compiled model files
    /// (`<pkg>.rs` / `.serde.rs` / `.tonic.rs`) in that same subdirectory, so the
    /// `mod.rs` includes them with `include!("./<pkg>.rs")`.
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
}

impl CodeGenOutput {
    /// Absolute path of the generated subdirectory (`models/models_subdir`), if `models` is set.
    pub fn models_subdir_path(&self) -> Option<PathBuf> {
        self.models.as_ref().map(|m| m.join(&self.models_subdir))
    }
}
