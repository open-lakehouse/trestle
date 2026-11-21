//! NAPI-RS binding generation for protobuf-defined services.
//!
//! The dispatch and "which methods land where" logic is shared with the Python emitter via
//! [`crate::codegen::bindings`]; this module supplies the NAPI-specific scaffold, marshaling, and
//! method bodies through [`NapiBackend`]'s [`BindingBackend`] impl.

use convert_case::{Case, Casing};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::super::format_tokens;
use super::caps::{crosses_napi, is_message_param, is_message_type, is_napi_supported_type};
use crate::codegen::bindings::builder::{OptionalSetter, SetterRender, generate_builder_pattern};
use crate::codegen::bindings::{BindingBackend, ShapeParts, driver};
use crate::codegen::{BindingMode, MethodHandler, Runtime, ServiceHandler};
use crate::parsing::types::{BaseType, RenderContext, UnifiedType};

/// The `use <crate>::Message;` import that brings the model runtime's `Message` trait
/// (`encode_to_vec` / decode) into scope for the generated NAPI module.
fn message_trait_import(runtime: Runtime) -> TokenStream {
    match runtime {
        Runtime::Prost => quote! { use prost::Message; },
        Runtime::Buffa => quote! { use buffa::Message; },
    }
}

/// NAPI-RS binding backend.
///
/// Carries the target [`Runtime`] so the setter renderer (which only receives `&self`) can emit
/// the correct enum conversion for the consumed model ABI.
pub(crate) struct NapiBackend {
    runtime: Runtime,
}

pub fn main_module(services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
    let runtime = services
        .first()
        .map(|s| s.config.runtime)
        .unwrap_or(Runtime::Prost);
    NapiBackend { runtime }.main_module(services)
}

pub(crate) fn generate(service: &ServiceHandler<'_>) -> crate::error::Result<String> {
    NapiBackend {
        runtime: service.config.runtime,
    }
    .generate_service(service)
}

impl BindingBackend for NapiBackend {
    fn generate_service(&self, service: &ServiceHandler<'_>) -> crate::error::Result<String> {
        let bindings = service.config.bindings_config();

        let rust_client_ident = service.client_type();
        let napi_client_ident = format_ident!("Napi{}", rust_client_ident);

        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let napi_error_ext_ident = format_ident!("{}", bindings.napi_error_ext_trait);

        let methods = driver::scoped_client_methods(self, service);
        let mod_path = service.models_path();
        let message_import = message_trait_import(service.config.runtime);

        let tokens = quote! {
            #![allow(unused_mut, unused_imports, dead_code, clippy::all)]
            use std::collections::HashMap;
            use napi::bindgen_prelude::Buffer;
            use napi_derive::napi;
            #message_import
            use #client_crate::#rust_client_ident;
            use #mod_path::*;
            use crate::error::#napi_error_ext_ident;

            #[napi]
            pub struct #napi_client_ident {
                pub(crate) client: #rust_client_ident,
            }

            #[napi]
            impl #napi_client_ident {
                #(#methods)*
            }

