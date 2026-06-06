//! TypeScript client generation for idiomatic Node.js API.
//!
//! Generates a `client.ts` that wraps NAPI-RS native bindings with typed
//! protobuf decoding via `@bufbuild/protobuf`.

use convert_case::{Case, Casing};
use itertools::Itertools;

use super::super::python::derive_resource_accessor_params;
use crate::analysis::{RequestParam, RequestType};
use crate::codegen::{BindingMode, BindingsConfig, MethodHandler, ServiceHandler};
use crate::google::api::http_rule::Pattern;
use crate::parsing::types::{BaseType, unified_to_typescript};

/// Format optional documentation as a JSDoc comment block.
fn format_jsdoc(documentation: Option<&str>, indent: &str) -> String {
    let Some(doc) = documentation else {
        return String::new();
    };
    let trimmed = doc.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = trimmed
        .lines()
        .map(|l| format!("{}   * {}", indent, l.trim()))
        .collect();
    format!("{}/**\n{}\n{}   */\n", indent, lines.join("\n"), indent)
}

fn is_napi_supported(param: &RequestParam) -> bool {
    is_napi_supported_type(&param.field_type().base_type)
}

/// Whether a param is a required, singular protobuf message — passed to the native binding as
/// serialized bytes via `toBinary(<Type>Schema, value)` and accepted as a typed object in the TS
/// method signature. Mirrors the NAPI-side `is_required_message_body`.
fn is_required_message_body(param: &RequestParam) -> bool {
    !param.is_optional()
        && !param.field_type().is_repeated
        && matches!(
            param.field_type().base_type,
            BaseType::Message(_) | BaseType::OneOf(_)
        )
}

/// The simple message type name of a param's type (e.g. `TagPolicy`), for `toBinary`/type rendering.
fn message_type_name(param: &RequestParam) -> Option<String> {
    match &param.field_type().base_type {
        BaseType::Message(n) | BaseType::OneOf(n) => {
            Some(crate::utils::extract_simple_type_name(n))
        }
        _ => None,
    }
}

fn is_napi_supported_type(base_type: &BaseType) -> bool {
    match base_type {
        BaseType::String
        | BaseType::Int32
        | BaseType::Int64
        | BaseType::Bool
        | BaseType::Float32
        | BaseType::Float64
        | BaseType::Bytes
        | BaseType::Unit
        | BaseType::Enum(_) => true,
        BaseType::Map(k, v) => {
            is_napi_supported_type(&k.base_type) && is_napi_supported_type(&v.base_type)
        }
        BaseType::Message(_) | BaseType::OneOf(_) => false,
    }
}

