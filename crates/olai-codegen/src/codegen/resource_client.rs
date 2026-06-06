//! Ergonomic resource-scoped client generation.
//!
//! For a resource-scoped service this emits a thin client (e.g. `CatalogClient`) that captures the
//! resource's name components and exposes the instance operations (`get`/`update`/`delete` and
//! resource-targeted custom RPCs), each returning the matching generated builder with the captured
//! components injected as the request's path argument(s).
//!
//! This replaces the previously hand-written scoped clients in consuming crates. The struct is
//! generated into the consuming crate's source tree alongside the low-level client and builders, so
//! hand-written extension `impl` blocks (pagination streams, bespoke helpers, child navigation) in
//! that crate compose with it as additional inherent-impl blocks.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{MethodHandler, ServiceHandler, doc_tokens, format_tokens};
use crate::analysis::RequestParam;
use crate::error::Result;

/// Generate the `resource.rs` module for one resource-scoped service.
///
/// Returns `None` for resource-less services (they have no scoped client — their methods live on
/// the aggregate/root client).
pub(crate) fn generate(service: &ServiceHandler<'_>) -> Result<Option<String>> {
    let Some(scoped_ident) = service.scoped_client_type() else {
        return Ok(None);
    };
    let Some(spec) = service.accessor_spec() else {
        return Ok(None);
    };

    let low_level_ident = service.low_level_client_type();
    let components: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
    let join_format = spec.join_format();

    let struct_def = scoped_struct(&scoped_ident, &components, &low_level_ident);
    let constructor = scoped_constructor(&components, &low_level_ident);
    let methods = instance_methods(service, &components, &join_format);

    let singular_doc = format!(" A client scoped to a single `{}`.", spec.singular);
    let mod_path = service.models_path();

    let tokens = quote! {
        use #mod_path::*;
        use super::builders::*;
        use super::client::#low_level_ident;

        #[doc = #singular_doc]
        #[derive(Clone)]
        pub struct #scoped_ident {
            #struct_def
            client: #low_level_ident,
        }

        impl #scoped_ident {
            #constructor
            #(#methods)*
        }
    };

    Ok(Some(format_tokens(tokens)?))
}

