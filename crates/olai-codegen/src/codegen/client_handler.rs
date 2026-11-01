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
        .map(|m| forward_method(&m, &client_ident, &handlers_crate))
        .collect::<Result<Vec<_>>>()?;

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
///
/// An existence check (`HEAD` without an output) resolves to `bool` on the client but to `()` in
/// the trait: `false` becomes the upstream 404 again, as an `ApiError` of the generated error
/// module, so the handler error keeps telling a missing resource apart.
fn forward_method(
    method: &MethodHandler<'_>,
    client_ident: &syn::Ident,
    handlers_crate: &syn::Path,
) -> Result<TokenStream> {
    let method_name = method.plan.base_method_ident();
    let input_type = method.input_type();
    let output_type = method.handler_output_type();
    let not_found = if method.is_existence_check() {
        if method.config.rest_client.error_module.is_none() {
            return Err(crate::Error::Build(format!(
                "`RestClientOptions::handler_impls` forwards the existence check `{method_name}` \
                 as a 404 `ApiError`, which needs `RestClientOptions::error_module`; or `skip` \
                 its service and write the impl by hand"
            )));
        }
        let error_module = method.config.client_error_module();
        quote! {
            .and_then(|found| {
                if found {
                    Ok(())
                } else {
                    Err(#error_module::Error::Api(#error_module::ApiError {
                        status: 404,
                        code: None,
                        message: "not found".to_string(),
                        details: ::std::vec::Vec::new(),
                        body: ::std::string::String::new(),
                    }))
                }
            })
        }
    } else {
        quote! {}
    };
    // The inherent method shares the trait method's name; the explicit path selects it.
    Ok(quote! {
        async fn #method_name(
            &self,
            request: #input_type,
            _context: Cx,
        ) -> #handlers_crate::Result<#output_type> {
            #client_ident::#method_name(self, &request)
                .await
                #not_found
                .map_err(::core::convert::Into::into)
        }
    })
}

#[cfg(test)]
mod tests {
    use protobuf::Message;
    use protobuf::descriptor::FileDescriptorSet;

    use crate::codegen::{
        ClientHandlerImpls, CodeGenConfig, CodeGenOutput, RestClientOptions, Runtime, generate_code,
    };
    use crate::google::api::http_rule::Pattern;
    use crate::google::api::{CustomHttpPattern, HttpRule};
    use crate::parsing::{CodeGenMetadata, parse_file_descriptor_set};

    /// The example fixture with `TouchTagAssignment` (an `Empty` output) turned into a `HEAD`
    /// existence check.
    fn metadata_with_existence_check() -> CodeGenMetadata {
        let bytes = include_bytes!("../../proto/example.bin");
        let fds = FileDescriptorSet::parse_from_bytes(bytes).unwrap();
        let mut metadata = parse_file_descriptor_set(&fds).unwrap();
        let method = metadata
            .services
            .values_mut()
            .flat_map(|s| &mut s.methods)
            .find(|m| m.method_name == "TouchTagAssignment")
            .expect("fixture has TouchTagAssignment");
        method.http_rule = HttpRule {
            pattern: Some(Pattern::Custom(CustomHttpPattern {
                kind: "HEAD".into(),
                path: "/entities/{entity_type}/{entity_name}/tags/{tag_key}:touch".into(),
            })),
            ..Default::default()
        };
        metadata
    }

    fn config(client: &std::path::Path, error_module: bool) -> CodeGenConfig {
        CodeGenConfig {
            context_type_path: "crate::Context".into(),
            result_type_path: "crate::Result".into(),
            models_path_template: "example_common::models::{service}::v1".into(),
            models_path_crate_template: "crate::models::{service}::v1".into(),
            resource_store_crate_name: "olai_store".into(),
            runtime: Runtime::Prost,
            transport_type_path: crate::codegen::DEFAULT_TRANSPORT_TYPE_PATH.into(),
            dual_transport: false,
            client_protocols: Default::default(),
            connect_client_path: None,
            rest_client: RestClientOptions {
                error_module: error_module.then(Default::default),
                handler_impls: Some(ClientHandlerImpls {
                    crate_path: "example_handlers".into(),
                    feature: "handlers".into(),
                    skip: Vec::new(),
                }),
                ..Default::default()
            },
            output: CodeGenOutput {
                common: None,
                models: None,
                models_subdir: "_gen".into(),
                server: None,
                client: Some(client.to_path_buf()),
                python: None,
                node: None,
                node_ts: None,
                wasm: None,
                generate_resource_clients: false,
            },
            generate_resource_enum: false,
            generate_store_integration: false,
            error_type_path: Some("crate::Error".into()),
            generate_object_conversions: false,
            bindings: None,
        }
    }

    #[test]
    fn existence_check_forwards_false_as_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        generate_code(&metadata_with_existence_check(), &config(tmp.path(), true)).unwrap();

        let handler =
            std::fs::read_to_string(tmp.path().join("tag_assignments/handler.rs")).unwrap();
        syn::parse_file(&handler).unwrap();
        assert!(
            handler.contains("-> example_handlers::Result<()>") && handler.contains("status: 404"),
            "the existence check must map `false` to a 404 `ApiError`:\n{handler}"
        );
    }

    #[test]
    fn existence_check_needs_the_generated_error_module() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = generate_code(&metadata_with_existence_check(), &config(tmp.path(), false))
            .unwrap_err();
        assert!(err.to_string().contains("touch_tag_assignment"), "{err}");
    }
}
