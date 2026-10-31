//! Handler-trait impls for the generated per-service REST clients.
//!
//! The REST client and the server handler trait are generated from the same routed methods, so
//! each client method has a handler counterpart with the same request and response types. This
//! module emits, per service, an `impl <Handler><Cx> for <ServiceClient>` that forwards every call
//! to the client: a client then serves as the handler for its service, backed by an upstream
//! server. The request context is ignored; client errors convert into the handler crate's error
//! via `Into`. Enabled by [`RestClientOptions::handler_impls`](super::RestClientOptions).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{ClientHandlerImpls, ClientProtocol, MethodHandler, ServiceHandler, format_tokens};
use crate::Result;

/// Generate the `handler.rs` impl module for a service.
pub(super) fn generate(service: &ServiceHandler<'_>, impls: &ClientHandlerImpls) -> Result<String> {
    let handlers_crate: syn::Path =
        syn::parse_str(&impls.crate_path).map_err(|source| crate::Error::InvalidRustPath {
            path: impls.crate_path.clone(),
            message: "client handler_impls crate".to_string(),
            source,
        })?;
    let module = format_ident!("{}", service.plan.base_path);
    let trait_ident = format_ident!("{}", service.plan.handler_name);
    let client_ident = service.low_level_client_type(ClientProtocol::Rest);
    let mod_path = service.models_path();
    let methods = service
        .rest_methods()
        .map(|m| forward_method(&m, &client_ident, &handlers_crate));

    let tokens = quote! {
        #![allow(unused_imports)]
        use #mod_path::*;
        use super::client::#client_ident;

        #[::async_trait::async_trait]
        impl<Cx: Send + 'static> #handlers_crate::#module::#trait_ident<Cx> for #client_ident {
            #(#methods)*
        }
    };
    format_tokens(tokens)
}

/// One trait method forwarding to the inherent client method of the same name.
fn forward_method(
    method: &MethodHandler<'_>,
    client_ident: &syn::Ident,
    handlers_crate: &syn::Path,
) -> TokenStream {
    let method_name = method.plan.base_method_ident();
    let input_type = method.input_type();
    // Same rule as the trait method signature in `handler.rs`.
    let output_type = if method.plan.has_response {
        let output = method.output_type();
        quote! { #output }
    } else {
        quote! { () }
    };
    // The inherent method shares the trait method's name; the explicit path selects it.
    quote! {
        async fn #method_name(
            &self,
            request: #input_type,
            _context: Cx,
        ) -> #handlers_crate::Result<#output_type> {
            #client_ident::#method_name(self, &request)
                .await
                .map_err(::core::convert::Into::into)
        }
    }
}
