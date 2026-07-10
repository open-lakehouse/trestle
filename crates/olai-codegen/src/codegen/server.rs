//! Axum server-side glue generation.
//!
//! This is part of the **Generation** stage of the Analysis → Planning → Generation → Output
//! pipeline (see [`super`]). It emits the wiring that connects incoming HTTP requests to the
//! handler trait produced by [`super::handler`]:
//!
//! - `axum::extract::FromRequestParts` extractors for path and query parameters and
//!   `axum::extract::FromRequest` extractors for the JSON body (the "common" output, always
//!   generated);
//! - per-method route handler functions that build the typed request, call the handler trait, and
//!   serialize the response (the "server" output).
//!
//! Which extractors and imports are emitted is driven by the per-method
//! [`GenerationPlan`](crate::analysis::GenerationPlan) computed during Planning (e.g. only services
//! with path/query params pull in `RequestPartsExt`). Emitted token streams are pretty-printed via
//! `super::format_tokens`.

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{Path, Type};

use super::format_tokens;
use crate::{
    analysis::{MethodPlan, RequestParam},
    codegen::{MethodHandler, Runtime, ServiceHandler},
    parsing::types::{BaseType, RenderContext},
};

/// Generate server side code for axum servers
///
/// This generates:
/// - FromRequestParts extractor implementations for path/query parameters
/// - FromRequest extractor implementations for JSON body
pub(super) fn generate_common(service: &ServiceHandler<'_>) -> crate::error::Result<String> {
    let extractor_impls = service
        .rest_methods()
        .map(|method| from_request_extractor(&method))
        .collect_vec();
    let mod_path = service.models_path_crate();

    // Only import RequestPartsExt when there are FromRequestParts impls (path/query params).
    let has_parts_extractors = service.rest_methods().any(|m| m.plan.needs_request_parts);

    let axum_imports = if has_parts_extractors {
        quote! { use axum::{RequestExt, RequestPartsExt}; }
    } else {
        quote! { use axum::RequestExt; }
    };

    // NB: these extractor impls return `Result<Self, Self::Rejection>` — the
    // 2-arg std `Result` from the prelude, NOT the app's 1-arg `result_type`
    // alias. This file lands in the models crate (`output_common`), which has no
    // `crate::api`, so importing the configured `result_type` here would be both
    // unresolvable and an arity mismatch. Do not import it.
    let tokens = quote! {
        // `unused_imports`: `RequestExt`/the model glob are only used by some
        // extractor shapes. `unused_mut`: the `mut req`/`mut parts` bindings
        // aren't always mutated.
        #![allow(unused_mut, unused_imports)]
        use #mod_path::*;
        #axum_imports

        #(#extractor_impls)*
    };

    format_tokens(tokens)
}

pub(super) fn generate_server(service: &ServiceHandler<'_>) -> crate::error::Result<String> {
    let handler_function_impls = service
        .rest_methods()
        .map(|method| axum_route_handler_impl(&method, &service.plan.handler_name))
        .collect_vec();

    let mod_path = service.models_path();
    // handler_name is a validated Rust identifier, so this parse is infallible.
    let trait_path: Path =
        syn::parse_str(&format!("super::handler::{}", service.plan.handler_name)).unwrap();
    let result_path: Path =
        syn::parse_str(&service.config.result_type_path).expect("valid result_type_path");

    let tokens = quote! {
        // Generated route fns: `too_many_arguments` for flat methods with many
        // path/query params; `unused_mut` for the `State`/extractor bindings.
        #![allow(unused_mut, clippy::too_many_arguments)]
        use #result_path;
        use #mod_path::*;
        use #trait_path;
        use axum::extract::State;

        #(#handler_function_impls)*

    };

    format_tokens(tokens)
}

