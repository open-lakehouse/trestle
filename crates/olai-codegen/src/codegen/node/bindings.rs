//! NAPI-RS binding generation for protobuf-defined services.
//!
//! The dispatch and "which methods land where" logic is shared with the Python emitter via
//! [`crate::codegen::bindings`]; this module supplies the NAPI-specific scaffold, marshaling, and
//! method bodies through [`NapiBackend`]'s [`BindingBackend`] impl.

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::super::format_tokens;
use super::caps::{is_napi_supported, is_napi_supported_type, is_required_message_body};
use crate::codegen::bindings::builder::{OptionalSetter, SetterRender, generate_builder_pattern};
use crate::codegen::bindings::{BindingBackend, ShapeParts, driver};
use crate::codegen::{BindingMode, MethodHandler, ServiceHandler};
use crate::parsing::types::{BaseType, RenderContext};

/// NAPI-RS binding backend.
pub(crate) struct NapiBackend;

pub fn main_module(services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
    NapiBackend.main_module(services)
}

pub(crate) fn generate(service: &ServiceHandler<'_>) -> crate::error::Result<String> {
    NapiBackend.generate_service(service)
}

impl BindingBackend for NapiBackend {
    fn generate_service(&self, service: &ServiceHandler<'_>) -> crate::error::Result<String> {
        let bindings = service
            .config
            .bindings
            .as_ref()
            .expect("bindings config required for node output");

        let rust_client_ident = service.client_type();
        let napi_client_ident = format_ident!("Napi{}", rust_client_ident);

        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let napi_error_ext_ident = format_ident!("{}", bindings.napi_error_ext_trait);

        let methods = driver::scoped_client_methods(self, service);
        let mod_path = service.models_path();

        let tokens = quote! {
            #![allow(unused_mut, unused_imports, dead_code, clippy::all)]
            use std::collections::HashMap;
            use napi::bindgen_prelude::Buffer;
            use napi_derive::napi;
            use prost::Message;
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
        // Only resource-scoped services have a per-service scoped module; resource-less services'
        // methods live on the root client.
        let service_modules = services.iter().filter(|s| s.is_resource_scoped()).map(|s| {
            let module_name = format_ident!("{}", s.plan.base_path);
            quote! { pub mod #module_name; }
        });

        let bindings = services
            .first()
            .and_then(|s| s.config.bindings.as_ref())
            .expect("bindings config required for node output");

        let aggregate_client_name = &bindings.aggregate_client_name;
        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let aggregate_client_ident = format_ident!("{}", aggregate_client_name);
        let napi_aggregate_client_ident = format_ident!("Napi{}", aggregate_client_name);
        let napi_error_ext_ident = format_ident!("{}", bindings.napi_error_ext_trait);

        let mut sorted_services = services.iter().collect_vec();
        sorted_services.sort_by(|a, b| a.plan.service_name.cmp(&b.plan.service_name));

        let mod_paths = sorted_services.iter().map(|s| {
            let mod_path = s.models_path();
            quote! { use #mod_path::*; }
        });

        // Only resource-scoped services expose a per-service scoped client to import.
        let codegen_imports = sorted_services
            .iter()
            .filter(|s| s.is_resource_scoped())
            .map(|s| {
                let mod_name = format_ident!("{}", s.plan.base_path);
                let client_name = format_ident!("Napi{}", s.client_type().to_string());
                quote! { use crate::codegen::#mod_name::#client_name; }
            });

        let methods = driver::root_client_methods(self, &sorted_services);
        let resource_accessor_methods = driver::resource_accessor_methods(self, &sorted_services);

        let tokens = quote! {
            #![allow(unused_mut, unused_imports, dead_code, clippy::all)]
            #(#service_modules)*

            use std::collections::HashMap;
            use futures::stream::TryStreamExt;
            use futures::StreamExt;
            use napi::bindgen_prelude::{Buffer, ReadableStream};
            use napi::Env;
            use napi_derive::napi;
            use prost::Message;
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
    /// NAPI-specific capability filter (drop params NAPI can't pass, except required message bodies,
    /// which cross as a `Buffer`). Path/`page_token` filtering already happened in `param_plan`.
    fn param_defs(&self, method: &MethodHandler<'_>, mode: BindingMode) -> Vec<TokenStream> {
        method
            .param_plan(mode)
            .into_iter()
            // Keep NAPI-native params and required message bodies (passed as a `Buffer`); drop other
            // unsupported params (e.g. optional message setters, which stay codegen-omitted for now).
            .filter(|p| is_napi_supported(p) || is_required_message_body(p))
            .map(|p| {
                let param_name = p.field_ident();
                if is_required_message_body(p) {
                    // Message bodies cross the NAPI boundary as serialized bytes (decoded below).
                    quote! { #param_name: napi::bindgen_prelude::Buffer }
                } else {
                    let rust_type = method.field_type(p.field_type(), RenderContext::NapiParameter);
                    quote! { #param_name: #rust_type }
                }
            })
            .collect()
    }

    fn client_call(&self, method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream {
        let (method_name, drop_path) = match mode {
            BindingMode::Scoped => (method.plan.resource_client_method(), true),
            BindingMode::Flat => (method.plan.base_method_ident(), false),
        };
        let args = method
            .required_parameters()
            .filter(|param| {
                !(drop_path && param.is_path_param())
                    && (is_napi_supported(param) || is_required_message_body(param))
            })
            .map(|param| {
                let param_name = param.field_ident();
                if is_required_message_body(param) {
                    // Decode the protobuf bytes received over the NAPI boundary back into the message.
                    let msg_ty = format_ident!(
                        "{}",
                        match &param.field_type().base_type {
                            BaseType::Message(n) | BaseType::OneOf(n) =>
                                crate::utils::extract_simple_type_name(n),
                            _ => unreachable!("is_required_message_body guarantees a message type"),
                        }
                    );
                    quote! {
                        <#msg_ty as prost::Message>::decode(#param_name.as_ref())
                            .map_err(|e| napi::Error::new(napi::Status::GenericFailure, format!("invalid {} payload: {e}", stringify!(#msg_ty))))?
                    }
                } else if matches!(param.field_type().base_type, BaseType::Enum(..)) {
                    quote! { #param_name.try_into().map_err(|_| napi::Error::new(napi::Status::GenericFailure, "invalid enum value"))? }
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
                let mut request = #client_call;
                #(#builder_calls)*
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
                let mut request = #client_call;
                #(#builder_calls)*
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
        self.emit_unary(parts, self.unary_method_name(parts))
    }

    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method_name = self.unary_method_name(parts);
        let param_defs = &parts.param_defs;
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(parts.method, false, self);

        quote! {
            #[napi(catch_unwind)]
            pub async fn #method_name(
                &self,
                #(#param_defs,)*
            ) -> napi::Result<()> {
                let mut request = #client_call;
                #(#builder_calls)*
                request
                    .await
                    .default_error()
            }
        }
    }

    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream> {
        let spec = service.accessor_spec()?;
        let method_name = format_ident!("{}", spec.singular);
        let client_name = format_ident!("Napi{}", service.client_type().to_string());

        let param_idents: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
        let param_list = param_idents
            .iter()
            .map(|id| quote! { #id: String })
            .collect::<Vec<_>>();

        // A nested resource has a composite `full_name`; the underlying Rust client accessor takes a
        // single `full_name` string, so join the decomposed params with `.` and call
        // `{method}_from_full_name`.
        let method_call = if spec.nested {
            let from_full_name_method = format_ident!("{}_from_full_name", spec.singular);
            let format_str = spec.join_format();
            quote! {
                #[napi]
                pub fn #method_name(&self, #(#param_list),*) -> #client_name {
                    let full_name = format!(#format_str, #(#param_idents),*);
                    #client_name {
                        client: self.client.#from_full_name_method(full_name),
                    }
                }
            }
        } else {
            let param_refs = param_idents
                .iter()
                .map(|id| quote! { #id })
                .collect::<Vec<_>>();
            quote! {
                #[napi]
                pub fn #method_name(&self, #(#param_list),*) -> #client_name {
                    #client_name {
                        client: self.client.#method_name(#(#param_refs),*),
                    }
                }
            }
        };

        Some(method_call)
    }
}

impl NapiBackend {
    /// Scoped methods use the resource-client method name; flat methods use the base name.
    fn unary_method_name(&self, parts: &ShapeParts<'_>) -> Ident {
        match parts.mode {
            BindingMode::Scoped => parts.method.plan.resource_client_method(),
            BindingMode::Flat => parts.method.binding_method_name(),
        }
    }

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
                    let mut request = #client_call;
                    #(#builder_calls)*
                    request
                        .await
                        .map(|item| Buffer::from(item.encode_to_vec()))
                        .default_error()
                }
            }
        } else {
            // Custom POST/PATCH RPCs that target a resource can return `Empty` (e.g. a
            // delta-commit-style `Commit`). Emit a `()` return rather than a `Buffer` of an
            // empty message.
            quote! {
                #[napi(catch_unwind)]
                pub async fn #method_name(
                    &self,
                    #(#param_defs,)*
                ) -> napi::Result<()> {
                    let mut request = #client_call;
                    #(#builder_calls)*
                    request
                        .await
                        .default_error()
                }
            }
        }
    }
}

impl SetterRender for NapiBackend {
    fn supports(&self, setter: &OptionalSetter<'_>) -> bool {
        is_napi_supported_type(&setter.field_type.base_type)
    }

    fn render_setter(
        &self,
        setter: &OptionalSetter<'_>,
        param_ident: &Ident,
        with_method: &Ident,
    ) -> TokenStream {
        if matches!(setter.field_type.base_type, BaseType::Map(_, _))
            || setter.field_type.is_repeated
        {
            // Maps and repeated fields arrive `Option`-wrapped; guard before forwarding.
            quote! {
                if let Some(#param_ident) = #param_ident {
                    request = request.#with_method(#param_ident);
                }
            }
        } else if matches!(setter.field_type.base_type, BaseType::Enum(_)) {
            // Enums cross as `Option<i32>`; convert and drop invalid values.
            quote! {
                request = request.#with_method(
                    #param_ident.map(|v| v.try_into().ok()).flatten()
                );
            }
        } else {
            quote! {
                request = request.#with_method(#param_ident);
            }
        }
    }
}
