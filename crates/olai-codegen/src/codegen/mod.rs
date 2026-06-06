//! Code generation module for REST API handlers and language bindings.
//!
//! This module provides the core functionality for generating Rust code from
//! protobuf metadata extracted from service definitions.
//!
//! ## Architecture
//!
//! The code generation process follows these phases:
//! 1. **Analysis**: Process collected metadata to understand service structure
//! 2. **Planning**: Determine what code needs to be generated
//! 3. **Generation**: Create Rust code using templates and metadata
//! 4. **Output**: Write generated code to appropriate files
//!
//! ## Generated Code Types
//!
//! - **Handler Traits**: Async trait definitions for service operations
//! - **Request Extractors**: Axum FromRequest/FromRequestParts implementations
//! - **Route Handlers**: Axum handler functions that delegate to traits
//! - **Client Code**: HTTP client implementations for services
//! - **Type Mappings**: Conversions between protobuf and Rust types

use std::collections::{BTreeMap, BTreeSet, HashMap};

use convert_case::{Case, Casing};
use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::error::{Error, Result};

use crate::analysis::{
    BodyField, GenerationPlan, ManagedResource, MethodPlan, RequestParam, RequestType, ServicePlan,
    analyze_metadata, split_body_fields,
};
use crate::google::api::http_rule::Pattern;
use crate::output;
use crate::parsing::types::{self, RenderContext, UnifiedType};
use crate::parsing::{CodeGenMetadata, MessageField, MessageInfo};

mod aggregate;
mod builder;
mod client;
mod config;
mod handler;
pub(crate) mod node;
mod python;
mod resource_client;
mod resources;
mod server;
mod tokens;

pub use config::{BindingsConfig, CodeGenConfig, CodeGenOutput, ModelsPath};
pub(crate) use tokens::{doc_tokens, format_tokens, format_tokens_static};

/// How a language binding lowers a method's call into the underlying Rust client.
///
/// Resource services expose a scoped client (e.g. `CatalogClient`) that captures path params;
/// resource-less services have no such client, so their methods live on the root client and must
/// pass every param themselves (the "flat" shape, treated like collection methods).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingMode {
    /// Method lives on a resource-scoped client that already holds the path params.
    Scoped,
    /// Method lives on the root client and receives all params, including path params.
    Flat,
}

/// Language-agnostic description of a service's resource accessor method on the aggregate client.
///
/// Computed once from the IR (`ServicePlan::resource_accessor_params`) and rendered by each language
/// emitter, so the param list and nested-vs-flat decision are shared by construction.
pub(crate) struct AccessorSpec {
    /// The resource singular, used as the accessor method name (e.g. `catalog`).
    pub(crate) singular: String,
    /// The ordered accessor params (ancestors + `<singular>_name` leaf), e.g.
    /// `["catalog_name", "schema_name"]` or `["name"]`.
    pub(crate) params: Vec<String>,
    /// Whether the resource is nested (has parent components); its full name is the dot-joined
    /// params and the accessor delegates through a `<singular>_from_full_name` helper.
    pub(crate) nested: bool,
}

impl AccessorSpec {
    /// The `format!` template for joining the params into a composite full name
    /// (e.g. `"{}.{}"` for two params). Only meaningful when [`Self::nested`] is true.
    pub(crate) fn join_format(&self) -> String {
        std::iter::repeat_n("{}", self.params.len())
            .collect::<Vec<_>>()
            .join(".")
    }
}

impl MethodPlan {
    pub(crate) fn resource_client_method(&self) -> Ident {
        match &self.scoped_verb {
            Some(verb) => format_ident!("{}", verb),
            None => format_ident!("{}", self.handler_function_name),
        }
    }

    pub(crate) fn base_method_ident(&self) -> Ident {
        format_ident!("{}", self.handler_function_name)
    }
}

