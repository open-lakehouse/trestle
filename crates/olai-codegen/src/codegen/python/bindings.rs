//! PyO3 binding generation for protobuf-defined services.

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::super::format_tokens;
use crate::analysis::{EmitShape, RequestParam};
use crate::codegen::{BindingMode, MethodHandler, ServiceHandler};
use crate::parsing::types::{BaseType, RenderContext};
use crate::utils::strings;

pub fn main_module(services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
    // Only resource-scoped services have a per-service scoped module; resource-less services'
    // methods live on the root client (see `collection_client_struct`).
    let service_modules = services.iter().filter(|s| s.is_resource_scoped()).map(|s| {
        let module_name = format_ident!("{}", s.plan.base_path);
        quote! { pub mod #module_name; }
    });
    let uc_client_module = collection_client_struct(services);

    let tokens = quote! {
        #(#service_modules)*

        #uc_client_module
    };

    format_tokens(tokens)
}

pub(crate) fn generate(service: &ServiceHandler<'_>) -> crate::error::Result<String> {
    let bindings = service
        .config
        .bindings
        .as_ref()
        .expect("bindings config required for python output");

    let rust_client_ident = service.client_type();
    let client_ident = format_ident!("{}", format!("Py{}", rust_client_ident));
    let rust_client_name = rust_client_ident.to_string();

    let client_crate = format_ident!("{}", bindings.client_crate_name);

    let py_error_type = format_ident!("{}", bindings.py_error_type);
    let py_result_type = format_ident!("{}", bindings.py_result_type);

    let methods = service
        .methods()
        .filter_map(|m| resource_client_method(m, &py_error_type, &py_result_type));
    let mod_path = service.models_path();

    let tokens = quote! {
        use std::collections::HashMap;
        use pyo3::prelude::*;
        use #client_crate::#rust_client_ident;
        use #mod_path::*;
        use crate::error::{#py_error_type, #py_result_type};
        use crate::runtime::get_runtime;

        #[pyclass(name = #rust_client_name)]
        pub struct #client_ident {
            pub(crate) client: #rust_client_ident,
        }

        #[pymethods]
        impl #client_ident {
            #(#methods)*
        }

        impl #client_ident {
            pub fn new(client: #rust_client_ident) -> Self {
                Self { client }
            }
        }
    };

    format_tokens(tokens)
}

