//! `#[wasm_bindgen]` wrapper generation.
//!
//! For each service we emit a `#[wasm_bindgen]` struct wrapping the generated low-level client
//! (`crate::codegen::<base>::<Service>ServiceClient`), and an aggregate struct that constructs each
//! per-service wrapper on demand. Every RPC becomes an `async` method that:
//!   1. deserializes the request from a JS object (`serde_wasm_bindgen::from_value`),
//!   2. calls the inner client method,
//!   3. serializes the response back to a JS object.
//!
//! Request/response values are plain JS objects, so this requires serde-native models
//! (`runtime: Buffa`). Errors are surfaced to JS as `JsValue` (the project's error type stringified).

use convert_case::{Case, Casing};
use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::super::format_tokens;
use crate::codegen::ServiceHandler;

/// Generate `bindings.rs`: one `#[wasm_bindgen]` wrapper per service plus an aggregate root.
pub(crate) fn generate_bindings(services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
    let bindings = services
        .first()
        .and_then(|s| s.config.bindings.as_ref())
        .expect("bindings config required for wasm output");

    let aggregate_js_name = bindings.aggregate_client_name.clone();
    let wasm_aggregate_ident = format_ident!("Wasm{}", bindings.aggregate_client_name);

    // Bring every service's low-level client into scope (one module each, all distinct).
    let client_imports = services.iter().map(|s| {
        let module = format_ident!("{}", s.plan.base_path);
        quote! { use crate::codegen::#module::*; }
    });
    // Models are keyed by proto package, which several services can share — dedup the glob imports.
    let model_imports = services
        .iter()
        .map(|s| s.models_path_crate())
        .unique_by(|p| quote! { #p }.to_string())
        .map(|models| quote! { use #models::*; });

    let service_wrappers = services.iter().map(service_wrapper);
    let accessors = services.iter().map(|s| {
        let module = format_ident!("{}", s.plan.base_path);
        let low_level = s.low_level_client_type(crate::codegen::ClientProtocol::Rest);
        let wasm_wrapper = format_ident!("Wasm{}", low_level);
        let accessor = format_ident!("{}", s.plan.base_path);
        let js_accessor = s.plan.base_path.to_case(Case::Camel);
        let doc = format!("Access the `{}` service.", s.plan.base_path);
        quote! {
            #[doc = #doc]
            #[wasm_bindgen(js_name = #js_accessor)]
            pub fn #accessor(&self) -> #wasm_wrapper {
                #wasm_wrapper {
                    inner: crate::codegen::#module::#low_level::new(
                        self.client.clone(),
                        self.base_url.clone(),
                    ),
                }
            }
        }
    });

    let tokens = quote! {
        // The whole bindings module is browser-only: it pulls in `wasm-bindgen`
        // and `olai-http-wasm` (both wasm32-target deps). Gating the file means a
        // native `cargo build` of the same client crate skips it entirely.
        #![cfg(target_arch = "wasm32")]
        #![allow(unused_mut, unused_imports, dead_code, clippy::all)]

        use wasm_bindgen::prelude::*;
        use olai_http_wasm::{CredentialsMode, WasmClient};
        use olai_http_wasm::reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
        use url::Url;
        #(#client_imports)*
        #(#model_imports)*

        #(#service_wrappers)*

        /// Optional client options, passed as the second constructor argument from JS.
        ///
        /// All fields are optional; the no-options / base-URL-only call keeps the default
        /// browser-session (cookie) behavior.
        #[derive(Default, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ClientOptions {
            /// Bearer token. When set, every request carries `Authorization: Bearer <token>`.
            #[serde(default)]
            auth_token: Option<String>,
            /// `fetch` credentials mode: `"include"` (default), `"same-origin"`, or `"omit"`.
            #[serde(default)]
            credentials: Option<String>,
        }

        /// Browser entry point: construct from a base URL, then access per-service clients.
        ///
        /// By default the browser manages the session (cookies / auth headers ride along with
        /// `fetch`). Pass an optional options object to use bearer-token auth instead and/or to
        /// choose the `fetch` credentials mode.
        #[wasm_bindgen(js_name = #aggregate_js_name)]
        pub struct #wasm_aggregate_ident {
            client: WasmClient,
            base_url: Url,
        }

        #[wasm_bindgen(js_class = #aggregate_js_name)]
        impl #wasm_aggregate_ident {
            /// Construct the client.
            ///
            /// `options` is optional: omit it (or pass `undefined`/`null`) to keep the
            /// browser-session cookie behavior. Supported fields:
            /// `{ authToken?: string, credentials?: "include" | "same-origin" | "omit" }`.
            #[wasm_bindgen(constructor)]
            pub fn new(base_url: String, options: Option<JsValue>) -> Result<#wasm_aggregate_ident, JsValue> {
                let mut base_url = Url::parse(&base_url)
                    .map_err(|e| JsValue::from_str(&format!("invalid base_url: {e}")))?;
                if !base_url.path().ends_with('/') {
                    base_url.set_path(&format!("{}/", base_url.path()));
                }

                // Deserialize options, treating undefined/null as "no options".
                let options: ClientOptions = match options {
                    Some(v) if !v.is_undefined() && !v.is_null() => {
                        serde_wasm_bindgen::from_value(v)
                            .map_err(|e| JsValue::from_str(&format!("invalid options: {e}")))?
                    }
                    _ => ClientOptions::default(),
                };

                let mut client = WasmClient::new();
                if let Some(credentials) = options.credentials.as_deref() {
                    let mode = match credentials {
                        "include" => CredentialsMode::Include,
                        "same-origin" => CredentialsMode::SameOrigin,
                        "omit" => CredentialsMode::Omit,
                        other => return Err(JsValue::from_str(&format!(
                            "invalid credentials mode: {other:?} (expected \"include\", \"same-origin\", or \"omit\")"
                        ))),
                    };
                    client = client.with_credentials(mode);
                } else if options.auth_token.is_some() {
                    // A bearer token without an explicit credentials choice defaults to omitting
                    // the browser session so a stale cookie can't shadow the Authorization header.
                    client = client.with_credentials(CredentialsMode::Omit);
                }
                if let Some(token) = options.auth_token {
                    let value = HeaderValue::from_str(&format!("Bearer {token}"))
                        .map_err(|e| JsValue::from_str(&format!("invalid authToken: {e}")))?;
                    client = client.with_auth(move || {
                        let mut headers = HeaderMap::new();
                        headers.insert(AUTHORIZATION, value.clone());
                        headers
                    });
                }

                Ok(Self { client, base_url })
            }

            #(#accessors)*
        }
    };

    format_tokens(tokens)
}