/// TypeScript error class definitions and `parseNativeError` helper.
fn generate_error_classes(bindings: &BindingsConfig) -> String {
    let base = &bindings.ts_error_base_class;
    let prefix = &bindings.ts_error_code_prefix;
    format!(
        r#"// ── {base} error hierarchy ────────────────────────────────────────────────────────

/** Base class for all {base} errors. */
export class {base} extends Error {{
  readonly errorCode: string;
  constructor(message: string, errorCode: string) {{
    super(message);
    this.name = "{base}";
    this.errorCode = errorCode;
  }}
}}

export class NotFoundError extends {base} {{
  constructor(message: string) {{
    super(message, "RESOURCE_NOT_FOUND");
    this.name = "NotFoundError";
  }}
}}

export class AlreadyExistsError extends {base} {{
  constructor(message: string) {{
    super(message, "RESOURCE_ALREADY_EXISTS");
    this.name = "AlreadyExistsError";
  }}
}}

export class PermissionDeniedError extends {base} {{
  constructor(message: string) {{
    super(message, "PERMISSION_DENIED");
    this.name = "PermissionDeniedError";
  }}
}}

export class UnauthenticatedError extends {base} {{
  constructor(message: string) {{
    super(message, "UNAUTHENTICATED");
    this.name = "UnauthenticatedError";
  }}
}}

export class InvalidParameterError extends {base} {{
  constructor(message: string) {{
    super(message, "INVALID_PARAMETER_VALUE");
    this.name = "InvalidParameterError";
  }}
}}

export class RequestLimitError extends {base} {{
  constructor(message: string) {{
    super(message, "REQUEST_LIMIT_EXCEEDED");
    this.name = "RequestLimitError";
  }}
}}

export class InternalServerError extends {base} {{
  constructor(message: string) {{
    super(message, "INTERNAL_ERROR");
    this.name = "InternalServerError";
  }}
}}

export class ServiceUnavailableError extends {base} {{
  constructor(message: string) {{
    super(message, "TEMPORARILY_UNAVAILABLE");
    this.name = "ServiceUnavailableError";
  }}
}}

type ErrorConstructor = new (message: string) => {base};

const ERROR_MAP: Record<string, ErrorConstructor> = {{
  RESOURCE_NOT_FOUND: NotFoundError,
  RESOURCE_ALREADY_EXISTS: AlreadyExistsError,
  PERMISSION_DENIED: PermissionDeniedError,
  UNAUTHENTICATED: UnauthenticatedError,
  INVALID_PARAMETER_VALUE: InvalidParameterError,
  REQUEST_LIMIT_EXCEEDED: RequestLimitError,
  INTERNAL_ERROR: InternalServerError,
  TEMPORARILY_UNAVAILABLE: ServiceUnavailableError,
}};

/**
 * Parse a native NAPI error that may carry a `{prefix}:<CODE>:<message>` prefix
 * and re-throw as the appropriate typed subclass of `{base}`.
 */
function parseNativeError(e: unknown): never {{
  if (e instanceof Error) {{
    const match = e.message.match(/^{prefix}:([^:]+):([\s\S]*)$/);
    if (match) {{
      const [, code, message] = match;
      const Ctor = ERROR_MAP[code] ?? {base};
      throw new Ctor(message);
    }}
  }}
  throw e;
}}

// ── end {base} error hierarchy ─────────────────────────────────────────────────────

"#
    )
}