/// The struct's captured-component fields (each a `String`).
fn scoped_struct(
    _scoped_ident: &proc_macro2::Ident,
    components: &[proc_macro2::Ident],
    _low_level_ident: &proc_macro2::Ident,
) -> TokenStream {
    let fields = components.iter().map(|c| quote! { #c: String, });
    quote! { #(#fields)* }
}

/// `pub fn new(<component>: impl Into<String>, …, client: <LowLevel>) -> Self`.
fn scoped_constructor(
    components: &[proc_macro2::Ident],
    low_level_ident: &proc_macro2::Ident,
) -> TokenStream {
    let params = components.iter().map(|c| quote! { #c: impl Into<String> });
    let assigns = components.iter().map(|c| quote! { #c: #c.into() });
    quote! {
        /// Create a client bound to the resource's name components.
        pub fn new(#(#params,)* client: #low_level_ident) -> Self {
            Self {
                #(#assigns,)*
                client,
            }
        }
    }
}

/// One method per `is_scoped_instance_method()` RPC, returning its builder.
fn instance_methods(
    service: &ServiceHandler<'_>,
    components: &[proc_macro2::Ident],
    join_format: &str,
) -> Vec<TokenStream> {
    service
        .methods()
        .filter(|m| m.is_scoped_instance_method())
        .map(|m| instance_method(&m, components, join_format))
        .collect()
}

/// Emit a single instance method: `pub fn <verb>(&self, <non-path args>) -> <Builder> { … }`.
///
/// The builder's `::new` takes the method's required params in order (path params + required body
/// fields). Path params are filled from the captured components; the remaining required params
/// become arguments of the generated method.
fn instance_method(
    method: &MethodHandler<'_>,
    components: &[proc_macro2::Ident],
    join_format: &str,
) -> TokenStream {
    let doc = doc_tokens(method.plan.metadata.documentation.as_deref());
    let method_name = method.plan.resource_client_method();
    let builder_ty = method.builder_type();

    let required: Vec<&RequestParam> = method.required_parameters().collect();
    let path_param_count = required.iter().filter(|p| p.is_path_param()).count();

    // Build the ordered argument expressions for `<Builder>::new(self.client.clone(), <args>)`, and
    // collect the non-path required params that must become method arguments.
    let mut new_args: Vec<TokenStream> = Vec::new();
    let mut method_param_defs: Vec<TokenStream> = Vec::new();
    for param in &required {
        if param.is_path_param() {
            new_args.push(path_arg_expr(
                param,
                components,
                join_format,
                path_param_count,
            ));
        } else {
            // A required non-path field (e.g. a required body field) becomes a method argument,
            // typed like the builder's constructor expects.
            let ident = param.field_ident();
            let ty = method.field_type(
                param.field_type(),
                crate::parsing::types::RenderContext::Constructor,
            );
            method_param_defs.push(quote! { #ident: #ty });
            new_args.push(quote! { #ident });
        }
    }

    quote! {
        #doc
        pub fn #method_name(&self, #(#method_param_defs),*) -> #builder_ty {
            #builder_ty::new(self.client.clone(), #(#new_args),*)
        }
    }
}

/// The expression filling a single path parameter from the captured components.
///
/// - **Composite**: a single path param on a multi-component resource is the dot-joined full name
///   (e.g. `format!("{}.{}", self.catalog_name, self.schema_name)`).
/// - **Direct**: otherwise the path param maps to the same-named captured component (`&self.name`).
fn path_arg_expr(
    param: &RequestParam,
    components: &[proc_macro2::Ident],
    join_format: &str,
    path_param_count: usize,
) -> TokenStream {
    if path_param_count == 1 && components.len() > 1 {
        // Composite full name: join all captured components in order.
        let field_refs = components.iter().map(|c| quote! { self.#c });
        quote! { format!(#join_format, #(#field_refs),*) }
    } else {
        // Direct: this path param corresponds to a captured component of the same name. Fall back to
        // the first component if no exact name match (single-component resources name their path
        // param `name` while the component is e.g. `catalog_name`).
        let name = param.name();
        let field = components
            .iter()
            .find(|c| *c == name)
            .cloned()
            .unwrap_or_else(|| components[0].clone());
        quote! { &self.#field }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{PathParam, QueryParam};
    use crate::parsing::types::{BaseType, UnifiedType};

    fn idents(names: &[&str]) -> Vec<proc_macro2::Ident> {
        names.iter().map(|n| format_ident!("{}", n)).collect()
    }

    fn string_type() -> UnifiedType {
        UnifiedType {
            base_type: BaseType::String,
            is_optional: false,
            is_repeated: false,
        }
    }

    fn path(name: &str) -> RequestParam {
        RequestParam::Path(PathParam {
            name: name.to_string(),
            field_type: string_type(),
            documentation: None,
        })
    }

    fn join_format(n: usize) -> String {
        std::iter::repeat_n("{}", n).collect::<Vec<_>>().join(".")
    }

    /// Flat resource: one component, one path param named `name` (mismatch with the component name)
    /// → falls back to the single captured component by reference.
    #[test]
    fn flat_single_component_direct_ref() {
        let components = idents(&["catalog_name"]);
        let expr = path_arg_expr(&path("name"), &components, &join_format(1), 1);
        assert_eq!(expr.to_string(), quote! { &self.catalog_name }.to_string());
    }

    /// Flat resource where the path param name matches the component → direct ref to that component.
    #[test]
    fn flat_matching_name_direct_ref() {
        let components = idents(&["catalog_name"]);
        let expr = path_arg_expr(&path("catalog_name"), &components, &join_format(1), 1);
        assert_eq!(expr.to_string(), quote! { &self.catalog_name }.to_string());
    }

    /// Nested resource with a single composite path param (e.g. `full_name`) and multiple captured
    /// components → dot-joined `format!`.
    #[test]
    fn nested_single_path_param_joins_components() {
        let components = idents(&["catalog_name", "schema_name"]);
        let expr = path_arg_expr(&path("full_name"), &components, &join_format(2), 1);
        assert_eq!(
            expr.to_string(),
            quote! { format!("{}.{}", self.catalog_name, self.schema_name) }.to_string()
        );
    }

    /// Nested resource whose builder takes separate path params (count > 1) → each path param maps
    /// to its same-named captured component by reference (no join).
    #[test]
    fn nested_separate_path_params_map_by_name() {
        let components = idents(&["catalog_name", "schema_name"]);
        let catalog = path_arg_expr(&path("catalog_name"), &components, &join_format(2), 2);
        let schema = path_arg_expr(&path("schema_name"), &components, &join_format(2), 2);
        assert_eq!(
            catalog.to_string(),
            quote! { &self.catalog_name }.to_string()
        );
        assert_eq!(schema.to_string(), quote! { &self.schema_name }.to_string());
    }

    /// A non-path param is never routed through `path_arg_expr`; this guards the classifier we rely
    /// on (`is_path_param`) so the instance-method split stays correct.
    #[test]
    fn query_param_is_not_a_path_param() {
        let q = RequestParam::Query(QueryParam {
            name: "page_token".to_string(),
            field_type: string_type(),
            documentation: None,
            resource_reference: None,
        });
        assert!(!q.is_path_param());
        assert!(path("name").is_path_param());
    }
}