/// Generate all code described by `config` from `metadata`.
///
/// Writes the following outputs, depending on which [`CodeGenOutput`] fields are `Some`:
///
/// | Field | Contents |
/// |-------|----------|
/// | `output.common` | Axum extractor code, per-service `mod.rs` (always written) |
/// | `output.models_gen` | `labels.rs` resource-enum file (falls back to `common` if `None`) |
/// | `output.server` | Handler trait + Axum route wiring per service |
/// | `output.client` | HTTP client structs and request builders per service |
/// | `output.python` | PyO3 binding wrappers + `.pyi` typings stub |
/// | `output.node` | NAPI binding wrappers |
/// | `output.node_ts` | TypeScript client (`client.ts`) |
///
/// # Required fields
///
/// - `output.common` is always required.
/// - `bindings` must be `Some` when any of `output.python`, `output.node`, or `output.node_ts`
///   is `Some`; otherwise returns [`Error::MissingBindingsConfig`].
/// - `models_path_template` and `models_path_crate_template` must be valid Rust path templates
///   (containing `{service}`); invalid templates return [`Error::InvalidModelsPathTemplate`].
///
/// # Optional fields
///
/// Setting `generate_resource_enum` to `false` skips `labels.rs` generation.
/// Setting `bindings` to `None` skips all language-binding output.
///
/// Call [`CodeGenConfig::validate`] before this function to surface config errors at
/// construction time rather than mid-generation.
pub fn generate_code(metadata: &CodeGenMetadata, config: &CodeGenConfig) -> Result<()> {
    // Validate templates early so callers get a clean error before any generation starts.
    let models_path = ModelsPath::new(&config.models_path_template)?;
    let models_path_crate = ModelsPath::new(&config.models_path_crate_template)?;

    // The context/result type paths come from user config and are parsed as `syn::Path` deep in
    // the per-method generators (handler/client/server/builder). Validate them once here so a
    // typo surfaces as a clean error instead of panicking mid-generation.
    for (field, path) in [
        ("context_type", &config.context_type_path),
        ("result_type", &config.result_type_path),
    ] {
        syn::parse_str::<syn::Path>(path).map_err(|source| Error::InvalidConfigPath {
            field,
            path: path.clone(),
            source,
        })?;
    }

    // Validate that bindings config is present when language-binding output is requested.
    if (config.output.python.is_some()
        || config.output.node.is_some()
        || config.output.node_ts.is_some())
        && config.bindings.is_none()
    {
        return Err(Error::MissingBindingsConfig);
    }

    let plan = analyze_metadata(metadata)?;

    // Validate every service's `{service}` substitution up front so a base path that isn't a
    // valid Rust path segment fails cleanly here rather than panicking inside per-service codegen.
    for service in &plan.services {
        models_path.validate_for(&service.base_path)?;
        models_path_crate.validate_for(&service.base_path)?;
    }

    let common_code = generate_common_code(&plan, metadata, config)?;
    output::write_generated_code(&common_code, &config.output.common)?;

    if config.output.models.is_some() {
        let subdir = config
            .output
            .models_subdir_path()
            .expect("models is Some so subdir is Some");
        std::fs::create_dir_all(&subdir).map_err(Error::Io)?;

        if config.generate_resource_enum {
            let resource_enum = resources::generate_resource_enum(
                &plan,
                metadata,
                config,
                config.error_type_path.as_deref(),
            )?;
            let mut models_files = GeneratedCode {
                files: std::collections::HashMap::new(),
            };
            models_files
                .files
                .insert("labels.rs".to_string(), resource_enum);
            output::write_generated_code(&models_files, &subdir)?;
        }

        let gen_dir = config.models_gen_dir.as_deref().unwrap_or("../gen");
        let include_labels = config.generate_resource_enum;
        let mod_content = generate_models_mod(&plan.services, gen_dir, include_labels, metadata);
        // Route through write_generated_code so the `// @generated` header is prepended like
        // every other emitted file, rather than writing it raw.
        let mut mod_files = GeneratedCode {
            files: std::collections::HashMap::new(),
        };
        mod_files.files.insert("mod.rs".to_string(), mod_content);
        output::write_generated_code(&mod_files, &subdir)?;
    }

    if let Some(ref server_dir) = config.output.server {
        let server_code = generate_server_code(&plan, metadata, config)?;
        output::write_generated_code(&server_code, server_dir)?;
    }

    if let Some(ref client_dir) = config.output.client {
        let client_code = generate_client_code(&plan, metadata, config)?;
        output::write_generated_code(&client_code, client_dir)?;
    }

    if let Some(ref python_dir) = config.output.python {
        let python_code = generate_python_code(&plan, metadata, config)?;
        output::write_generated_code(&python_code, python_dir)?;
    }

    if let Some(ref node_dir) = config.output.node {
        let node_code = generate_node_code(&plan, metadata, config)?;
        output::write_generated_code(&node_code, node_dir)?;
    }

    if let Some(ref node_ts_dir) = config.output.node_ts {
        let node_ts_code = generate_node_ts_code(&plan, metadata, config)?;
        output::write_generated_code(&node_ts_code, node_ts_dir)?;
    }

    Ok(())
}