/// Generate the `models/index.ts` barrel that re-exports every service's generated protobuf-es
/// modules (both the message types in `models_pb` and the service request/response types in
/// `svc_pb`).
///
/// `client.ts` imports message and response types from `"./models"`, so this barrel must surface
/// both `*_pb` files. The path for each is derived from the service's proto package
/// (e.g. `unitycatalog.tags.v1` → `./gen/unitycatalog/tags/v1/{models_pb,svc_pb}`).
pub(crate) fn generate_models_barrel(services: &[ServiceHandler<'_>]) -> String {
    let mut paths: Vec<String> = services
        .iter()
        .map(|s| s.plan.package.replace('.', "/"))
        .sorted()
        .dedup()
        .flat_map(|pkg| {
            [
                format!("export * from \"./gen/{pkg}/models_pb\";"),
                format!("export * from \"./gen/{pkg}/svc_pb\";"),
            ]
        })
        .collect();
    paths.push(String::new()); // trailing newline
    paths.join("\n")
}

/// Generate the complete `client.ts` file for all services.
pub(crate) fn generate_client_ts(services: &[ServiceHandler<'_>]) -> String {
    let bindings = services
        .first()
        .and_then(|s| s.config.bindings.as_ref())
        .expect("bindings config required for node_ts output");

    // Sort once up-front so all loops below produce stable output regardless of
    // the HashMap iteration order of `CodeGenMetadata::services`.
    let sorted: Vec<&ServiceHandler<'_>> = services
        .iter()
        .sorted_by_key(|s| &s.plan.service_name)
        .collect();

    let mut out = String::new();

    out.push_str(&generate_imports_sorted(&sorted));
    out.push('\n');
    out.push_str(&generate_error_classes(bindings));

    // Generate options interfaces for all services
    for service in &sorted {
        for method in service.methods() {
            if let Some(iface) = generate_options_interface(&method) {
                out.push_str(&iface);
                out.push('\n');
            }
        }
    }

    // Generate resource client classes
    for service in &sorted {
        if let Some(class) = generate_resource_client_class(service) {
            out.push_str(&class);
            out.push('\n');
        }
    }

    // Generate the main aggregate client class
    out.push_str(&generate_aggregate_client_sorted(&sorted, bindings));

    out
}

fn generate_imports_sorted(services: &[&ServiceHandler<'_>]) -> String {
    let bindings = services
        .first()
        .and_then(|s| s.config.bindings.as_ref())
        .expect("bindings config required for node_ts output");

    let napi_aggregate_name = format!("Napi{}", bindings.aggregate_client_name);

    let mut type_names: Vec<String> = Vec::new();
    let mut schema_names: Vec<String> = Vec::new();

    for service in services {
        if let Some(resource) = service.resource() {
            let type_name = resource
                .type_name
                .split('.')
                .next_back()
                .unwrap_or(&resource.type_name);
            if !type_names.contains(&type_name.to_string()) {
                type_names.push(type_name.to_string());
                schema_names.push(format!("{}Schema", type_name));
            }
        }

        for method in service.methods() {
            // Import exactly the type each method's generated code decodes (see
            // `decoded_type_name`): item type for standard list unwrapping, otherwise the full
            // output message. This keeps imports in lockstep with emitted `fromBinary` calls,
            // including resource-less (Flat) services that decode `*Response` messages directly.
            if let Some(name) = decoded_type_name(service, &method) {
                if !type_names.contains(&name) {
                    type_names.push(name.clone());
                    schema_names.push(format!("{}Schema", name));
                }
            }

            // Required message bodies are encoded with `toBinary(<Type>Schema, value)` before
            // crossing to the native binding, so import each body's type and schema too.
            for param in method.required_parameters() {
                if is_required_message_body(param) {
                    if let Some(name) = message_type_name(param) {
                        if !type_names.contains(&name) {
                            type_names.push(name.clone());
                            schema_names.push(format!("{}Schema", name));
                        }
                    }
                }
            }
        }
    }

    type_names.sort();
    type_names.dedup();
    schema_names.sort();
    schema_names.dedup();

    let mut native_classes: Vec<String> = vec![format!("{} as NativeClient", napi_aggregate_name)];
    for service in services {
        if service.resource().is_some() {
            let napi_name = format!("Napi{}", service.client_type());
            let native_alias = format!("Native{}", service.client_type());
            native_classes.push(format!("{} as {}", napi_name, native_alias));
        }
    }
    // native_classes already stable since services slice is pre-sorted; sort for safety
    native_classes.sort();

    let type_imports = type_names
        .iter()
        .map(|t| format!("  type {},", t))
        .collect::<Vec<_>>()
        .join("\n");

    let schema_imports = schema_names
        .iter()
        .map(|s| format!("  {},", s))
        .collect::<Vec<_>>()
        .join("\n");

    let native_imports = native_classes
        .iter()
        .map(|n| format!("  {},", n))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"import {{ fromBinary, toBinary }} from "@bufbuild/protobuf";
import {{
{type_imports}
{schema_imports}
}} from "./models";
import {{
{native_imports}
}} from "./native";
"#
    )
}

/// The model type name that a method's generated code decodes via `fromBinary`, if any.
///
/// Mirrors the per-request-type emit logic so the import set stays in lockstep with the emitted
/// `fromBinary(<Type>Schema, ...)` calls:
/// - standard `List` methods unwrap the repeated field → the item type (e.g. `Catalog`);
/// - every other returning method decodes its full output message (e.g. `TagAssignment`,
///   `ListTagAssignmentsResponse`);
/// - `Empty`-returning (void) methods decode nothing → `None`.
fn decoded_type_name(_service: &ServiceHandler<'_>, method: &MethodHandler<'_>) -> Option<String> {
    match &method.plan.request_type {
        RequestType::List => Some(
            method
                .list_output_field()?
                .unified_type
                .type_ident()
                .to_string(),
        ),
        _ => method.output_type().map(|t| t.to_string()),
    }
}

/// Generate an options interface for a method's optional parameters.
fn generate_options_interface(method: &MethodHandler<'_>) -> Option<String> {
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| !p.is_path_param() && is_napi_supported(p))
        .collect();

    if optional_params.is_empty() {
        return None;
    }

    let interface_name = format!("{}Options", method.plan.metadata.method_name);

    let mut fields = String::new();
    for param in &optional_params {
        let ts_name = param.name().to_case(Case::Camel);
        let ts_type = unified_to_typescript(param.field_type());
        // Strip the " | undefined" suffix since we use `?:` syntax
        let ts_type = ts_type.strip_suffix(" | undefined").unwrap_or(&ts_type);
        if let Some(doc) = param.documentation() {
            let cleaned = doc.trim().replace('\n', "\n   * ");
            fields.push_str(&format!("  /** {} */\n", cleaned));
        }
        fields.push_str(&format!("  {}?: {};\n", ts_name, ts_type));
    }

    Some(format!(
        "export interface {} {{\n{}}}\n",
        interface_name, fields
    ))
}

/// Generate a resource client class (e.g. CatalogClient, SchemaClient).
fn generate_resource_client_class(service: &ServiceHandler<'_>) -> Option<String> {
    let resource = service.resource()?;
    let type_name = resource
        .type_name
        .split('.')
        .next_back()
        .unwrap_or(&resource.type_name);
    let client_type = service.client_type().to_string();
    let native_type = format!("Native{}", client_type);

    let mut methods = String::new();

    for method in service.methods() {
        match &method.plan.request_type {
            RequestType::Get => {
                methods.push_str(&generate_resource_get_method(
                    &method,
                    type_name,
                    BindingMode::Scoped,
                ));
            }
            RequestType::Update => {
                methods.push_str(&generate_resource_update_method(
                    &method,
                    type_name,
                    BindingMode::Scoped,
                ));
            }
            RequestType::Delete => {
                methods.push_str(&generate_resource_delete_method(
                    &method,
                    BindingMode::Scoped,
                ));
            }
            _ => {}
        }
    }

    Some(format!(
        r#"export class {client_type} {{
  private readonly inner: {native_type};

  /** @internal */
  constructor(inner: {native_type}) {{
    this.inner = inner;
  }}

{methods}}}
"#
    ))
}