/// Emit one `#[wasm_bindgen]` wrapper struct + impl for a single service's low-level client.
fn service_wrapper(service: &ServiceHandler<'_>) -> TokenStream {
    let low_level = service.low_level_client_type(crate::codegen::ClientProtocol::Rest);
    let wasm_wrapper = format_ident!("Wasm{}", low_level);

    let methods = service.methods().map(|method| {
        let inner_call = method.plan.base_method_ident();
        // Rust fn keeps the snake_case binding name; JS sees the camelCase name via `js_name`.
        let rust_name = method.binding_method_name();
        let js_name = method.binding_method_name_str().to_case(Case::Camel);
        let has_body = method.plan.has_request_body || method.input_type().is_some();

        // Request: deserialize a JS object into the typed request, or default for no-input RPCs.
        let (req_param, req_build) = if let Some(input_ty) = method.input_type() {
            (
                quote! { request: JsValue },
                quote! {
                    let request: #input_ty = serde_wasm_bindgen::from_value(request)
                        .map_err(|e| JsValue::from_str(&format!("invalid request: {e}")))?;
                },
            )
        } else {
            let _ = has_body;
            (quote! {}, quote! { let request = Default::default(); })
        };

        // Response: serialize the typed response to a JS object, or return undefined for `Empty`.
        if method.output_type().is_some() {
            quote! {
                #[wasm_bindgen(js_name = #js_name)]
                pub async fn #rust_name(&self, #req_param) -> Result<JsValue, JsValue> {
                    #req_build
                    let response = self.inner.#inner_call(&request).await
                        .map_err(|e| JsValue::from_str(&e.to_string()))?;
                    serde_wasm_bindgen::to_value(&response)
                        .map_err(|e| JsValue::from_str(&format!("invalid response: {e}")))
                }
            }
        } else {
            quote! {
                #[wasm_bindgen(js_name = #js_name)]
                pub async fn #rust_name(&self, #req_param) -> Result<(), JsValue> {
                    #req_build
                    self.inner.#inner_call(&request).await
                        .map_err(|e| JsValue::from_str(&e.to_string()))?;
                    Ok(())
                }
            }
        }
    });

    let doc = format!(
        "WASM/browser binding for the `{}` service.",
        service.plan.base_path
    );
    quote! {
        #[doc = #doc]
        #[wasm_bindgen]
        pub struct #wasm_wrapper {
            inner: #low_level,
        }

        #[wasm_bindgen]
        impl #wasm_wrapper {
            #(#methods)*
        }
    }
}