/// Generate extractor implementation for a specific method.
///
/// Path/query-only methods use `FromRequestParts`; body-bearing methods use `FromRequest`. The
/// split is precomputed on the plan (`needs_request_parts`) so this no longer re-matches the
/// `RequestType` enum.
fn from_request_extractor(method: &MethodHandler<'_>) -> TokenStream {
    if method.plan.needs_request_parts {
        from_request_parts_impl(method)
    } else {
        from_request_impl(method)
    }
}

/// Generate route handler function
fn axum_route_handler_impl(method: &MethodHandler<'_>, handler_trait: &str) -> TokenStream {
    let handler_method = format_ident!("{}", method.plan.handler_function_name);
    let input_type = method.input_type();
    let handler_trait_ident = format_ident!("{}", handler_trait);

    if method.plan.has_response {
        let output_type = method.output_type();
        quote! {
            pub async fn #handler_method<T, Cx>(
                State(handler): State<T>,
                context: Cx,
                request: #input_type,
            ) -> Result<::axum::Json<#output_type>>
            where
                T: #handler_trait_ident<Cx> + Clone + Send + Sync + 'static,
                Cx: axum::extract::FromRequestParts<T> + Send,
            {
                let result = handler.#handler_method(request, context).await?;
                Ok(axum::Json(result))
            }
        }
    } else {
        quote! {
            pub async fn #handler_method<T, Cx>(
                State(handler): State<T>,
                context: Cx,
                request: #input_type,
            ) -> Result<()>
            where
                T: #handler_trait_ident<Cx> + Clone + Send + Sync + 'static,
                Cx: axum::extract::FromRequestParts<T> + Send,
            {
                handler.#handler_method(request, context).await?;
                Ok(())
            }
        }
    }
}

/// Generate FromRequestParts implementation for path/query parameters
fn from_request_parts_impl(method: &MethodHandler<'_>) -> TokenStream {
    let input_type = method.input_type();
    let path_extractions = path_extractions(method);
    let query_extractions = query_extractions(method);
    let field_assignments = field_assignments(method.plan, method.config.runtime);

    quote! {
        impl<S: Send + Sync> axum::extract::FromRequestParts<S> for #input_type {
            type Rejection = axum::response::Response;

            async fn from_request_parts(
                parts: &mut axum::http::request::Parts,
                _state: &S,
            ) -> Result<Self, Self::Rejection> {
                #path_extractions
                #query_extractions

                Ok(#input_type {
                    #field_assignments
                })
            }
        }
    }
}

/// Generate FromRequest implementation for JSON body
fn from_request_impl(method: &MethodHandler<'_>) -> TokenStream {
    let input_type = method.input_type();

    let is_hybrid = method
        .plan
        .parameters
        .iter()
        .any(|param| matches!(param, RequestParam::Path(_) | RequestParam::Query(_)));

    // Check if we need a hybrid extractor (path/query + body)
    if is_hybrid {
        // Generate hybrid implementation
        generate_hybrid_request_impl(method)
    } else {
        // Simple JSON body extraction
        quote! {
            impl<S: Send + Sync> axum::extract::FromRequest<S> for #input_type {
                type Rejection = axum::response::Response;

                async fn from_request(
                    req: axum::extract::Request<axum::body::Body>,
                    _state: &S,
                ) -> Result<Self, Self::Rejection> {
                    let axum::extract::Json(request) = req
                        .extract()
                        .await
                        .map_err(axum::response::IntoResponse::into_response)?;
                    Ok(request)
                }
            }
        }
    }
}