/// Whether the path-param filter should drop path params for this mode.
///
/// [`BindingMode::Scoped`] methods live on a resource-scoped client that already holds the path
/// params, so they are omitted from the signature. [`BindingMode::Flat`] methods live on the root
/// client and must accept and forward every param, including path params.
fn drops_path(mode: BindingMode) -> bool {
    mode == BindingMode::Scoped
}

/// The TS method name and the native (NAPI) call name for a get/update/delete-style method.
///
/// - [`BindingMode::Scoped`]: short verb (`get`/`update`/`delete`) on the scoped client.
/// - [`BindingMode::Flat`]: the full base method name (camelCased) on the root client, matching
///   the NAPI-exposed flat method (snake_case Rust → camelCase JS).
fn instance_method_names(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    verb: &str,
) -> (String, String) {
    match mode {
        BindingMode::Scoped => (verb.to_string(), verb.to_string()),
        BindingMode::Flat => {
            let name = method.binding_method_name_str().to_case(Case::Camel);
            (name.clone(), name)
        }
    }
}

fn generate_resource_get_method(
    method: &MethodHandler<'_>,
    type_name: &str,
    mode: BindingMode,
) -> String {
    generate_instance_returning_method(method, type_name, mode, "get")
}

fn generate_resource_update_method(
    method: &MethodHandler<'_>,
    type_name: &str,
    mode: BindingMode,
) -> String {
    generate_instance_returning_method(method, type_name, mode, "update")
}

/// Shared body for get/update-style methods: forward params and decode a typed response.
fn generate_instance_returning_method(
    method: &MethodHandler<'_>,
    type_name: &str,
    mode: BindingMode,
    verb: &str,
) -> String {
    let schema_name = format!("{}Schema", type_name);
    let jsdoc = format_jsdoc(method.plan.metadata.documentation.as_deref(), "  ");
    let drop_path = drops_path(mode);
    let (ts_name, native_name) = instance_method_names(method, mode, verb);

    let required_params: Vec<&RequestParam> = method
        .required_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param())
                && (is_napi_supported(p) || is_required_message_body(p))
        })
        .collect();
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| !(drop_path && p.is_path_param()) && is_napi_supported(p))
        .collect();

    let spec = MethodCallSpec::build(method, &required_params, &optional_params);
    let MethodCallSpec {
        full_param_list,
        optional_destructure,
        all_args,
    } = spec;

    format!(
        r#"{jsdoc}  async {ts_name}({full_param_list}): Promise<{type_name}> {{
{optional_destructure}    try {{
      return fromBinary({schema_name}, await this.inner.{native_name}({all_args}));
    }} catch (e) {{ throw parseNativeError(e); }}
  }}

"#
    )
}

