//! Low-level HTTP client generation.
//!
//! This is part of the **Generation** stage of the Analysis → Planning → Generation → Output
//! pipeline (see [`super`]). For each service it emits a low-level client struct holding a
//! transport client plus a base [`Url`](url::Url), with one async method per RPC that builds the
//! request URL, serializes the body, and dispatches through the configured transport. The client
//! is written to each service's `client.rs` and is the foundation that the request builders in
//! [`super::builder`] and the ergonomic resource clients in [`super::resource_client`] build upon.
//!
//! The transport type is configurable (`transport_type_path`): the default cloud transport and the
//! WASM browser transport are both selected here. URL routing, parameter sources, and body shape
//! all come from the per-method [`GenerationPlan`](crate::analysis::GenerationPlan) produced during
//! Planning; the emitted token stream is pretty-printed via `super::format_tokens`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{doc_tokens, extract_type_ident, format_tokens};
use crate::Result;
use crate::codegen::{MethodHandler, Runtime, ServiceHandler};
use crate::parsing::types::BaseType;

/// Generate client code for a service
pub(crate) fn generate(service: &ServiceHandler<'_>) -> Result<String> {
    let mut method_tokens = Vec::new();

    for method in service.rest_methods() {
        let full_name = format!(
            "{}.{}.{}",
            service.plan.package, service.plan.service_name, method.plan.metadata.method_name
        );
        let options = &service.config.rest_client;
        let form_body = options.form_encoded_methods.contains(&full_name);
        let path_fields_in_body = options.path_fields_in_body_methods.contains(&full_name);
        let method_code = client_method(method, form_body, path_fields_in_body)?;
        method_tokens.push(method_code);
    }

    let client_ident = service.low_level_client_type(crate::codegen::ClientProtocol::Rest);
    let mod_path = service.models_path();
    let foreign_models = service.foreign_models_imports();
    let result_path = service.config.client_result_path();
    let (transport_import, transport_ident) = transport_tokens(service.config);
    // `encode_path_value` lives in the client root `mod.rs`.
    let encode_import = service
        .rest_methods()
        .any(|m| m.plan.formats_path_values())
        .then(|| {
            if service.config.client_protocols.is_multi() {
                quote! { use super::super::super::encode_path_value; }
            } else {
                quote! { use super::super::encode_path_value; }
            }
        });

    let tokens = quote! {
        // `use #mod_path::*` is a convenience-wide model glob; a service that
        // references only some model types (common under buffa) would otherwise
        // trip `unused_imports` under `-D warnings`.
        #![allow(unused_imports)]
        #transport_import
        use url::Url;
        use #result_path;
        use #mod_path::*;
        #(#foreign_models)*
        #encode_import

        /// HTTP client for service operations
        #[derive(Clone)]
        pub struct #client_ident {
            pub(crate) client: #transport_ident,
            pub(crate) base_url: Url,
        }

        impl #client_ident {
            /// Create a new client instance
            pub fn new(client: #transport_ident, mut base_url: Url) -> Self {
                if !base_url.path().ends_with('/') {
                    base_url.set_path(&format!("{}/", base_url.path()));
                }
                Self { client, base_url }
            }

            #(#method_tokens)*
        }
    };

    format_tokens(tokens)
}