fn collection_client_struct(services: &[ServiceHandler<'_>]) -> TokenStream {
    // All services must share the same config; read bindings from the first service.
    let bindings = services
        .first()
        .and_then(|s| s.config.bindings.as_ref())
        .expect("bindings config required for python output");

    let aggregate_client_name = &bindings.aggregate_client_name;
    let client_crate = format_ident!("{}", bindings.client_crate_name);
    let aggregate_client_ident = format_ident!("{}", aggregate_client_name);
    let py_aggregate_client_ident = format_ident!("Py{}", aggregate_client_name);
    let py_error_type = format_ident!("{}", bindings.py_error_type);
    let py_result_type = format_ident!("{}", bindings.py_result_type);

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
            let client_name = format_ident!("Py{}", s.client_type().to_string());
            quote! { use crate::codegen::#mod_name::#client_name; }
        });

    // Root-client methods: collection-style methods from every service, plus — for resource-less
    // services, which have no scoped client — *all* of their methods (get/update/delete/custom),
    // lowered flat so they pass every param including path params.
    let py_error_type_ref = &py_error_type;
    let py_result_type_ref = &py_result_type;
    let methods = sorted_services.iter().flat_map(|s| {
        let mode = s.binding_mode();
        s.methods()
            .filter_map(move |m| root_client_method(m, mode, py_error_type_ref, py_result_type_ref))
    });

    let resource_accessor_methods = sorted_services
        .iter()
        .filter_map(|s| generate_resource_accessor_method(s));

    quote! {
        use std::collections::HashMap;
        use futures::stream::TryStreamExt;
        use pyo3::prelude::*;
        use #client_crate::{#aggregate_client_ident};
        use crate::error::{#py_error_type, #py_result_type};
        use crate::runtime::get_runtime;
        #(#mod_paths)*
        #(#codegen_imports)*

        #[pyclass(name = #aggregate_client_name)]
        pub struct #py_aggregate_client_ident {
            client: #aggregate_client_ident
        }

        #[pymethods]
        impl #py_aggregate_client_ident {
            #[new]
            #[pyo3(signature = (base_url, token = None))]
            pub fn new(base_url: String, token: Option<String>) -> PyResult<Self> {
                let client = if let Some(token) = token {
                    olai_http::CloudClient::new_with_token(token)
                } else {
                    olai_http::CloudClient::new_unauthenticated()
                };
                let base_url = base_url.parse().map_err(#py_error_type::from)?;
                Ok(Self { client: #aggregate_client_ident::new(client, base_url) })
            }

            #(#methods)*

            #(#resource_accessor_methods)*
        }
    }
}

/// Emit an instance method for a resource-scoped client (`get`/`update`/`delete` and
/// resource-targeted custom POST/PATCH RPCs). Always [`BindingMode::Scoped`].
///
/// Only methods that belong on the scoped client are emitted (see
/// [`MethodHandler::is_scoped_instance_method`]); collection methods live on the root client and
/// other custom verbs are not surfaced here. The emit shape is chosen from [`EmitShape`].
fn resource_client_method(
    method: MethodHandler<'_>,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> Option<TokenStream> {
    if !method.is_scoped_instance_method() {
        return None;
    }
    Some(emit_for_shape(
        &method,
        BindingMode::Scoped,
        py_error_type,
        py_result_type,
    ))
}

/// Emit a method on the root (aggregate) client.
///
/// - [`BindingMode::Scoped`] services contribute only their collection-style methods (list /
///   create / factory RPCs); their instance methods live on the scoped client.
/// - [`BindingMode::Flat`] services (resource-less) contribute **every** method, lowered flat so
///   each passes all params (including path params) directly to the root client.
fn root_client_method(
    method: MethodHandler<'_>,
    mode: BindingMode,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> Option<TokenStream> {
    match mode {
        // Scoped services contribute only their collection-style methods to the root client, and
        // only those with a collection emit shape (list / create / factory). A non-collection or
        // non-collection-shaped method (e.g. a scoped `Custom(Get)`) is not surfaced here — its
        // instance form, if any, lives on the scoped client.
        BindingMode::Scoped => {
            let collection_shaped =
                matches!(method.emit_shape(), EmitShape::List | EmitShape::Create);
            if method.is_collection_method() && collection_shaped {
                Some(emit_for_shape(&method, mode, py_error_type, py_result_type))
            } else {
                None
            }
        }
        // Flat (resource-less) services contribute every method, lowered flat.
        BindingMode::Flat => Some(emit_for_shape(&method, mode, py_error_type, py_result_type)),
    }
}

/// Emit a method in the given mode, dispatching purely on its [`EmitShape`]. This is the single
/// dispatch point that replaces the per-(scoped/collection/flat) `match request_type` arms — those
/// only ever varied in *which methods* they emitted (decided by the callers above), not in the
/// per-shape emit logic.
fn emit_for_shape(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> TokenStream {
    match method.emit_shape() {
        EmitShape::List => {
            // `collection_list_method_impl` returns `Option` (needs a list output field); the old
            // dispatch propagated `None` via `?`, but every `EmitShape::List` method has one, so an
            // absent field is a real bug — surface it rather than silently dropping the method.
            collection_list_method_impl(method, mode, py_error_type, py_result_type)
                .expect("List-shaped method must have a list output field")
        }
        EmitShape::Create => {
            collection_create_method_impl(method, mode, py_error_type, py_result_type)
                .expect("Create-shaped method emit")
        }
        EmitShape::GetUpdate => {
            resource_get_update_method_impl(method, mode, py_error_type, py_result_type)
        }
        EmitShape::Delete => {
            resource_delete_method_impl(method, mode, py_error_type, py_result_type)
        }
    }
}

fn collection_list_method_impl(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> Option<TokenStream> {
    let method_name = method.binding_method_name();

    let (param_defs, pyo3_signature) = python_method_parameters(method, mode);
    let client_call = inner_resource_client_call(method, mode);
    let builder_calls = generate_builder_pattern(method, true);

    let items_field = method.list_output_field()?;
    let response_type = method.field_type(&items_field.unified_type, RenderContext::ReturnType);

    Some(quote! {
        #pyo3_signature
        pub fn #method_name(
            &self,
            py: Python,
            #(#param_defs,)*
        ) -> #py_result_type<#response_type> {
            let mut request = #client_call;
            #(#builder_calls)*
            let runtime = get_runtime(py)?;
            py.allow_threads(|| {
                let result = runtime.block_on(async move { request.into_stream().try_collect().await })?;
                Ok::<_, #py_error_type>(result)
            })
        }
    })
}

fn collection_create_method_impl(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> Option<TokenStream> {
    let method_name = method.binding_method_name();
    let response_type = method.output_type_or_unit();
    let (param_defs, pyo3_signature) = python_method_parameters(method, mode);
    let client_call = inner_resource_client_call(method, mode);
    let builder_calls = generate_builder_pattern(method, false);

    Some(quote! {
        #pyo3_signature
        pub fn #method_name(
            &self,
            py: Python,
            #(#param_defs,)*
        ) -> #py_result_type<#response_type> {
            let mut request = #client_call;
            #(#builder_calls)*
            let runtime = get_runtime(py)?;
            py.allow_threads(|| {
                let result = runtime.block_on(request.into_future())?;
                Ok::<_, #py_error_type>(result)
            })
        }
    })
}

fn resource_get_update_method_impl(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> TokenStream {
    let method_name = match mode {
        BindingMode::Scoped => method.plan.resource_client_method(),
        BindingMode::Flat => method.binding_method_name(),
    };
    let response_type = method.output_type_or_unit();
    let (param_defs, pyo3_signature) = python_method_parameters(method, mode);
    let client_call = inner_resource_client_call(method, mode);
    let builder_calls = generate_builder_pattern(method, false);

    quote! {
        #pyo3_signature
        pub fn #method_name(
            &self,
            py: Python,
            #(#param_defs,)*
        ) -> #py_result_type<#response_type> {
            let mut request = #client_call;
            #(#builder_calls)*
            let runtime = get_runtime(py)?;
            py.allow_threads(|| {
                let result = runtime.block_on(request.into_future())?;
                Ok::<_, #py_error_type>(result)
            })
        }
    }
}

fn resource_delete_method_impl(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    py_error_type: &Ident,
    py_result_type: &Ident,
) -> TokenStream {
    let method_name = match mode {
        BindingMode::Scoped => method.plan.resource_client_method(),
        BindingMode::Flat => method.binding_method_name(),
    };
    let (param_defs, pyo3_signature) = python_method_parameters(method, mode);
    let client_call = inner_resource_client_call(method, mode);
    let builder_calls = generate_builder_pattern(method, false);

    quote! {
        #pyo3_signature
        pub fn #method_name(
            &self,
            py: Python,
            #(#param_defs,)*
        ) -> #py_result_type<()> {
            let mut request = #client_call;
            #(#builder_calls)*
            let runtime = get_runtime(py)?;
            py.allow_threads(|| {
                runtime.block_on(request.into_future())?;
                Ok::<_, #py_error_type>(())
            })
        }
    }
}