fn generate_resource_delete_method(method: &MethodHandler<'_>, mode: BindingMode) -> String {
    let jsdoc = format_jsdoc(method.plan.metadata.documentation.as_deref(), "  ");
    let drop_path = drops_path(mode);
    let (ts_name, native_name) = instance_method_names(method, mode, "delete");

    let required_params: Vec<&RequestParam> = method
        .required_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param())
                && (is_napi_supported(p) || is_required_message_body(p))
        })
        .collect();
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| !(drop_path && p.is_path_param()) && is_napi_supported(p))
        .collect();

    let spec = MethodCallSpec::build(method, &required_params, &optional_params);
    let MethodCallSpec {
        full_param_list,
        optional_destructure,
        all_args,
    } = spec;

    format!(
        r#"{jsdoc}  async {ts_name}({full_param_list}): Promise<void> {{
{optional_destructure}    try {{
      await this.inner.{native_name}({all_args});
    }} catch (e) {{ throw parseNativeError(e); }}
  }}

"#
    )
}

/// Emit a single method for a resource-less (Flat) service onto the root client.
///
/// Every method of a resource-less service is emitted here, lowered [`BindingMode::Flat`] so the
/// signature includes all params (including path params) and forwards them to the native client
/// method of the same (camelCased) name. The emit shape is chosen by request type:
/// list/stream, create-with-return (or void), get/update-style returning, or delete (void).
fn generate_flat_method(service: &ServiceHandler<'_>, method: &MethodHandler<'_>) -> String {
    let mode = BindingMode::Flat;
    match &method.plan.request_type {
        RequestType::List => {
            let mut out = generate_collection_list_method(service, method, false);
            out.push_str(&generate_collection_list_stream_method(
                service, method, false,
            ));
            out
        }
        RequestType::Create | RequestType::Custom(Pattern::Post(_) | Pattern::Patch(_)) => {
            generate_collection_create_method(service, method, false)
        }
        RequestType::Delete => generate_resource_delete_method(method, mode),
        // Get / Update, and any remaining custom RPC on a resource-less service: forward all params
        // and decode the typed response. An `Empty`-returning RPC produces a `Promise<void>`.
        RequestType::Get | RequestType::Update | RequestType::Custom(_) => {
            match method.output_type() {
                Some(output_type) => {
                    generate_resource_get_method(method, &output_type.to_string(), mode)
                }
                None => generate_resource_delete_method(method, mode),
            }
        }
    }
}

/// Generate the main aggregate client class (e.g. `MyServiceClient`).
fn generate_aggregate_client_sorted(
    services: &[&ServiceHandler<'_>],
    bindings: &BindingsConfig,
) -> String {
    let aggregate_client_name = &bindings.aggregate_client_name;

    let mut methods = String::new();

    for service in services {
        match service.binding_mode() {
            // Resource-scoped services contribute only collection-style methods to the root client;
            // their instance methods live on the scoped client. Path params are dropped here.
            BindingMode::Scoped => {
                for method in service.methods() {
                    if !method.is_collection_method() {
                        continue;
                    }
                    match &method.plan.request_type {
                        RequestType::List => {
                            methods
                                .push_str(&generate_collection_list_method(service, &method, true));
                            methods.push_str(&generate_collection_list_stream_method(
                                service, &method, true,
                            ));
                        }
                        RequestType::Create => {
                            methods.push_str(&generate_collection_create_method(
                                service, &method, true,
                            ));
                        }
                        _ => {}
                    }
                }

                // Resource accessor methods (e.g. .catalog("name"), .schema("cat", "schema"))
                if let Some(accessor) = generate_resource_accessor(service) {
                    methods.push_str(&accessor);
                }
            }
            // Resource-less services contribute *every* method to the root client, lowered flat:
            // each passes all params (including path params) straight to the native client.
            BindingMode::Flat => {
                for method in service.methods() {
                    methods.push_str(&generate_flat_method(service, &method));
                }
            }
        }
    }

    format!(
        r#"export class {aggregate_client_name} {{
  private readonly inner: NativeClient;

  constructor(url: string, token?: string) {{
    this.inner = NativeClient.fromUrl(url, token);
  }}

{methods}}}
"#
    )
}

/// Computed parameters for generating a typed aggregate-client method.
///
/// All three collection method variants (list, create-with-return, create-void) share
/// the same parameter-building logic. `MethodCallSpec` captures that shared state once
/// so the rendering functions only differ in their return type and native call expression.
struct MethodCallSpec {
    full_param_list: String,
    optional_destructure: String,
    all_args: String,
}

