//! Low-level **ConnectRPC** client generation.
//!
//! This is the Connect-protocol counterpart of [`super::client`] (the HTTP/JSON client). It is part
//! of the **Generation** stage of the Analysis → Planning → Generation → Output pipeline (see
//! [`super`]), selected when [`ClientProtocol::Connect`](crate::codegen::ClientProtocol) is set.
//!
//! Rather than dispatch ConnectRPC itself, it emits a thin adapter over the **connect-rust** (BSR
//! plugin) generated `{Service}Client<T>` — the type that already owns `connectrpc::client::call_unary`
//! dispatch. The adapter is generic over the transport `T` (defaulting to
//! [`olai_http::connectrpc::CloudTransport`]) and exposes, per RPC, an
//! `async fn <method>(&self, request: &Req) -> Result<Resp>` whose signature matches what the shared
//! builder layer in [`super::builder`] already calls. So the same builders, `with_*` setters, and
//! `IntoFuture` impls are reused unchanged; only the low-level client differs from REST.
//!
//! Each generated method clones the borrowed request (the connect-rust client takes it by value,
//! matching how a hand-written wrapper passes owned protos), awaits the connect-rust call, maps
//! `connectrpc::ConnectError` into the project's error type, and `.into_owned()`s the returned
//! `UnaryResponse<OwnedView<…>>` into the owned response message.

use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::quote;

use super::{doc_tokens, format_tokens};
use crate::Result;
use crate::codegen::{MethodHandler, ServiceHandler};

/// Generate the ConnectRPC adapter client for a service.
pub(crate) fn generate(service: &ServiceHandler<'_>) -> Result<String> {
    let wrapper_ident = service.low_level_client_type(crate::codegen::ClientProtocol::Connect);
    let inner_path = service.connect_inner_client_path()?;
    let mod_path = service.models_path();
    let result_path: syn::Path =
        syn::parse_str(&service.config.result_type_path).expect("valid result_type_path");

    let method_tokens: Vec<TokenStream> = service.methods().map(connect_method).try_collect()?;

    let tokens = quote! {
        // The model glob is convenience-wide; a service that references only some model types
        // would otherwise trip `unused_imports` under `-D warnings`.
        #![allow(unused_imports)]
        use #result_path;
        use #mod_path::*;

        /// ConnectRPC client for service operations.
        ///
        /// A thin ergonomic adapter over the connect-rust generated service client. Generic over the
        /// ConnectRPC transport `T`, defaulting to [`olai_http::connectrpc::CloudTransport`] for
        /// cloud-authenticated server-side use.
        #[derive(Clone)]
        pub struct #wrapper_ident<T = ::olai_http::connectrpc::CloudTransport>
        where
            T: ::connectrpc::client::ClientTransport,
        {
            pub(crate) inner: #inner_path<T>,
        }

        impl<T> #wrapper_ident<T>
        where
            T: ::connectrpc::client::ClientTransport,
            <T::ResponseBody as ::http_body::Body>::Error: ::std::fmt::Display,
        {
            /// Create a new client over a connect-rust generated service client.
            pub fn new(inner: #inner_path<T>) -> Self {
                Self { inner }
            }

            /// Create a new client from a transport and ConnectRPC client configuration.
            pub fn from_transport(transport: T, config: ::connectrpc::client::ClientConfig) -> Self {
                Self { inner: #inner_path::new(transport, config) }
            }

            #(#method_tokens)*
        }
    };

    format_tokens(tokens)
}

/// Generate one adapter method delegating to the connect-rust client.
fn connect_method(method: MethodHandler<'_>) -> Result<TokenStream> {
    let doc_attrs = doc_tokens(method.plan.metadata.documentation.as_deref());
    let method_name = method.plan.base_method_ident();
    let input_type_ident = method.input_type().ok_or_else(|| {
        crate::Error::Build(format!(
            "Connect method `{}` has no input type (Empty input is unsupported)",
            method.plan.handler_function_name
        ))
    })?;

    // The connect-rust client takes the request by value and returns a `UnaryResponse` whose
    // `.into_owned()` yields the owned response message. We accept `&Req` (matching the builder's
    // `client.method(&request)` call) and clone before forwarding.
    if let Some(output_type) = method.output_type() {
        Ok(quote! {
            #doc_attrs
            pub async fn #method_name(&self, request: &#input_type_ident) -> Result<#output_type> {
                let response = self
                    .inner
                    .#method_name(request.clone())
                    .await
                    .map_err(crate::error::from_connect_error)?;
                Ok(response.into_owned())
            }
        })
    } else {
        Ok(quote! {
            #doc_attrs
            pub async fn #method_name(&self, request: &#input_type_ident) -> Result<()> {
                self
                    .inner
                    .#method_name(request.clone())
                    .await
                    .map_err(crate::error::from_connect_error)?;
                Ok(())
            }
        })
    }
}