/// The transport import + the identifier generated client code refers to it by.
///
/// - **Single transport** (default): emit `use <transport_type_path>;` and refer
///   to the transport by its final segment (`CloudClient`), keeping native-only
///   output simple.
/// - **Dual transport** ([`CodeGenConfig::dual_transport`], i.e. WASM output is
///   enabled): emit a `cfg(target_arch = "wasm32")`-gated alias so the one client
///   crate builds natively against `CloudClient` and for the browser against
///   `olai_http_wasm::WasmClient`, and refer to it by the alias `Transport`.
pub(crate) fn transport_tokens(
    config: &crate::codegen::CodeGenConfig,
) -> (TokenStream, proc_macro2::Ident) {
    if config.dual_transport() {
        let import = quote! {
            #[cfg(not(target_arch = "wasm32"))]
            use ::olai_http::CloudClient as Transport;
            #[cfg(target_arch = "wasm32")]
            use ::olai_http_wasm::WasmClient as Transport;
        };
        (import, format_ident!("Transport"))
    } else {
        let transport_path: syn::Path =
            syn::parse_str(&config.transport_type_path).expect("valid transport_type_path");
        let ident = transport_path
            .segments
            .last()
            .expect("transport_type_path has at least one segment")
            .ident
            .clone();
        (quote! { use #transport_path; }, ident)
    }
}

/// Generate a client method. `form_body` sends the request body form-encoded instead of as
/// JSON; `path_fields_in_body` keeps the path fields in the body, which `google.api.http`
/// otherwise leaves out.
pub fn client_method(
    method: MethodHandler<'_>,
    form_body: bool,
    path_fields_in_body: bool,
) -> Result<TokenStream> {
    let doc_attrs = doc_tokens(method.plan.metadata.documentation.as_deref());
    let method_name = method.plan.base_method_ident();
    let input_type_ident = method.input_type();
    let http_method = format_ident!("{}", method.plan.http_method.to_lowercase());
    let has_query_params = method.plan.query_parameters().next().is_some();
    let url_formatting = generate_url_formatting(&method, has_query_params);
    let query_handling = generate_query_parameters(&method);
    let error_module = method.config.client_error_module();
    let result_type = method.result_type();
    let request_ident =
        if method.plan.formats_path_values() || has_query_params || method.plan.has_request_body {
            format_ident!("request")
        } else {
            format_ident!("_request")
        };

    // Path fields go in the URL, not the body: send a copy with them reset to their defaults.
    let path_fields: Vec<_> = if method.plan.has_request_body
        && method.plan.formats_path_values()
        && !path_fields_in_body
    {
        method
            .plan
            .path_parameters()
            .map(|param| crate::utils::field_ident(&param.name))
            .collect()
    } else {
        Vec::new()
    };
    let (body_copy, body) = if path_fields.is_empty() {
        (quote! {}, quote! { request })
    } else {
        let copy = quote! {
            let mut body = request.clone();
            #(body.#path_fields = Default::default();)*
        };
        (copy, quote! { &body })
    };
    let body_handling = match (method.plan.has_request_body, form_body) {
        (true, false) => quote! { .json(#body) },
        (true, true) => quote! { .form(#body) },
        (false, _) => quote! {},
    };

    let send = quote! { let response = self.client.#http_method(url)#body_handling.send().await?; };
    let read_response = match method.output_type() {
        Some(_) => read_output(&method),
        None if method.is_existence_check() => quote! { Ok(true) },
        None => quote! { Ok(()) },
    };
    // An existence check returns `false` on 404.
    let not_found = method.is_existence_check().then(|| {
        quote! {
            if response.status().as_u16() == 404 {
                return Ok(false);
            }
        }
    });

    Ok(quote! {
        #doc_attrs
        pub async fn #method_name(&self, #request_ident: &#input_type_ident) -> Result<#result_type> {
            #url_formatting
            #query_handling
            #body_copy
            #send
            #not_found
            if !response.status().is_success() {
                return Err(#error_module::parse_error_response(response).await);
            }
            #read_response
        }
    })
}

/// The statements that turn a successful `response` into the method's output.
fn read_output(method: &MethodHandler<'_>) -> TokenStream {
    let lenient = method.config.rest_client.ignore_unknown_enum_values
        && method.config.runtime == Runtime::Buffa;
    let options = lenient.then(|| {
        quote! { let options = ::buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(true); }
    });
    let from_slice = |bytes: TokenStream| {
        if lenient {
            quote! { ::buffa::json::with_json_parse_options(&options, || serde_json::from_slice(#bytes)) }
        } else {
            quote! { serde_json::from_slice(#bytes) }
        }
    };

    let parse = from_slice(quote! { &result });
    quote! {
        let result = response.bytes().await?;
        #options
        Ok(#parse?)
    }
}

/// Generate URL formatting code that properly substitutes path parameters.
///
/// `needs_mut` controls whether the generated `url` binding is declared `mut`.
/// Pass `true` only when query parameters will be appended via `url.query_pairs_mut()`.
fn generate_url_formatting(
    method: &MethodHandler<'_>,
    needs_mut: bool,
) -> proc_macro2::TokenStream {
    let path = method.plan.http_pattern.base_path();
    let path = path.trim_start_matches('/');

    // Only the binding's mutability differs between the two callers.
    let mut_kw = if needs_mut {
        quote! { mut }
    } else {
        quote! {}
    };

    // No path values -> direct join; otherwise format then join.
    if !method.plan.formats_path_values() {
        quote! {
            let #mut_kw url = self.base_url.join(#path)?;
        }
    } else {
        let (format_string, format_args) = method.plan.http_pattern.to_format_string();
        let values: Vec<TokenStream> = format_args
            .iter()
            .map(|template_param| {
                let field_ident = crate::utils::field_ident(template_param);
                let keep_slash = method.plan.http_pattern.is_multi_segment(template_param);
                quote! { encode_path_value(&request.#field_ident, #keep_slash) }
            })
            .collect();
        quote! {
            let formatted_path = format!(#format_string, #(#values),*);
            let #mut_kw url = self.base_url.join(&formatted_path)?;
        }
    }
}

/// The `encode_path_value` helper of the client `mod.rs`: percent-encoding per gRPC transcoding,
/// with `/` kept only for multi-segment patterns.
pub(crate) fn encode_path_value_fn() -> TokenStream {
    quote! {
        /// Percent-encode a URL path value. `keep_slash` keeps `/` (for a multi-segment value)
        /// and drops a leading one.
        fn encode_path_value(value: &impl ::std::fmt::Display, keep_slash: bool) -> String {
            use ::std::fmt::Write as _;
            let value = value.to_string();
            let value = match keep_slash {
                true => value.trim_start_matches('/'),
                false => value.as_str(),
            };
            let mut out = String::with_capacity(value.len());
            for byte in value.bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                        out.push(byte as char)
                    }
                    b'/' if keep_slash => out.push('/'),
                    _ => {
                        let _ = write!(out, "%{byte:02X}");
                    }
                }
            }
            out
        }
    }
}

/// Generate query parameter handling code
fn generate_query_parameters(method: &MethodHandler<'_>) -> proc_macro2::TokenStream {
    let mut param_assignments = Vec::new();
    for param in method.plan.query_parameters() {
        let field_ident = crate::utils::field_ident(&param.name);
        let param_name = method.query_key(param);
        let field_type = &param.field_type;

        if field_type.is_repeated {
            if let BaseType::Enum(enum_name) = &field_type.base_type {
                let enum_type_ident = extract_type_ident(enum_name);
                param_assignments.push(match method.config.runtime {
                    // prost: the field is `Vec<i32>`; recover the typed variant via `from_i32`.
                    Runtime::Prost => quote! {
                        for &raw in &request.#field_ident {
                            if let Some(v) = #enum_type_ident::from_i32(raw) {
                                url.query_pairs_mut().append_pair(#param_name, v.as_str_name());
                            }
                        }
                    },
                    // buffa: the field is `Vec<EnumValue<E>>`; emit only known variants by name.
                    Runtime::Buffa => quote! {
                        for v in &request.#field_ident {
                            if let Some(known) = v.as_known() {
                                use buffa::Enumeration as _;
                                url.query_pairs_mut().append_pair(#param_name, known.proto_name());
                            }
                        }
                    },
                });
            } else {
                param_assignments.push(quote! {
                    for value in &request.#field_ident {
                        url.query_pairs_mut().append_pair(#param_name, &value.to_string());
                    }
                });
            }
        } else if matches!(method.config.runtime, Runtime::Buffa)
            && matches!(field_type.base_type, BaseType::Enum(_))
        {
            // buffa singular enum: the field is `EnumValue<E>` (or `Option<EnumValue<E>>`), which
            // has no `Display`. Emit the known variant by its proto name; skip unknown/absent.
            if param.is_optional() {
                param_assignments.push(quote! {
                    if let Some(known) = request.#field_ident.and_then(|v| v.as_known()) {
                        use buffa::Enumeration as _;
                        url.query_pairs_mut().append_pair(#param_name, known.proto_name());
                    }
                });
            } else {
                param_assignments.push(quote! {
                    if let Some(known) = request.#field_ident.as_known() {
                        use buffa::Enumeration as _;
                        url.query_pairs_mut().append_pair(#param_name, known.proto_name());
                    }
                });
            }
        } else if param.is_optional() {
            param_assignments.push(quote! {
                if let Some(ref value) = request.#field_ident {
                    url.query_pairs_mut().append_pair(#param_name, &value.to_string());
                }
            });
        } else if matches!(field_type.base_type, BaseType::String) {
            // Required string param: always sent. Append the value directly (it is already a
            // `String`, so `.to_string()` would be redundant).
            param_assignments.push(quote! {
                url.query_pairs_mut().append_pair(#param_name, &request.#field_ident);
            });
        } else {
            // Required non-string scalar: always sent.
            param_assignments.push(quote! {
                url.query_pairs_mut().append_pair(#param_name, &request.#field_ident.to_string());
            });
        }
    }

    if param_assignments.is_empty() {
        return quote! {};
    }

    quote! {
        #(#param_assignments)*
    }
}