impl MethodCallSpec {
    fn build(
        method: &MethodHandler<'_>,
        required_params: &[&RequestParam],
        optional_params: &[&RequestParam],
    ) -> Self {
        let options_type = format!("{}Options", method.plan.metadata.method_name);

        let required_param_list = required_params
            .iter()
            .map(|p| {
                // A required message body is accepted as its typed object (e.g.
                // `tagPolicy: TagPolicy`) and serialized to bytes when forwarded to the native
                // binding (see `all_args` below).
                let ty = match message_type_name(p) {
                    Some(name) if is_required_message_body(p) => name,
                    _ => unified_to_typescript(p.field_type()).replace(" | undefined", ""),
                };
                format!("{}: {}", p.name().to_case(Case::Camel), ty)
            })
            .collect::<Vec<_>>()
            .join(", ");

        let has_options = !optional_params.is_empty();

        let full_param_list = if has_options {
            if required_param_list.is_empty() {
                format!("options?: {}", options_type)
            } else {
                format!("{}, options?: {}", required_param_list, options_type)
            }
        } else {
            required_param_list.clone()
        };

        let optional_destructure = if has_options {
            let fields = optional_params
                .iter()
                .map(|p| p.name().to_case(Case::Camel))
                .collect::<Vec<_>>()
                .join(", ");
            format!("    const {{ {} }} = options || {{}};\n", fields)
        } else {
            String::new()
        };

        let mut args: Vec<String> = required_params
            .iter()
            .map(|p| {
                let name = p.name().to_case(Case::Camel);
                // Serialize required message bodies to protobuf bytes for the native binding.
                match message_type_name(p) {
                    // `toBinary` yields a `Uint8Array`; the native binding expects a Node `Buffer`.
                    Some(ty) if is_required_message_body(p) => {
                        format!("Buffer.from(toBinary({ty}Schema, {name}))")
                    }
                    _ => name,
                }
            })
            .collect();
        for p in optional_params {
            args.push(p.name().to_case(Case::Camel));
        }
        let all_args = args.join(", ");

        Self {
            full_param_list,
            optional_destructure,
            all_args,
        }
    }
}

fn generate_collection_list_method(
    _service: &ServiceHandler<'_>,
    method: &MethodHandler<'_>,
    drop_path: bool,
) -> String {
    let jsdoc = format_jsdoc(method.plan.metadata.documentation.as_deref(), "  ");
    let method_name = method.binding_method_name_str().to_case(Case::Camel);
    let items_field = match method.list_output_field() {
        Some(field) => field,
        None => return String::new(),
    };
    let item_type_name = items_field.unified_type.type_ident().to_string();
    let schema_name = format!("{}Schema", item_type_name);

    let required_params: Vec<&RequestParam> = method
        .required_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param())
                && (is_napi_supported(p) || is_required_message_body(p))
        })
        .collect();
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param()) && p.name() != "page_token" && is_napi_supported(p)
        })
        .collect();

    let spec = MethodCallSpec::build(method, &required_params, &optional_params);
    let MethodCallSpec {
        full_param_list,
        optional_destructure,
        all_args,
    } = spec;

    format!(
        r#"{jsdoc}  async {method_name}({full_param_list}): Promise<{item_type_name}[]> {{
{optional_destructure}    try {{
      return (await this.inner.{method_name}({all_args})).map((data) =>
        fromBinary({schema_name}, data),
      );
    }} catch (e) {{ throw parseNativeError(e); }}
  }}

"#
    )
}