            impl #napi_client_ident {
                pub fn new(client: #rust_client_ident) -> Self {
                    Self { client }
                }
            }
        };

        format_tokens(tokens)
    }

    fn main_module(&self, services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
        // Only services with a scoped binding have a per-service scoped module; resource-less and
        // flat-lowered services' methods live on the root client.
        let service_modules = services
            .iter()
            .filter(|s| s.emits_scoped_binding())
            .map(|s| {
                let module_name = format_ident!("{}", s.plan.base_path);
                quote! { pub mod #module_name; }
            });

        let bindings = services
            .first()
            .expect("binding output has a service")
            .config
            .bindings_config();

        let aggregate_client_name = &bindings.aggregate_client_name;
        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let aggregate_client_ident = format_ident!("{}", aggregate_client_name);
        let napi_aggregate_client_ident = format_ident!("Napi{}", aggregate_client_name);
        let napi_error_ext_ident = format_ident!("{}", bindings.napi_error_ext_trait);

        let sorted_services = driver::sorted_services(services);

        let mod_paths = sorted_services.iter().map(|s| {
            let mod_path = s.models_path();
            quote! { use #mod_path::*; }
        });

        let codegen_imports = driver::scoped_client_imports(&sorted_services, "Napi");

        let methods = driver::root_client_methods(self, &sorted_services);
        let resource_accessor_methods = driver::resource_accessor_methods(self, &sorted_services);

        // All services in a generation share one runtime; read it off the first.
        let message_import = message_trait_import(
            services
                .first()
                .map(|s| s.config.runtime)
                .unwrap_or(Runtime::Prost),
        );

        let tokens = quote! {
            #![allow(unused_mut, unused_imports, dead_code, clippy::all)]
            #(#service_modules)*

            use std::collections::HashMap;
            use futures::stream::TryStreamExt;
            use futures::StreamExt;
            use napi::bindgen_prelude::{Buffer, ReadableStream};
            use napi::Env;
            use napi_derive::napi;
            #message_import
            use #client_crate::#aggregate_client_ident;
            use crate::error::#napi_error_ext_ident;
            #(#mod_paths)*
            #(#codegen_imports)*

            #[napi]
            pub struct #napi_aggregate_client_ident {
                client: #aggregate_client_ident
            }

            #[napi]
            impl #napi_aggregate_client_ident {
                #[napi(factory)]
                pub fn from_url(base_url: String, token: Option<String>) -> napi::Result<Self> {
                    let client = if let Some(token) = token {
                        olai_http::CloudClient::new_with_token(token)
                    } else {
                        olai_http::CloudClient::new_unauthenticated()
                    };
                    let base_url = base_url.parse().map_err(|e: url::ParseError| {
                        napi::Error::new(napi::Status::GenericFailure, e.to_string())
                    })?;
                    Ok(Self { client: #aggregate_client_ident::new(client, base_url) })
                }

                #(#methods)*

                #(#resource_accessor_methods)*
            }
        };

        format_tokens(tokens)
    }

    /// Render the NAPI method param defs from the shared [`MethodHandler::param_plan`], then apply the
    /// NAPI-specific capability filter (drop params NAPI can't pass; messages cross as `Buffer`s).
    /// Path/`page_token` filtering already happened in `param_plan`.
    fn param_defs(&self, method: &MethodHandler<'_>, mode: BindingMode) -> Vec<TokenStream> {
        method
            .param_plan(mode)
            .into_iter()
            // Keep NAPI-native params and messages (passed as `Buffer`s); drop the rest (oneofs and
            // free-form JSON fields).
            .filter(|p| crosses_napi(p))
            .map(|p| {
                let param_name = napi_param_ident(p.name());
                if is_message_param(p) {
                    // Messages cross the NAPI boundary as serialized bytes (decoded below).
                    let mut ty = quote! { napi::bindgen_prelude::Buffer };
                    if p.field_type().is_repeated {
                        ty = quote! { ::std::vec::Vec<#ty> };
                    }
                    if p.is_optional() {
                        ty = quote! { ::core::option::Option<#ty> };
                    }
                    quote! { #param_name: #ty }
                } else {
                    let rust_type = method.field_type(p.field_type(), RenderContext::NapiParameter);
                    quote! { #param_name: #rust_type }
                }
            })
            .collect()
    }

    fn client_call(&self, method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream {
        let (method_name, drop_path) = driver::client_method(method, mode);
        let args = method
            .required_parameters()
            .filter(|param| !(drop_path && param.is_path_param()) && crosses_napi(param))
            .map(|param| {
                if driver::is_seeded_page_token(method, param) {
                    return quote! { ::std::string::String::new() };
                }
                let param_name = napi_param_ident(param.name());
                if is_message_param(param) {
                    // Decode the protobuf bytes received over the NAPI boundary back into the
                    // message(s).
                    let ty = param.field_type();
                    if ty.is_repeated {
                        let decode = decode_message(method.config.runtime, ty, quote! { b });
                        quote! {
                            #param_name
                                .iter()
                                .map(|b| #decode)
                                .collect::<napi::Result<::std::vec::Vec<_>>>()?
                        }
                    } else {
                        let decode =
                            decode_message(method.config.runtime, ty, quote! { #param_name });
                        quote! { #decode? }
                    }
                } else if let BaseType::Enum(enum_name) = &param.field_type().base_type {
                    // The enum crosses the NAPI boundary as `i32`; convert to the bare enum the
                    // builder method expects (it wraps to the runtime representation internally).
                    // prost enums impl `TryFrom<i32>`; buffa enums use `Enumeration::from_i32`.
                    match method.config.runtime {
                        Runtime::Prost => quote! {
                            #param_name.try_into().map_err(|_| napi::Error::new(napi::Status::GenericFailure, "invalid enum value"))?
                        },
                        Runtime::Buffa => {
                            // Resolve to the enum's in-crate Rust path: a nested (message-defined)
                            // enum lives in a snake_case parent module (`req::Operation`) and is not
                            // reachable by the model glob import, so the bare name won't resolve.
                            let enum_ty = enum_rust_type_path(enum_name);
                            quote! {
                                <#enum_ty as buffa::Enumeration>::from_i32(#param_name)
                                    .ok_or_else(|| napi::Error::new(napi::Status::GenericFailure, "invalid enum value"))?
                            }
                        }
                    }
                } else {
                    quote! { #param_name }
                }
            });
        quote! {
            self.client.#method_name(#(#args,)*)
        }
    }

    /// A list method emits both the batch and streaming variants (the Node-specific shape).
    fn emit_list(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let param_defs = &parts.param_defs;
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, true, self);
        let method_name = method.binding_method_name();
        let stream_method_name = format_ident!("{}_stream", method_name);

        // In napi-rs v3, `ReadableStream::new` requires an `&Env` parameter and returns
        // `napi::Result`. The streaming method is non-async; the stream is driven lazily by the
        // Node.js consumer via the Web Streams `pull` protocol. The `Env` argument is injected by
        // napi-rs when the method signature includes `env: napi::Env`.
        quote! {
            #[napi(catch_unwind)]
            pub async fn #method_name(
                &self,
                #(#param_defs,)*
            ) -> napi::Result<Vec<Buffer>> {
                let request = #client_call #(#builder_calls)*;
                request
                    .into_stream()
                    .map_ok(|item| Buffer::from(item.encode_to_vec()))
                    .try_collect::<Vec<_>>()
                    .await
                    .default_error()
            }

            #[napi(catch_unwind)]
            pub fn #stream_method_name(
                &self,
                env: Env,
                #(#param_defs,)*
            ) -> napi::Result<ReadableStream<'_, Buffer>> {
                let request = #client_call #(#builder_calls)*;
                ReadableStream::new(
                    &env,
                    request
                        .into_stream()
                        .map(|item| {
                            item.map(|v| Buffer::from(v.encode_to_vec()))
                                .map_err(|e| crate::error::convert_error(&e))
                        }),
                )
            }
        }
    }

    fn emit_create(&self, parts: &ShapeParts<'_>) -> TokenStream {
        self.emit_unary(parts, parts.method.binding_method_name())
    }

    fn emit_get_update(&self, parts: &ShapeParts<'_>) -> TokenStream {
        self.emit_unary(parts, parts.unary_method_name())
    }

    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method_name = parts.unary_method_name();
        let param_defs = &parts.param_defs;
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(parts.method, false, self);

        quote! {
            #[napi(catch_unwind)]
            pub async fn #method_name(
                &self,
                #(#param_defs,)*
            ) -> napi::Result<()> {
                let request = #client_call #(#builder_calls)*;
                request
                    .await
                    .default_error()
            }
        }
    }

    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream> {
        let spec = service.accessor_spec()?;
        // napi-derive can't take a raw ident (it derives `r#type_c_callback`), so a keyword
        // singular gets a `_` suffix in Rust and its own name in JS.
        let name = napi_ident(&spec.singular);
        let attr = if name == spec.singular {
            quote! { #[napi] }
        } else {
            let js_name = spec.singular.to_case(Case::Camel);
            quote! { #[napi(js_name = #js_name)] }
        };
        driver::resource_accessor(service, attr, "Napi", name)
    }
}

impl NapiBackend {
    /// Shared body for the unary (create / get-update) methods: a `Buffer` return when the method
    /// has a response type, or `()` otherwise. Delete is `()`-only and emitted directly.
    fn emit_unary(&self, parts: &ShapeParts<'_>, method_name: Ident) -> TokenStream {
        let method = parts.method;
        let param_defs = &parts.param_defs;
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, false, self);

        if method.output_type().is_some() {
            quote! {
                #[napi(catch_unwind)]
                pub async fn #method_name(
                    &self,
                    #(#param_defs,)*
                ) -> napi::Result<Buffer> {
                    let request = #client_call #(#builder_calls)*;
                    request
                        .await
                        .map(|item| Buffer::from(item.encode_to_vec()))
                        .default_error()
                }
            }
        } else {
            // Custom POST/PATCH RPCs that target a resource can return `Empty` (e.g. a
            // delta-commit-style `Commit`). Emit a `()` return rather than a `Buffer` of an
            // empty message; an existence check returns its `bool`.
            let ret = method.result_type();
            quote! {
                #[napi(catch_unwind)]
                pub async fn #method_name(
                    &self,
                    #(#param_defs,)*
                ) -> napi::Result<#ret> {
                    let request = #client_call #(#builder_calls)*;
                    request
                        .await
                        .default_error()
                }
            }
        }
    }
}

impl SetterRender for NapiBackend {
    fn param_ident(&self, name: &str) -> Ident {
        napi_param_ident(name)
    }

    fn supports(&self, setter: &OptionalSetter<'_>) -> bool {
        is_napi_supported_type(&setter.field_type.base_type) || is_message_type(setter.field_type)
    }

    fn render_setter(
        &self,
        setter: &OptionalSetter<'_>,
        param_ident: &Ident,
        with_method: &Ident,
    ) -> TokenStream {
        if is_message_type(setter.field_type) {
            // Optional messages cross as `Option<Buffer>` (`Option<Vec<Buffer>>` for a list, left
            // out as the empty list); decode them for the builder.
            let decode = decode_message(self.runtime, setter.field_type, quote! { b });
            return if setter.field_type.is_repeated {
                quote! {
                    .#with_method(
                        #param_ident
                            .unwrap_or_default()
                            .iter()
                            .map(|b| #decode)
                            .collect::<napi::Result<::std::vec::Vec<_>>>()?,
                    )
                }
            } else {
                quote! { .#with_method(#param_ident.as_ref().map(|b| #decode).transpose()?) }
            };
        }
        if matches!(setter.field_type.base_type, BaseType::Map(_, _))
            || setter.field_type.is_repeated
        {
            // A map or list left out is the empty collection.
            quote! { .#with_method(#param_ident.unwrap_or_default()) }
        } else if let BaseType::Enum(enum_name) = &setter.field_type.base_type {
            // Enums cross as `Option<i32>`; convert to `Option<E>` (the bare enum the builder's
            // `with_*` setter expects) and drop invalid values. prost enums impl `TryFrom<i32>`;
            // buffa enums use `Enumeration::from_i32`.
            match self.runtime {
                Runtime::Prost => quote! {
                    .#with_method(#param_ident.map(|v| v.try_into().ok()).flatten())
                },
                Runtime::Buffa => {
                    let enum_ty = enum_rust_type_path(enum_name);
                    quote! {
                        .#with_method(#param_ident.and_then(<#enum_ty as buffa::Enumeration>::from_i32))
                    }
                }
            }
        } else {
            quote! { .#with_method(#param_ident) }
        }
    }
}

/// In-crate Rust path for an enum referenced from generated node bindings.
///
/// The buffa conversion (`Enumeration::from_i32`) names the enum type explicitly,
/// so a message-nested enum must be reached through its snake_case parent module
/// (`generate_temporary_table_credentials_request::Operation`) rather than the bare
/// leaf name, which the model glob import does not bring into scope. Top-level enums
/// resolve to their simple name unchanged.
fn enum_rust_type_path(enum_name: &str) -> TokenStream {
    let path = crate::parsing::types::convert_protobuf_enum_to_rust_type(enum_name);
    path.parse()
        .expect("enum rust path is a valid token stream")
}

/// The `napi::Result` of decoding the protobuf bytes `bytes` (a `Buffer` or `&Buffer`) into the
/// message type of `ty`, with the consumed runtime's `Message` trait: prost's `decode(impl Buf)`,
/// buffa's `decode_from_slice(&[u8])`.
fn decode_message(runtime: Runtime, ty: &UnifiedType, bytes: TokenStream) -> TokenStream {
    let msg_ty = match &ty.base_type {
        BaseType::Message(n) | BaseType::OneOf(n) => {
            format_ident!("{}", crate::utils::extract_simple_type_name(n))
        }
        other => unreachable!("only messages cross NAPI as bytes, not {other:?}"),
    };
    let decode = match runtime {
        Runtime::Prost => quote! { <#msg_ty as prost::Message>::decode(#bytes.as_ref()) },
        Runtime::Buffa => {
            quote! { <#msg_ty as buffa::Message>::decode_from_slice(#bytes.as_ref()) }
        }
    };
    quote! {
        #decode.map_err(|e| {
            napi::Error::new(
                napi::Status::GenericFailure,
                format!("invalid {} payload: {e}", stringify!(#msg_ty)),
            )
        })
    }
}

/// The Rust ident of a `#[napi]` method named `name`: a Rust keyword gets a `_` suffix (`type_`),
/// since napi-derive builds further idents from it and can't take a raw one.
fn napi_ident(name: &str) -> Ident {
    let ident = crate::utils::field_ident(name);
    if ident.to_string().starts_with("r#") {
        format_ident!("{name}_")
    } else {
        ident
    }
}

/// The Rust ident of a `#[napi]` method parameter for the proto field `name`. napi-derive writes
/// the camelCased ident into the `.d.ts` (dropping a `_` suffix), so it must be neither raw
/// (`r#type`) nor a JS reserved word once camelCased (`delete`, `super`): a reserved word becomes
/// `<name>_param` (`deleteParam`), another Rust keyword `<name>_` (`type`). The call is positional,
/// so the name doesn't reach the wire.
fn napi_param_ident(name: &str) -> Ident {
    if super::is_js_reserved(&name.to_case(Case::Camel)) {
        format_ident!("{name}_param")
    } else {
        napi_ident(name)
    }
}