/// Build the Python method signature (typed param defs + the `#[pyo3(signature)]` attr) from the
/// shared [`MethodHandler::param_plan`], so path/`page_token` filtering lives in one place.
fn python_method_parameters(
    method: &MethodHandler<'_>,
    mode: BindingMode,
) -> (Vec<TokenStream>, TokenStream) {
    let parameters = method.param_plan(mode);
    let signature = render_pyo3(&parameters);
    let param_defs = parameters
        .into_iter()
        .map(|p| {
            let param_name = p.field_ident();
            let rust_type = method.field_type(p.field_type(), RenderContext::PythonParameter);
            quote! { #param_name: #rust_type }
        })
        .collect();
    (param_defs, signature)
}

fn render_pyo3(signature_parts: &[&RequestParam]) -> TokenStream {
    let signature_parts = signature_parts
        .iter()
        .map(|p| {
            if p.is_optional() {
                format!("{} = None", p.name())
            } else {
                p.name().to_string()
            }
        })
        .collect_vec();
    if signature_parts.is_empty() {
        quote! {}
    } else {
        let signature_string = signature_parts.join(", ");
        // signature_parts are protobuf field names (always valid identifiers), so parsing
        // as a TokenStream is infallible.
        let tokens = signature_string
            .parse::<proc_macro2::TokenStream>()
            .unwrap();
        quote! {
            #[pyo3(signature = (#tokens))]
        }
    }
}

/// Emit the call into the underlying Rust client for a binding method.
///
/// Two shapes, selected by `mode`:
/// - [`BindingMode::Scoped`] — the method lives on a resource-scoped client (e.g. `CatalogClient`)
///   that already captured the path params, so the call uses the scoped method name
///   (`get`/`update`/…) and omits path params.
/// - [`BindingMode::Flat`] — the method lives on the root client (resource-less services, treated
///   like collection methods). There is no scoped client to supply path params, so the call uses
///   the base method name and passes **every** required param, including path params.
fn inner_resource_client_call(method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream {
    let (method_name, drop_path) = match mode {
        BindingMode::Scoped => (method.plan.resource_client_method(), true),
        BindingMode::Flat => (method.plan.base_method_ident(), false),
    };
    let args = method
        .required_parameters()
        .filter(|param| !(drop_path && param.is_path_param()))
        .map(|param| {
            let param_name = param.field_ident();
            quote! { #param_name }
        });
    quote! {
        self.client.#method_name(#(#args,)*)
    }
}

fn generate_builder_pattern(method: &MethodHandler<'_>, is_list: bool) -> Vec<TokenStream> {
    let mut builder_calls = Vec::new();

    for query_param in method.plan.query_parameters() {
        if query_param.is_optional() && !(is_list && query_param.name == "page_token") {
            let param_name =
                format_ident!("{}", strings::operation_to_method_name(&query_param.name));
            let with_method = format_ident!("with_{}", query_param.name);
            builder_calls.push(quote! {
                request = request.#with_method(#param_name);
            });
        }
    }

    for body_field in method.plan.body_fields() {
        if body_field.is_optional() {
            let param_name =
                format_ident!("{}", strings::operation_to_method_name(&body_field.name));
            let with_method = format_ident!("with_{}", body_field.name);

            if matches!(body_field.field_type.base_type, BaseType::Map(_, _))
                || body_field.field_type.is_repeated
            {
                builder_calls.push(quote! {
                    if let Some(#param_name) = #param_name {
                        request = request.#with_method(#param_name);
                    }
                });
            } else {
                builder_calls.push(quote! {
                    request = request.#with_method(#param_name);
                });
            }
        }
    }

    builder_calls
}

fn generate_resource_accessor_method(service: &ServiceHandler<'_>) -> Option<TokenStream> {
    let spec = service.accessor_spec()?;
    let method_name = format_ident!("{}", spec.singular);
    let client_name = format_ident!("Py{}", service.client_type().to_string());

    let param_idents: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
    let param_list = param_idents
        .iter()
        .map(|id| quote! { #id: String })
        .collect::<Vec<_>>();

    // A nested resource has a composite `full_name`; the underlying Rust client accessor takes a
    // single `full_name` string, so join the decomposed Python params back together with `.`.
    let method_call = if spec.nested {
        let from_full_name_method = format_ident!("{}_from_full_name", spec.singular);
        let format_str = spec.join_format();
        quote! {
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
            pub fn #method_name(&self, #(#param_list),*) -> #client_name {
                #client_name {
                    client: self.client.#method_name(#(#param_refs),*),
                }
            }
        }
    };

    Some(method_call)
}