/// Generate a streaming variant of a list method that yields items via `AsyncIterable<T>`.
///
/// The native Rust side returns a `ReadableStream<Buffer>` (napi-rs v3). On the TypeScript
/// side we wrap it in an async generator so callers can use `for await...of` directly.
/// Node.js 18+ (the minimum engine version) supports `for await...of` over `ReadableStream`.
fn generate_collection_list_stream_method(
    _service: &ServiceHandler<'_>,
    method: &MethodHandler<'_>,
    drop_path: bool,
) -> String {
    let jsdoc = format_jsdoc(method.plan.metadata.documentation.as_deref(), "  ");
    let base_method_name = method.binding_method_name_str().to_case(Case::Camel);
    let stream_method_name = format!("{}Stream", base_method_name);

    let items_field = match method.list_output_field() {
        Some(field) => field,
        None => return String::new(),
    };
    let item_type_name = items_field.unified_type.type_ident().to_string();
    let schema_name = format!("{}Schema", item_type_name);

    let required_params: Vec<&RequestParam> = method
        .required_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param())
                && (is_napi_supported(p) || is_required_message_body(p))
        })
        .collect();
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param()) && p.name() != "page_token" && is_napi_supported(p)
        })
        .collect();

    let spec = MethodCallSpec::build(method, &required_params, &optional_params);
    let MethodCallSpec {
        full_param_list,
        optional_destructure,
        all_args,
    } = spec;

    format!(
        r#"{jsdoc}  async *{stream_method_name}({full_param_list}): AsyncIterable<{item_type_name}> {{
{optional_destructure}    try {{
      for await (const data of this.inner.{base_method_name}Stream({all_args})) {{
        yield fromBinary({schema_name}, data);
      }}
    }} catch (e) {{ throw parseNativeError(e); }}
  }}

"#
    )
}

fn generate_collection_create_method(
    _service: &ServiceHandler<'_>,
    method: &MethodHandler<'_>,
    drop_path: bool,
) -> String {
    let output_type = match method.output_type() {
        Some(t) => t.to_string(),
        None => return generate_void_create_method(method, drop_path),
    };
    let schema_name = format!("{}Schema", output_type);

    let jsdoc = format_jsdoc(method.plan.metadata.documentation.as_deref(), "  ");
    let method_name = method.binding_method_name_str().to_case(Case::Camel);

    let required_params: Vec<&RequestParam> = method
        .required_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param())
                && (is_napi_supported(p) || is_required_message_body(p))
        })
        .collect();
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| !(drop_path && p.is_path_param()) && is_napi_supported(p))
        .collect();

    let spec = MethodCallSpec::build(method, &required_params, &optional_params);
    let MethodCallSpec {
        full_param_list,
        optional_destructure,
        all_args,
    } = spec;

    format!(
        r#"{jsdoc}  async {method_name}({full_param_list}): Promise<{output_type}> {{
{optional_destructure}    try {{
      return fromBinary({schema_name}, await this.inner.{method_name}({all_args}));
    }} catch (e) {{ throw parseNativeError(e); }}
  }}

"#
    )
}

fn generate_void_create_method(method: &MethodHandler<'_>, drop_path: bool) -> String {
    let jsdoc = format_jsdoc(method.plan.metadata.documentation.as_deref(), "  ");
    let method_name = method.binding_method_name_str().to_case(Case::Camel);

    let required_params: Vec<&RequestParam> = method
        .required_parameters()
        .filter(|p| {
            !(drop_path && p.is_path_param())
                && (is_napi_supported(p) || is_required_message_body(p))
        })
        .collect();
    let optional_params: Vec<&RequestParam> = method
        .optional_parameters()
        .filter(|p| !(drop_path && p.is_path_param()) && is_napi_supported(p))
        .collect();

    let spec = MethodCallSpec::build(method, &required_params, &optional_params);
    let MethodCallSpec {
        full_param_list,
        optional_destructure,
        all_args,
    } = spec;

    format!(
        r#"{jsdoc}  async {method_name}({full_param_list}): Promise<void> {{
{optional_destructure}    try {{
      await this.inner.{method_name}({all_args});
    }} catch (e) {{ throw parseNativeError(e); }}
  }}

"#
    )
}

fn generate_resource_accessor(service: &ServiceHandler<'_>) -> Option<String> {
    if service.plan.managed_resources.is_empty() {
        return None;
    }

    // managed_resources is non-empty (checked above), so resource() is always Some here.
    let resource = service.resource().unwrap();
    let method_name = resource.descriptor.singular.to_case(Case::Camel);
    let client_type = service.client_type().to_string();

    let params = derive_resource_accessor_params(service);

    let param_list = params
        .iter()
        .map(|p| format!("{}: string", p.to_case(Case::Camel)))
        .collect::<Vec<_>>()
        .join(", ");

    let arg_list = params
        .iter()
        .map(|p| p.to_case(Case::Camel))
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!(
        r#"  {method_name}({param_list}): {client_type} {{
    return new {client_type}(this.inner.{method_name}({arg_list}));
  }}

"#
    ))
}