/// Generate hybrid FromRequest implementation for methods with path/query + body
fn generate_hybrid_request_impl(method: &MethodHandler<'_>) -> TokenStream {
    // Only reached when is_hybrid == true (caller checked path/query params exist),
    // which requires a non-Empty input message, so input_type() is always Some here.
    let input_type = method.input_type().unwrap();
    let path_extractions = path_extractions(method);
    let query_extractions = query_extractions(method);
    // Oneof fields deserialize from JSON like any other field, so no special treatment needed.
    let body_extractions = generate_body_extractions_tokens(method.plan, &input_type);
    let field_assignments = field_assignments(method.plan, method.config.runtime);

    // When the method has no body fields, the request body is unused — bind it to
    // `_body` so it doesn't trip `unused_variables`.
    let body_binding = if method.plan.body_fields().next().is_some() {
        quote! { body }
    } else {
        quote! { _body }
    };

    quote! {
        impl<S: Send + Sync> axum::extract::FromRequest<S> for #input_type {
            type Rejection = axum::response::Response;

            async fn from_request(
                mut req: axum::extract::Request<axum::body::Body>,
                _state: &S,
            ) -> Result<Self, Self::Rejection> {
                // Extract path and query parameters
                let (mut parts, #body_binding) = req.into_parts();
                #path_extractions
                #query_extractions

                // Extract body fields (only when the request has a body).
                #body_extractions

                Ok(#input_type {
                    #field_assignments
                })
            }
        }
    }
}

