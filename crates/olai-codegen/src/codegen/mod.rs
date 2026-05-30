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

use std::collections::HashMap;

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

mod builder;
mod client;
mod config;
mod handler;
pub(crate) mod node;
mod python;
mod resources;
mod server;
mod tokens;

pub use config::{BindingsConfig, CodeGenConfig, CodeGenOutput, ModelsPath};
pub(crate) use tokens::{doc_tokens, format_tokens, format_tokens_static};

impl MethodPlan {
    pub(crate) fn resource_client_method(&self) -> Ident {
        match self.request_type {
            RequestType::Get => format_ident!("get"),
            RequestType::Update => format_ident!("update"),
            RequestType::Delete => format_ident!("delete"),
            _ => format_ident!("{}", self.handler_function_name),
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
    ModelsPath::new(&config.models_path_template)?;
    ModelsPath::new(&config.models_path_crate_template)?;

    // Validate that bindings config is present when language-binding output is requested.
    if (config.output.python.is_some()
        || config.output.node.is_some()
        || config.output.node_ts.is_some())
        && config.bindings.is_none()
    {
        return Err(Error::MissingBindingsConfig);
    }

    let plan = analyze_metadata(metadata)?;

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
        let mod_path = subdir.join("mod.rs");
        std::fs::write(&mod_path, mod_content).map_err(Error::Io)?;
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

    for service in &handlers {
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

    for service in &handlers {
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
        let module_code = generate_client_module();
        files.insert(format!("{}/mod.rs", service.base_path), module_code);
    }

    let module_code = generate_client_main_module(&plan.services);
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

fn generate_client_module() -> String {
    let tokens = quote! {
        pub use client::*;
        pub use builders::*;

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

fn generate_client_main_module(services: &[ServicePlan]) -> String {
    let service_modules: Vec<TokenStream> = services
        .iter()
        .map(|s| {
            let module_name = format_ident!("{}", s.base_path);
            quote! { pub mod #module_name; }
        })
        .collect();

    let tokens = quote! {
        #(#service_modules)*

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
    let mut sorted_services: Vec<&ServicePlan> = services.iter().collect();
    sorted_services.sort_by_key(|s| &s.base_path);

    // Build the `pub mod` blocks
    let service_mods: Vec<TokenStream> = sorted_services
        .iter()
        .map(|svc| {
            // package = "unitycatalog.catalogs.v1"
            // parts   = ["unitycatalog", "catalogs", "v1"]
            let parts: Vec<&str> = svc.package.split('.').collect();
            // service module = second-to-last segment (e.g. "catalogs")
            // version module = last segment (e.g. "v1")
            let (svc_seg, ver_seg) = if parts.len() >= 2 {
                (parts[parts.len() - 2], parts[parts.len() - 1])
            } else {
                (svc.base_path.as_str(), "v1")
            };

            let svc_mod = format_ident!("{}", svc_seg);
            let ver_mod = format_ident!("{}", ver_seg);

            let main_include = format!("./{}/{}.rs", gen_dir, svc.package);
            let tonic_include = format!("./{}/{}.tonic.rs", gen_dir, svc.package);

            quote! {
                pub mod #svc_mod {
                    pub mod #ver_mod {
                        include!(#main_include);
                        #[cfg(feature = "grpc")]
                        include!(#tonic_include);
                    }
                }
            }
        })
        .collect();

    // Collect `pub use` re-exports: include managed resources AND all resource-descriptor
    // messages in the same package (catches nested types like Column that aren't direct
    // RPC return types but still have google.api.resource annotations).
    let mut reexports: Vec<TokenStream> = Vec::new();
    for svc in &sorted_services {
        let package = &svc.package;
        let fq_prefix = format!(".{}.", package);

        let parts: Vec<&str> = svc.package.split('.').collect();
        let (svc_seg, ver_seg) = if parts.len() >= 2 {
            (parts[parts.len() - 2], parts[parts.len() - 1])
        } else {
            (svc.base_path.as_str(), "v1")
        };
        let svc_mod = format_ident!("{}", svc_seg);
        let ver_mod = format_ident!("{}", ver_seg);

        // Gather from managed_resources first.
        let mut type_names: std::collections::BTreeSet<String> = svc
            .managed_resources
            .iter()
            .map(|r| r.type_name.clone())
            .collect();

        // Also include all resource-annotated messages in this package.
        for (fq_name, msg_info) in &metadata.messages {
            if msg_info.resource_descriptor.is_some()
                && (fq_name.starts_with(&fq_prefix)
                    || fq_name.starts_with(&format!(".{}", package)))
            {
                // Simple name = last component after '.'
                let simple = fq_name
                    .rfind('.')
                    .map(|i| &fq_name[i + 1..])
                    .unwrap_or(fq_name.as_str());
                type_names.insert(simple.to_string());
            }
        }

        for type_name in &type_names {
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

    pub(crate) fn methods(&self) -> impl Iterator<Item = MethodHandler<'_>> {
        self.plan.methods.iter().map(|plan| MethodHandler {
            plan,
            metadata: self.metadata,
        })
    }

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

    pub(crate) fn models_path(&self) -> syn::Path {
        // Templates are validated by `generate_code` before any `ServiceHandler` is used,
        // so this substitution is guaranteed to succeed.
        ModelsPath::new(&self.config.models_path_template)
            .expect("models_path_template already validated by generate_code")
            .resolve(&self.plan.base_path)
    }

    pub(crate) fn models_path_crate(&self) -> syn::Path {
        ModelsPath::new(&self.config.models_path_crate_template)
            .expect("models_path_crate_template already validated by generate_code")
            .resolve(&self.plan.base_path)
    }
}

pub(crate) struct MethodHandler<'a> {
    plan: &'a MethodPlan,
    metadata: &'a CodeGenMetadata,
}

impl MethodHandler<'_> {
    pub(crate) fn is_collection_method(&self) -> bool {
        matches!(
            self.plan.request_type,
            RequestType::List | RequestType::Create
        ) || (matches!(self.plan.request_type, RequestType::Custom(Pattern::Get(_)))
            && self.plan.metadata.method_name.starts_with("List"))
            || (matches!(
                self.plan.request_type,
                RequestType::Custom(Pattern::Post(_))
            ) && self.plan.metadata.method_name.starts_with("Generate"))
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

    /// Get type representation for rust depending on context
    ///
    /// Depending on context we may want concrete types (e.g. 'String') or more flexible types (e.g. 'Into<String d>')
    pub(crate) fn field_type(&self, field_type: &UnifiedType, ctx: RenderContext) -> syn::Type {
        let rust_type = types::unified_to_rust(field_type, ctx);
        syn::parse_str(&rust_type).expect("proper field type")
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