fn generate_common_code(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> Result<GeneratedCode> {
    let mut files = HashMap::new();

    for service in &plan.services {
        let handler = ServiceHandler {
            plan: service,
            metadata,
            config,
        };
        let server_code = server::generate_common(&handler)?;
        files.insert(format!("{}/server.rs", service.base_path), server_code);
        let module_code = generate_common_module();
        files.insert(format!("{}/mod.rs", service.base_path), module_code);
    }

    let module_code = main_module(&plan.services);
    files.insert("mod.rs".to_string(), module_code);

    Ok(GeneratedCode { files })
}

fn generate_server_code(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> Result<GeneratedCode> {
    let mut files = HashMap::new();

    for service in &plan.services {
        let handler = ServiceHandler {
            plan: service,
            metadata,
            config,
        };
        let trait_code = handler::generate(&handler)?;
        files.insert(format!("{}/handler.rs", service.base_path), trait_code);
        let server_code = server::generate_server(&handler)?;
        files.insert(format!("{}/server.rs", service.base_path), server_code);
        let module_code = generate_server_module(service);
        files.insert(format!("{}/mod.rs", service.base_path), module_code);
    }

    let module_code = main_module(&plan.services);
    files.insert("mod.rs".to_string(), module_code);

    Ok(GeneratedCode { files })
}

fn generate_python_code(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> Result<GeneratedCode> {
    let mut files = HashMap::new();

    let handlers = plan
        .services
        .iter()
        .map(|service| ServiceHandler {
            plan: service,
            metadata,
            config,
        })
        .collect_vec();

    // Per-service scoped client modules are only meaningful for resource-scoped services. The
    // methods of resource-less services live on the root client (emitted by `main_module`), so
    // they don't get a scoped module of their own.
    for service in handlers.iter().filter(|s| s.is_resource_scoped()) {
        let python_code = python::generate(service)?;
        files.insert(format!("{}.rs", service.plan.base_path), python_code);
    }

    let module_code = python::main_module(&handlers)?;
    files.insert("mod.rs".to_string(), module_code);

    let python_typings_code = python::generate_typings(&handlers);
    files.insert(
        config.output.python_typings_filename.clone(),
        python_typings_code,
    );

    Ok(GeneratedCode { files })
}

fn generate_node_code(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> Result<GeneratedCode> {
    let mut files = HashMap::new();

    let handlers = plan
        .services
        .iter()
        .map(|service| ServiceHandler {
            plan: service,
            metadata,
            config,
        })
        .collect_vec();

    // See `generate_python_code`: only resource-scoped services get a per-service scoped module;
    // resource-less services' methods live on the root client.
    for service in handlers.iter().filter(|s| s.is_resource_scoped()) {
        let napi_code = node::generate(service)?;
        files.insert(format!("{}.rs", service.plan.base_path), napi_code);
    }

    let module_code = node::main_module(&handlers)?;
    files.insert("mod.rs".to_string(), module_code);

    Ok(GeneratedCode { files })
}

fn generate_node_ts_code(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> Result<GeneratedCode> {
    let handlers = plan
        .services
        .iter()
        .map(|service| ServiceHandler {
            plan: service,
            metadata,
            config,
        })
        .collect_vec();

    let ts_code = node::typescript::generate_client_ts(&handlers);
    let mut files = HashMap::new();
    files.insert("client.ts".to_string(), ts_code);

    // The `models/index.ts` barrel re-exports every service's generated protobuf-es modules so
    // `client.ts`'s `from "./models"` imports (message and service request/response types) resolve.
    let models_barrel = node::typescript::generate_models_barrel(&handlers);
    files.insert("models/index.ts".to_string(), models_barrel);

    Ok(GeneratedCode { files })
}

fn generate_client_code(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> Result<GeneratedCode> {
    let mut files = HashMap::new();

    for service in &plan.services {
        let handler = ServiceHandler {
            plan: service,
            metadata,
            config,
        };
        let client_code = client::generate(&handler)?;
        files.insert(format!("{}/client.rs", service.base_path), client_code);
        let builder_code = builder::generate(&handler)?;
        files.insert(format!("{}/builders.rs", service.base_path), builder_code);

        // Ergonomic resource-scoped client, when enabled and the service manages a resource.
        // `plan` is passed so the emitter can resolve child services for navigation/create accessors.
        let has_resource_client = if config.output.generate_resource_clients {
            resource_client::generate(&handler, plan)?
                .map(|code| {
                    files.insert(format!("{}/resource.rs", service.base_path), code);
                })
                .is_some()
        } else {
            false
        };

        let module_code = generate_client_module(has_resource_client);
        files.insert(format!("{}/mod.rs", service.base_path), module_code);
    }

    // Top-level aggregate root client (e.g. `UnityCatalogClient`), emitted only when a bindings
    // config (which carries the aggregate name) is present.
    let has_aggregate = aggregate::generate(plan, metadata, config)?
        .map(|aggregate_code| {
            files.insert("client.rs".to_string(), aggregate_code);
        })
        .is_some();

    let module_code = generate_client_main_module(&plan.services, has_aggregate);
    files.insert("mod.rs".to_string(), module_code);

    Ok(GeneratedCode { files })
}

fn generate_common_module() -> String {
    let tokens = quote! {
        #[cfg(feature = "axum")]
        pub mod server;
    };
    format_tokens_static(tokens)
}

fn generate_server_module(service: &ServicePlan) -> String {
    let handler_ident = format_ident!("{}", service.handler_name);
    let tokens = quote! {
        pub use handler::#handler_ident;

        mod handler;
        #[cfg(feature = "axum")]
        pub mod server;
    };
    format_tokens_static(tokens)
}

fn generate_client_module(has_resource_client: bool) -> String {
    // Emit the scoped resource client module only when it was generated for this service.
    let resource_module = if has_resource_client {
        quote! {
            pub use resource::*;
            pub mod resource;
        }
    } else {
        quote! {}
    };
    let tokens = quote! {
        pub use client::*;
        pub use builders::*;
        #resource_module

        pub mod client;
        pub mod builders;
    };
    format_tokens_static(tokens)
}

pub fn main_module(services: &[ServicePlan]) -> String {
    let service_modules: Vec<TokenStream> = services
        .iter()
        .map(|s| {
            let module_name = format_ident!("{}", s.base_path);
            quote! { pub mod #module_name; }
        })
        .collect();

    let tokens = quote! {
        #(#service_modules)*
    };
    format_tokens_static(tokens)
}

fn generate_client_main_module(services: &[ServicePlan], has_aggregate: bool) -> String {
    let service_modules: Vec<TokenStream> = services
        .iter()
        .map(|s| {
            let module_name = format_ident!("{}", s.base_path);
            quote! { pub mod #module_name; }
        })
        .collect();

    // Export the aggregate root client (`client.rs`) when it was generated. Per-service files live
    // under `<base_path>/`, so a top-level `client` module never collides with them.
    let aggregate_module = if has_aggregate {
        quote! {
            pub mod client;
            pub use client::*;
        }
    } else {
        quote! {}
    };

    let tokens = quote! {
        #(#service_modules)*

        #aggregate_module

        use futures::Future;

        pub(super) fn stream_paginated<F, Fut, S, T>(
            state: S,
            op: F,
        ) -> impl futures::Stream<Item = crate::Result<T>>
        where
            F: Fn(S, Option<String>) -> Fut + Copy,
            Fut: Future<Output = crate::Result<(T, S, Option<String>)>>,
        {
            enum PaginationState<T> {
                Start(T),
                HasMore(T, String),
                Done,
            }

            futures::stream::unfold(
                PaginationState::Start(state),
                move |state| async move {
                    let (s, page_token) = match state {
                        PaginationState::Start(s) => (s, None),
                        PaginationState::HasMore(s, page_token) if !page_token.is_empty() => {
                            (s, Some(page_token))
                        }
                        _ => {
                            return None;
                        }
                    };

                    let (resp, s, continuation) = match op(s, page_token).await {
                        Ok(resp) => resp,
                        Err(e) => return Some((Err(e), PaginationState::Done)),
                    };

                    let next_state = match continuation {
                        Some(token) => PaginationState::HasMore(s, token),
                        None => PaginationState::Done,
                    };

                    Some((Ok(resp), next_state))
                },
            )
        }
    };
    format_tokens_static(tokens)
}

/// Generate the `mod.rs` for `crates/common/src/models/`.
///
/// Emits `pub mod <service> { pub mod <version> { include!(...) } }` blocks for every
/// service in the plan, plus static re-exports and module declarations.
///
/// `gen_dir` is the relative path (from the subdir) to the prost-generated files,
/// e.g. `"../gen"`.
///
/// When `include_labels` is `true`, also emits `pub mod labels; pub use labels::{ObjectLabel, Resource};`.
///
/// `metadata` is used to discover all resource-annotated messages (even those not returned
/// directly by an RPC) so they can be included in `pub use` re-exports.
pub fn generate_models_mod(
    services: &[ServicePlan],
    gen_dir: &str,
    include_labels: bool,
    metadata: &CodeGenMetadata,
) -> String {
    // Services are keyed by proto package, not by service: multiple services can share a
    // package (e.g. `CatalogService` and `SchemaService` both in `example.catalog.v1`), and
    // emitting one `pub mod`/`pub use` block per service would produce duplicate modules and
    // imports (E0428). Group by package first, then emit once per package.
    //
    // package = "unitycatalog.catalogs.v1"
    // parts   = ["unitycatalog", "catalogs", "v1"]
    // service module = second-to-last segment ("catalogs"); version = last segment ("v1").
    let mod_segments = |svc: &ServicePlan| -> (String, String) {
        let parts: Vec<&str> = svc.package.split('.').collect();
        if parts.len() >= 2 {
            (
                parts[parts.len() - 2].to_string(),
                parts[parts.len() - 1].to_string(),
            )
        } else {
            (svc.base_path.clone(), "v1".to_string())
        }
    };

    // One entry per package, in sorted order, with the union of re-exported type names.
    let mut packages: BTreeMap<String, (String, String, BTreeSet<String>)> = BTreeMap::new();
    for svc in services {
        let (svc_seg, ver_seg) = mod_segments(svc);
        let entry = packages
            .entry(svc.package.clone())
            .or_insert_with(|| (svc_seg, ver_seg, BTreeSet::new()));
        let type_names = &mut entry.2;

        // Re-export managed resources for this service...
        type_names.extend(svc.managed_resources.iter().map(|r| r.type_name.clone()));

        // ...plus every resource-annotated message in the same package (catches nested types
        // like `Column` that aren't direct RPC return types but still carry annotations).
        let fq_prefix = format!(".{}.", svc.package);
        let fq_pkg = format!(".{}", svc.package);
        for (fq_name, msg_info) in &metadata.messages {
            if msg_info.resource_descriptor.is_some()
                && (fq_name.starts_with(&fq_prefix) || fq_name.starts_with(&fq_pkg))
            {
                let simple = fq_name
                    .rfind('.')
                    .map(|i| &fq_name[i + 1..])
                    .unwrap_or(fq_name.as_str());
                type_names.insert(simple.to_string());
            }
        }
    }

    let mut service_mods: Vec<TokenStream> = Vec::new();
    let mut reexports: Vec<TokenStream> = Vec::new();
    for (package, (svc_seg, ver_seg, type_names)) in &packages {
        let svc_mod = format_ident!("{}", svc_seg);
        let ver_mod = format_ident!("{}", ver_seg);

        let main_include = format!("./{}/{}.rs", gen_dir, package);
        let tonic_include = format!("./{}/{}.tonic.rs", gen_dir, package);
        service_mods.push(quote! {
            pub mod #svc_mod {
                pub mod #ver_mod {
                    include!(#main_include);
                    #[cfg(feature = "grpc")]
                    include!(#tonic_include);
                }
            }
        });

        for type_name in type_names {
            let type_ident = format_ident!("{}", type_name);
            reexports.push(quote! {
                pub use #svc_mod::#ver_mod::#type_ident;
            });
        }
    }

    let labels_decl: TokenStream = if include_labels {
        quote! {
            pub mod labels;
            pub use labels::{ObjectLabel, Resource};
        }
    } else {
        quote! {}
    };

    let tokens = quote! {
        use std::collections::HashMap;

        #labels_decl

        #(#reexports)*

        pub type PropertyMap = HashMap<String, serde_json::Value>;

        #(#service_mods)*
    };

    format_tokens_static(tokens)
}

/// Generated code ready for output
#[derive(Debug)]
pub struct GeneratedCode {
    /// Generated files mapped by relative path
    pub files: HashMap<String, String>,
}

impl CodeGenMetadata {
    fn get_message_meta(&self, message_name: &str) -> Option<MessageMeta<'_>> {
        self.messages.get(message_name).map(|info| MessageMeta {
            info,
            metadata: self,
        })
    }
}

pub(crate) struct MessageMeta<'a> {
    info: &'a MessageInfo,
    #[allow(dead_code)]
    metadata: &'a CodeGenMetadata,
}

pub(crate) struct ServiceHandler<'a> {
    pub(crate) plan: &'a ServicePlan,
    pub(crate) metadata: &'a CodeGenMetadata,
    pub(crate) config: &'a CodeGenConfig,
}

impl ServiceHandler<'_> {
    pub(crate) fn resource(&self) -> Option<&ManagedResource> {
        self.plan.managed_resources.first()
    }

    /// Whether this service manages a `google.api.resource` and therefore gets a resource-scoped
    /// client (e.g. `CatalogClient` bound to a name).
    ///
    /// Resource-less services (entity tag assignments, temporary credentials, delta commits) have
    /// no scoped client; their methods are emitted on the root client and pass every param
    /// directly ([`BindingMode::Flat`]). See [`Self::binding_mode`].
    pub(crate) fn is_resource_scoped(&self) -> bool {
        self.resource().is_some()
    }

    /// The [`BindingMode`] used to lower this service's methods in language bindings.
    pub(crate) fn binding_mode(&self) -> BindingMode {
        if self.is_resource_scoped() {
            BindingMode::Scoped
        } else {
            BindingMode::Flat
        }
    }

    /// The language-agnostic spec for this service's resource accessor on the aggregate client, or
    /// `None` for resource-less services.
    ///
    /// All four accessor emitters (Rust aggregate, Python, Node, TypeScript) render from this single
    /// spec rather than re-deriving the param list and nesting decision, which previously diverged
    /// between the Rust aggregate and the bindings.
    pub(crate) fn accessor_spec(&self) -> Option<AccessorSpec> {
        let resource = self.resource()?;
        let params = self.plan.resource_accessor_params.clone()?;
        // A resource is nested when its accessor params include parent components beyond its own
        // name; its full name is then the dot-joined components. This is the single, converged
        // nesting rule — previously the Rust aggregate used `len > 1` while Python/Node also gated
        // on `name_field`, which could disagree.
        let nested = params.len() > 1;
        Some(AccessorSpec {
            singular: resource.descriptor.singular.clone(),
            params,
            nested,
        })
    }

    pub(crate) fn methods(&self) -> impl Iterator<Item = MethodHandler<'_>> {
        self.plan.methods.iter().map(|plan| MethodHandler {
            plan,
            metadata: self.metadata,
        })
    }

    /// The ergonomic, public-facing client type for this service.
    ///
    /// For a resource-scoped service this is the **scoped resource client** (e.g. `CatalogClient`,
    /// bound to a name) — the type callers and language bindings use. For a resource-less service it
    /// is the single flat client (e.g. `TagAssignmentClient`). The low-level transport client (one
    /// async method per RPC) is [`Self::low_level_client_type`].
    pub(crate) fn client_type(&self) -> Ident {
        if let Some(resource) = self.resource() {
            format_ident!(
                "{}",
                format!("{} client", resource.descriptor.singular).to_case(Case::Pascal)
            )
        } else {
            format_ident!(
                "{}Client",
                self.plan
                    .service_name
                    .trim_end_matches("Service")
                    .trim_end_matches('s')
            )
        }
    }

    /// The low-level transport client type (`{ client: CloudClient, base_url: Url }` with one async
    /// method per RPC), emitted by [`client`](super::client).
    ///
    /// For a resource-scoped service this is `<Singular>ServiceClient` (e.g. `CatalogServiceClient`),
    /// distinct from the ergonomic scoped [`Self::client_type`] (`CatalogClient`). For a
    /// resource-less service there is no scoped client, so the low-level client *is* the
    /// [`Self::client_type`] (e.g. `TagAssignmentClient`).
    pub(crate) fn low_level_client_type(&self) -> Ident {
        if let Some(resource) = self.resource() {
            format_ident!(
                "{}ServiceClient",
                resource.descriptor.singular.to_case(Case::Pascal)
            )
        } else {
            self.client_type()
        }
    }

    /// The ergonomic scoped resource client type, or `None` for a resource-less service.
    ///
    /// Same as [`Self::client_type`] but `Option`-typed to make "resource-scoped only" explicit at
    /// call sites that generate the scoped client / its accessor.
    pub(crate) fn scoped_client_type(&self) -> Option<Ident> {
        self.resource().map(|_| self.client_type())
    }

    /// The module segment under which this service's generated models live.
    ///
    /// Models are emitted per proto *package* (e.g. `unitycatalog.tags.v1` → module `tags`),
    /// which is not always the same as the service's `base_path` (derived from the service
    /// *name*, e.g. `TagPoliciesService` → `tag_policies`). Use the package's leaf-before-version
    /// segment so the models import path matches where the proto plugin actually wrote the types.
    /// Falls back to `base_path` when the package is empty or unparsable.
    fn models_segment(&self) -> String {
        let segs: Vec<&str> = self
            .plan
            .package
            .split('.')
            .filter(|s| !s.is_empty())
            .collect();
        match segs.as_slice() {
            // `unitycatalog.tags.v1` → `tags`
            [.., name, version] if is_version_segment(version) => name.to_string(),
            // `unitycatalog.tags` → `tags`
            [.., name] => name.to_string(),
            // No package info: fall back to the service-name-derived base path.
            _ => self.plan.base_path.clone(),
        }
    }

    pub(crate) fn models_path(&self) -> syn::Path {
        // Templates (and each service's substitution) are validated by `generate_code` before
        // any `ServiceHandler` is used, so skip the redundant re-validation `new` would do.
        ModelsPath::from_template(&self.config.models_path_template).resolve(&self.models_segment())
    }

    pub(crate) fn models_path_crate(&self) -> syn::Path {
        ModelsPath::from_template(&self.config.models_path_crate_template)
            .resolve(&self.models_segment())
    }
}

/// Whether a proto package segment is a version marker like `v1`, `v2beta1`, etc.
fn is_version_segment(seg: &str) -> bool {
    seg.strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

pub(crate) struct MethodHandler<'a> {
    plan: &'a MethodPlan,
    metadata: &'a CodeGenMetadata,
}

impl MethodHandler<'_> {
    pub(crate) fn is_collection_method(&self) -> bool {
        crate::analysis::is_collection_method(
            &self.plan.request_type,
            &self.plan.metadata.method_name,
        )
    }

    /// The language-agnostic [`EmitShape`] for this method, derived once from its request type and
    /// name. The Python/Node/TypeScript emitters branch on this instead of each re-matching
    /// `RequestType`.
    pub(crate) fn emit_shape(&self) -> crate::analysis::EmitShape {
        crate::analysis::emit_shape(&self.plan.request_type, self.is_collection_method())
    }

    /// Whether this method is emitted as an instance method on a resource-scoped client
    /// (`get`/`update`/`delete` or a resource-targeted custom `POST`/`PATCH`).
    ///
    /// This is the exact set the bindings previously matched in their `resource_client_method`
    /// dispatch: standard `Get`/`Update`/`Delete`, plus a non-collection custom `POST`/`PATCH`
    /// (e.g. `POST /catalogs/{name}:rotateToken`). Other custom verbs (a custom `GET`/`DELETE`) are
    /// deliberately **not** surfaced on the scoped client, matching prior behavior.
    pub(crate) fn is_scoped_instance_method(&self) -> bool {
        matches!(
            self.plan.request_type,
            RequestType::Get | RequestType::Update | RequestType::Delete
        ) || (matches!(
            self.plan.request_type,
            RequestType::Custom(Pattern::Post(_) | Pattern::Patch(_))
        ) && !self.is_collection_method())
    }

    /// The ordered binding parameter list for `mode`, with shared filtering applied once.
    ///
    /// Required params come before optional ones (matching the previous
    /// `required_parameters().chain(optional_parameters())` order). Filtering reproduces exactly what
    /// the old per-language `collection_method_parameters` / `resource_method_parameters` did,
    /// keyed on [`EmitShape`]:
    ///
    /// - **collection shapes** (`List`, `Create`): never drop path params (collection methods live
    ///   on the root client and pass everything); drop `page_token` for `List` (streaming handles
    ///   pagination).
    /// - **instance shapes** (`GetUpdate`, `Delete`): drop path params in [`BindingMode::Scoped`]
    ///   (the scoped client already holds them); never filter `page_token`.
    ///
    /// Per-language emitters render the returned `&RequestParam`s (applying their own type mapping /
    /// capability filters) instead of each re-deriving this filtered list.
    pub(crate) fn param_plan(&self, mode: BindingMode) -> Vec<&RequestParam> {
        let (required, optional) = self.param_plan_split(mode);
        required.into_iter().chain(optional).collect()
    }

    /// Like [`Self::param_plan`] but keeps the required/optional partition, for emitters (e.g.
    /// TypeScript) that render the two groups differently (required positional args vs an optional
    /// `options` object). Both groups have the same shared path/`page_token` filtering applied.
    pub(crate) fn param_plan_split(
        &self,
        mode: BindingMode,
    ) -> (Vec<&RequestParam>, Vec<&RequestParam>) {
        use crate::analysis::EmitShape;
        let shape = self.emit_shape();
        let is_collection_shape = matches!(shape, EmitShape::List | EmitShape::Create);
        let drop_path = !is_collection_shape && mode == BindingMode::Scoped;
        let drop_page_token = shape == EmitShape::List;
        let keep = |p: &&RequestParam| {
            let is_dropped_path = drop_path && p.is_path_param();
            let is_dropped_page_token = drop_page_token && p.name() == "page_token";
            !is_dropped_path && !is_dropped_page_token
        };
        let required = self.required_parameters().filter(keep).collect();
        let optional = self.optional_parameters().filter(keep).collect();
        (required, optional)
    }

    pub(crate) fn output_message(&self) -> Option<MessageMeta<'_>> {
        if self.plan.metadata.output_type.ends_with("Empty") {
            return None;
        }
        self.metadata
            .get_message_meta(&self.plan.metadata.output_type)
    }

    pub(crate) fn output_type(&self) -> Option<Ident> {
        self.output_message()
            .map(|t| extract_type_ident(&t.info.name))
    }

    /// The method's output type as a `syn::Type`, falling back to the unit type `()` when the
    /// RPC returns `Empty` (i.e. [`Self::output_message`] is `None`).
    ///
    /// Binding emitters splice the response type into `Result<T>` wrappers
    /// (e.g. `PyUnityCatalogResult<T>`); interpolating a bare `Option::None` would produce an
    /// uncompilable `PyUnityCatalogResult<>`. Use this so `Empty`-returning RPCs yield
    /// `PyUnityCatalogResult<()>` instead.
    pub(crate) fn output_type_or_unit(&self) -> syn::Type {
        match self.output_type() {
            Some(ident) => syn::parse_quote!(#ident),
            None => syn::parse_quote!(()),
        }
    }

    pub(crate) fn list_output_field(&self) -> Option<&MessageField> {
        self.output_message()?
            .info
            .fields
            .iter()
            .find(|f| !f.name.contains("page_token"))
    }

    pub(crate) fn input_message(&self) -> Option<MessageMeta<'_>> {
        if self.plan.metadata.input_type == "Empty" {
            return None;
        }
        self.metadata
            .get_message_meta(&self.plan.metadata.input_type)
    }

    pub(crate) fn input_type(&self) -> Option<Ident> {
        self.input_message()
            .map(|t| extract_type_ident(&t.info.name))
    }

    pub(crate) fn builder_type(&self) -> Ident {
        format_ident!("{}Builder", self.plan.metadata.method_name)
    }

    /// The method name to expose in language bindings.
    ///
    /// Prefers the gnostic `operation_id` annotation (`gnostic.openapi.v3.operation`) when present,
    /// snake-cased, so proto authors can give bindings a hand-tuned name. Falls back to the
    /// proto-derived name ([`MethodPlan::base_method_ident`]).
    ///
    /// This affects only the *emitted* binding method name — the call into the underlying Rust
    /// client still uses [`MethodPlan::base_method_ident`], so the handler/client API is unchanged.
    pub(crate) fn binding_method_name(&self) -> Ident {
        format_ident!("{}", self.binding_method_name_str())
    }

    /// The snake_case binding method name (see [`Self::binding_method_name`]).
    ///
    /// Provided as a `String` so the string-templated TypeScript generator can camel-case it and
    /// stay consistent with the snake→camel name NAPI derives for the native method.
    pub(crate) fn binding_method_name_str(&self) -> String {
        match self.plan.metadata.operation.as_ref() {
            Some(op) if !op.operation_id.is_empty() => op.operation_id.to_case(Case::Snake),
            _ => self.plan.handler_function_name.clone(),
        }
    }

    /// Get type representation for rust depending on context
    ///
    /// Depending on context we may want concrete types (e.g. 'String') or more flexible types (e.g. 'Into<String d>')
    pub(crate) fn field_type(&self, field_type: &UnifiedType, ctx: RenderContext) -> syn::Type {
        let rust_type = types::unified_to_rust(field_type, ctx);
        // `rust_type` comes from this crate's own proto→Rust mapping, not user input, so an
        // unparsable result is a bug in `unified_to_rust`. Panic with the offending string so
        // the broken mapping is obvious (the generated-code parse test in
        // `tests/generation_integration.rs` is the broader guard for output validity).
        syn::parse_str(&rust_type).unwrap_or_else(|e| {
            panic!("internal: unified_to_rust produced an invalid type `{rust_type}`: {e}")
        })
    }

    /// Get field assignment TokenStream for constructor
    pub(crate) fn field_assignment(
        &self,
        field_type: &UnifiedType,
        field_ident: &proc_macro2::Ident,
        ctx: &RenderContext,
    ) -> TokenStream {
        types::field_assignment(field_type, field_ident, ctx)
    }

    pub(crate) fn required_parameters(&self) -> impl Iterator<Item = &RequestParam> {
        self.plan
            .parameters
            .iter()
            .filter(|param| !param.is_optional())
    }

    pub(crate) fn optional_parameters(&self) -> impl Iterator<Item = &RequestParam> {
        self.plan
            .parameters
            .iter()
            .filter(|param| param.is_optional())
    }

    /// Split body fields into required and optional subsets.
    pub(crate) fn split_body_fields(&self) -> (Vec<&BodyField>, Vec<&BodyField>) {
        split_body_fields(self.plan)
    }
}

/// Extract the final type name from a fully qualified protobuf type and convert to Ident
pub(crate) fn extract_type_ident(full_type: &str) -> Ident {
    let type_name = full_type.split('.').next_back().unwrap_or(full_type);
    format_ident!("{}", type_name)
}