/// Generate body parameter extractions as TokenStream
fn generate_body_extractions_tokens(method: &MethodPlan, response_type: &Ident) -> TokenStream {
    let body_fields = method.body_fields().collect_vec();
    if body_fields.is_empty() {
        quote! {}
    } else {
        let field_names: Vec<_> = body_fields
            .iter()
            .map(|f| format_ident!("{}", f.name))
            .collect();
        // A single body field is a plain binding, not a 1-tuple destructure — `(x) = (y)`
        // trips `unused_parens`.
        let binding = if field_names.len() == 1 {
            let name = &field_names[0];
            quote! { let #name = body.#name; }
        } else {
            quote! { let (#(#field_names),*) = (#(body.#field_names),*); }
        };
        quote! {
            let body_req = axum::extract::Request::from_parts(parts, body);
            let axum::extract::Json::<#response_type>(body) = body_req
                .extract()
                .await
                .map_err(axum::response::IntoResponse::into_response)?;
            #binding
        }
    }
}

/// Generate path parameter extractions as TokenStream
fn path_extractions(method: &MethodHandler<'_>) -> TokenStream {
    let params = &method.plan.path_parameters().collect_vec();

    if params.is_empty() {
        quote! {}
    } else {
        let param_names: Vec<Ident> = params.iter().map(|p| format_ident!("{}", p.name)).collect();
        let param_types: Vec<Type> = params
            .iter()
            .map(|p| method.field_type(&p.field_type, RenderContext::Extractor))
            .collect();

        // A single path param is a scalar, not a 1-tuple: `Path(name)` / `Path<String>`.
        // Wrapping it in parens (`Path((name))`) trips the `unused_parens` lint.
        let (pat, ty) = if param_names.len() == 1 {
            let name = &param_names[0];
            let ty = &param_types[0];
            (quote! { #name }, quote! { #ty })
        } else {
            (
                quote! { (#(#param_names),*) },
                quote! { (#(#param_types),*) },
            )
        };

        quote! {
            let axum::extract::Path(#pat) = parts
                .extract::<axum::extract::Path<#ty>>()
                .await
                .map_err(axum::response::IntoResponse::into_response)?;
        }
    }
}

/// Generate query parameter extractions as TokenStream
fn query_extractions(method: &MethodHandler<'_>) -> TokenStream {
    let params = method.plan.query_parameters().collect_vec();
    if params.is_empty() {
        quote! {}
    } else {
        let query_fields = params.iter().map(|p| {
            let name = format_ident!("{}", p.name);
            // Use QueryExtractor so enums render as their actual type (not i32):
            // query strings carry variant names as strings, not integers.
            let type_tokens = method.field_type(&p.field_type, RenderContext::QueryExtractor);
            // Optional and repeated fields get #[serde(default)] so an absent key deserializes to
            // None / an empty Vec rather than erroring. Required query params (proto3 fields not
            // marked `optional`) intentionally have NO default: omitting one is a 400/422, which
            // is the correct contract for a required parameter.
            if p.is_optional() || p.field_type.is_repeated {
                quote! { #[serde(default)] #name: #type_tokens }
            } else {
                quote! { #name: #type_tokens }
            }
        });

        let param_names: Vec<Ident> = params.iter().map(|p| format_ident!("{}", p.name)).collect();

        quote! {
            #[derive(serde::Deserialize)]
            struct QueryParams {
                #(#query_fields,)*
            }
            // axum_extra::extract::Query uses serde_html_form which supports repeated query
            // parameters (?foo=a&foo=b → Vec<T>), unlike axum::extract::Query (serde_urlencoded).
            let axum_extra::extract::Query(QueryParams { #(#param_names),* }) = parts
                .extract::<axum_extra::extract::Query<QueryParams>>()
                .await
                .map_err(axum::response::IntoResponse::into_response)?;
        }
    }
}

/// Generate field assignments for request struct construction as TokenStream.
///
/// Enum query params are extracted as their actual Rust enum type (via the `QueryExtractor`
/// context) but the request struct stores enums in the consumed runtime's representation, so
/// we wrap here: `as i32` for prost, `buffa::EnumValue::Known(..)` for buffa.
fn field_assignments(method: &MethodPlan, runtime: Runtime) -> TokenStream {
    let assignments = method.parameters.iter().map(|param| {
        let ident = param.field_ident();
        match param {
            RequestParam::Query(q) if matches!(q.field_type.base_type, BaseType::Enum(_)) => {
                match runtime {
                    Runtime::Prost if q.field_type.is_repeated => {
                        quote! { #ident: #ident.into_iter().map(|v| v as i32).collect() }
                    }
                    Runtime::Prost if q.is_optional() => {
                        quote! { #ident: #ident.map(|v| v as i32) }
                    }
                    Runtime::Prost => quote! { #ident: #ident as i32 },
                    Runtime::Buffa if q.field_type.is_repeated => {
                        quote! { #ident: #ident.into_iter().map(buffa::EnumValue::Known).collect() }
                    }
                    Runtime::Buffa if q.is_optional() => {
                        quote! { #ident: #ident.map(buffa::EnumValue::Known) }
                    }
                    Runtime::Buffa => quote! { #ident: buffa::EnumValue::Known(#ident) },
                }
            }
            _ => quote! { #ident },
        }
    });
    // buffa messages carry a hidden `__buffa_unknown_fields` field, so the
    // struct literal can't be exhaustive — close it with `..Default::default()`.
    // prost requests, by contrast, are fully composed of path/query/body params,
    // so the literal is already exhaustive and a spread would be a
    // `clippy::needless_update` warning. Only emit it for buffa.
    match runtime {
        Runtime::Buffa => quote! { #(#assignments,)* ..Default::default() },
        Runtime::Prost => quote! { #(#assignments,)* },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{QueryParam, RequestParam};
    use crate::parsing::types::{BaseType, UnifiedType};

    fn make_query_plan(params: Vec<RequestParam>) -> MethodPlan {
        use crate::analysis::RequestType;
        use crate::google::api::{HttpRule, http_rule::Pattern};
        use crate::parsing::{HttpPattern, MethodMetadata};
        MethodPlan {
            metadata: MethodMetadata {
                service_name: "TestService".to_string(),
                method_name: "ListThings".to_string(),
                input_type: "ListThingsRequest".to_string(),
                output_type: "ListThingsResponse".to_string(),
                operation: None,
                http_rule: HttpRule {
                    selector: "".to_string(),
                    pattern: Some(Pattern::Get("/things".to_string())),
                    body: "".to_string(),
                    response_body: "".to_string(),
                    additional_bindings: vec![],
                },
                http_pattern: HttpPattern::parse("/things"),
                documentation: None,
            },
            handler_function_name: "list_things".to_string(),
            http_pattern: HttpPattern::parse("/things"),
            http_method: "GET".to_string(),
            parameters: params,
            has_response: true,
            request_type: RequestType::List,
            output_resource_type: None,
            // Values for a List method: read-only (no body), extracted from request parts.
            has_request_body: false,
            needs_request_parts: true,
            scoped_verb: None,
            // Server extractor tests don't consult `shape`; the test service has no resource.
            shape: crate::analysis::MethodShape::Unbound,
            has_http_route: true,
        }
    }

    fn repeated_string_param(name: &str) -> RequestParam {
        RequestParam::Query(QueryParam {
            name: name.to_string(),
            field_type: UnifiedType {
                base_type: BaseType::String,
                is_optional: false,
                is_repeated: true,
            },
            documentation: None,
            resource_reference: None,
        })
    }

    fn optional_enum_param(name: &str) -> RequestParam {
        RequestParam::Query(QueryParam {
            name: name.to_string(),
            field_type: UnifiedType {
                base_type: BaseType::Enum("example.items.v1.ItemType".to_string()),
                is_optional: true,
                is_repeated: false,
            },
            documentation: None,
            resource_reference: None,
        })
    }

    fn repeated_enum_param(name: &str) -> RequestParam {
        RequestParam::Query(QueryParam {
            name: name.to_string(),
            field_type: UnifiedType {
                base_type: BaseType::Enum("example.items.v1.ItemType".to_string()),
                is_optional: false,
                is_repeated: true,
            },
            documentation: None,
            resource_reference: None,
        })
    }

    #[test]
    fn test_field_assignments_repeated_string_uses_shorthand() {
        let plan = make_query_plan(vec![repeated_string_param("tags")]);
        let tokens = field_assignments(&plan, Runtime::Prost).to_string();
        // Repeated strings use struct shorthand (no cast needed)
        assert!(tokens.contains("tags"), "should emit 'tags'");
        assert!(!tokens.contains("as i32"), "should not cast string to i32");
    }

    #[test]
    fn test_field_assignments_optional_enum_casts_to_i32() {
        let plan = make_query_plan(vec![optional_enum_param("item_type")]);
        let tokens = field_assignments(&plan, Runtime::Prost).to_string();
        assert!(
            tokens.contains("map"),
            "optional enum should use .map(|v| v as i32)"
        );
        assert!(tokens.contains("as i32"), "should cast enum to i32");
    }

    #[test]
    fn test_field_assignments_repeated_enum_collects_as_i32() {
        let plan = make_query_plan(vec![repeated_enum_param("item_types")]);
        let tokens = field_assignments(&plan, Runtime::Prost).to_string();
        assert!(
            tokens.contains("into_iter"),
            "repeated enum should use into_iter().map(|v| v as i32).collect()"
        );
        assert!(
            tokens.contains("as i32"),
            "should cast enum variants to i32"
        );
    }

    #[test]
    fn test_field_assignments_optional_enum_buffa_wraps_enum_value() {
        let plan = make_query_plan(vec![optional_enum_param("item_type")]);
        let tokens = field_assignments(&plan, Runtime::Buffa).to_string();
        assert!(
            tokens.contains("EnumValue :: Known"),
            "optional enum should wrap with buffa::EnumValue::Known under buffa, got: {tokens}"
        );
        assert!(
            !tokens.contains("as i32"),
            "buffa should not cast enum to i32"
        );
    }

    #[test]
    fn test_field_assignments_repeated_enum_buffa_collects_enum_value() {
        let plan = make_query_plan(vec![repeated_enum_param("item_types")]);
        let tokens = field_assignments(&plan, Runtime::Buffa).to_string();
        assert!(
            tokens.contains("into_iter") && tokens.contains("EnumValue :: Known"),
            "repeated enum should collect via buffa::EnumValue::Known under buffa, got: {tokens}"
        );
        assert!(
            !tokens.contains("as i32"),
            "buffa should not cast enum variants to i32"
        );
    }
}
