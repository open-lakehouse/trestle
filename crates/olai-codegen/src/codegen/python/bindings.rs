//! PyO3 binding generation for protobuf-defined services.
//!
//! The dispatch and "which methods land where" logic is shared with the Node emitter via
//! [`crate::codegen::bindings`]; this module supplies the PyO3-specific scaffold, marshaling, and
//! method bodies through [`PyBackend`]'s [`BindingBackend`] impl.

use std::collections::BTreeSet;

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::super::format_tokens;
use crate::codegen::bindings::builder::{OptionalSetter, SetterRender, generate_builder_pattern};
use crate::codegen::bindings::{BindingBackend, ShapeParts, driver};
use crate::codegen::{BindingMode, MethodHandler, ServiceHandler};
use crate::parsing::types::{BaseType, RenderContext, UnifiedType};

/// The `Py`-prefixed wrapper ident for a model type ident (e.g. `Catalog` →
/// `PyCatalog`). Wrapper names use the parent-qualified emitted type name, which
/// for a top-level `output_type()`/`input_type()` ident is just the simple name,
/// so prefixing is sufficient here.
fn wrapper_ident(model_ident: &Ident) -> Ident {
    format_ident!("Py{}", model_ident)
}

/// The `Py`-prefixed wrapper ident for a (possibly nested) model type, using the
/// **parent-qualified** emitted name so a message-nested type resolves to the same
/// ident the wrapper is defined under (e.g. the nested `…Request.Operation` enum →
/// `PyGenerate…RequestOperation`, not the colliding bare `PyOperation`). For a
/// top-level type this is identical to [`wrapper_ident`] of the simple name.
fn qualified_wrapper_ident(ty: &UnifiedType) -> Ident {
    match &ty.base_type {
        BaseType::Message(name) | BaseType::Enum(name) | BaseType::OneOf(name) => {
            format_ident!("Py{}", crate::utils::extract_qualified_type_name(name))
        }
        _ => wrapper_ident(&ty.type_ident()),
    }
}

/// Wrap a model output ident as its `Py` wrapper return type, and produce the
/// expression converting the bare client result `result` into it.
///
/// Returns `(return_type, convert_expr)`. For an `Empty`-returning method
/// ([`MethodHandler::output_type`] is `None`) the return type is `()` and the
/// value is passed through unchanged.
fn wrap_single_output(method: &MethodHandler<'_>) -> (TokenStream, TokenStream) {
    match method.output_type() {
        Some(ident) => {
            let w = wrapper_ident(&ident);
            (quote! { #w }, quote! { #w::from(result) })
        }
        None => (quote! { () }, quote! { result }),
    }
}

/// `mut` keyword for the `let request = ...` binding, but only when there are
/// optional-field setter calls that actually mutate it. Without setters the
/// binding is never reassigned, so `let mut` would trip `unused_mut`.
fn mut_if_setters(builder_calls: &[TokenStream]) -> TokenStream {
    if builder_calls.is_empty() {
        quote! {}
    } else {
        quote! { mut }
    }
}

/// PyO3 binding backend. Holds the configured Python error/result type idents so the method bodies
/// can wrap calls in the right `PyResult`/error conversion.
pub(crate) struct PyBackend {
    py_error_type: Ident,
    py_result_type: Ident,
}

impl PyBackend {
    fn from_service(service: &ServiceHandler<'_>) -> Self {
        let bindings = service
            .config
            .bindings
            .as_ref()
            .expect("bindings config required for python output");
        Self {
            py_error_type: format_ident!("{}", bindings.py_error_type),
            py_result_type: format_ident!("{}", bindings.py_result_type),
        }
    }

    fn from_services(services: &[ServiceHandler<'_>]) -> Self {
        let bindings = services
            .first()
            .and_then(|s| s.config.bindings.as_ref())
            .expect("bindings config required for python output");
        Self {
            py_error_type: format_ident!("{}", bindings.py_error_type),
            py_result_type: format_ident!("{}", bindings.py_result_type),
        }
    }
}

pub fn main_module(services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
    PyBackend::from_services(services).main_module(services)
}

pub(crate) fn generate(service: &ServiceHandler<'_>) -> crate::error::Result<String> {
    PyBackend::from_service(service).generate_service(service)
}

impl BindingBackend for PyBackend {
    fn generate_service(&self, service: &ServiceHandler<'_>) -> crate::error::Result<String> {
        let bindings = service
            .config
            .bindings
            .as_ref()
            .expect("bindings config required for python output");

        let rust_client_ident = service.client_type();
        let client_ident = format_ident!("Py{}", rust_client_ident);
        let rust_client_name = rust_client_ident.to_string();

        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;

        let methods = driver::scoped_client_methods(self, service);
        let mod_path = service.models_path();
        // The `Py*` model wrappers are re-exported at the models root; import them so
        // the wrapper return/param types resolve.
        let wrappers_import = service
            .models_root_path()
            .map(|root| quote! { use #root::*; });

        let tokens = quote! {
            use std::collections::HashMap;
            use pyo3::prelude::*;
            use #client_crate::#rust_client_ident;
            use #mod_path::*;
            #wrappers_import
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

    fn main_module(&self, services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
        // Only resource-scoped services have a per-service scoped module; resource-less services'
        // methods live on the root client.
        // Generated PyO3 modules structurally trip dead_code (full surface
        // emitted), too_many_arguments (flat `#[pymethods]`), and unused_imports
        // (per-module use-prelude). Allow them so the binding crate builds clean.
        let service_modules = services.iter().filter(|s| s.is_resource_scoped()).map(|s| {
            let module_name = format_ident!("{}", s.plan.base_path);
            quote! {
                #[allow(dead_code, unused_imports, clippy::too_many_arguments)]
                pub mod #module_name;
            }
        });

        let bindings = services
            .first()
            .and_then(|s| s.config.bindings.as_ref())
            .expect("bindings config required for python output");

        let aggregate_client_name = &bindings.aggregate_client_name;
        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let aggregate_client_ident = format_ident!("{}", aggregate_client_name);
        let py_aggregate_client_ident = format_ident!("Py{}", aggregate_client_name);
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;

        let mut sorted_services = services.iter().collect_vec();
        sorted_services.sort_by(|a, b| a.plan.service_name.cmp(&b.plan.service_name));

        let mod_paths = sorted_services.iter().map(|s| {
            let mod_path = s.models_path();
            quote! { use #mod_path::*; }
        });
        // Import the `Py*` model wrappers (re-exported at the models root). All
        // services share one root, so dedupe to a single glob import.
        let wrapper_imports: Vec<TokenStream> = sorted_services
            .iter()
            .filter_map(|s| s.models_root_path())
            .map(|root| quote! { #root }.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|root| {
                let path: syn::Path = syn::parse_str(&root).expect("models root path re-parses");
                quote! { use #path::*; }
            })
            .collect();

        // Only resource-scoped services expose a per-service scoped client to import.
        let codegen_imports = sorted_services
            .iter()
            .filter(|s| s.is_resource_scoped())
            .map(|s| {
                let mod_name = format_ident!("{}", s.plan.base_path);
                let client_name = format_ident!("Py{}", s.client_type().to_string());
                quote! { use crate::codegen::#mod_name::#client_name; }
            });

        let methods = driver::root_client_methods(self, &sorted_services);
        let resource_accessor_methods = driver::resource_accessor_methods(self, &sorted_services);

        let tokens = quote! {
            // The aggregate module file carries the root `#[pyclass]` (flat methods →
            // too_many_arguments) and a use-prelude that not every build consumes.
            #![allow(dead_code, unused_imports, clippy::too_many_arguments)]

            #(#service_modules)*

            use std::collections::HashMap;
            use futures::stream::TryStreamExt;
            use pyo3::prelude::*;
            use #client_crate::{#aggregate_client_ident};
            use crate::error::{#py_error_type, #py_result_type};
            use crate::runtime::get_runtime;
            #(#mod_paths)*
            #(#wrapper_imports)*
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
        };

        format_tokens(tokens)
    }

    fn param_defs(&self, method: &MethodHandler<'_>, mode: BindingMode) -> Vec<TokenStream> {
        python_param_defs(method, mode)
    }

    fn client_call(&self, method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream {
        let (method_name, drop_path) = match mode {
            BindingMode::Scoped => (method.plan.resource_client_method(), true),
            BindingMode::Flat => (method.plan.base_method_ident(), false),
        };
        let args = method
            .required_parameters()
            .filter(|param| !(drop_path && param.is_path_param()))
            .map(|param| {
                let param_name = param.field_ident();
                // Required message/enum params arrive as their `Py` wrapper and must
                // be converted back to the bare model the Rust client takes.
                convert_param_value(param.field_type(), quote! { #param_name })
            });
        quote! {
            self.client.#method_name(#(#args,)*)
        }
    }

    fn emit_list(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;
        let method_name = method.binding_method_name();
        let param_defs = &parts.param_defs;
        let signature = pyo3_signature(method, parts.mode);
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, true, self);
        let mut_kw = mut_if_setters(&builder_calls);

        let items_field = method
            .list_output_field()
            .expect("List-shaped method must have a list output field");
        // The list item type, and the wrapper it crosses the boundary as. List
        // items are always message types, so `type_ident()` is the model name.
        let item_ident = items_field.unified_type.type_ident();
        let item_wrapper = wrapper_ident(&item_ident);

        quote! {
            #signature
            pub fn #method_name(
                &self,
                py: Python,
                #(#param_defs,)*
            ) -> #py_result_type<::std::vec::Vec<#item_wrapper>> {
                let #mut_kw request = #client_call;
                #(#builder_calls)*
                let runtime = get_runtime(py)?;
                py.allow_threads(|| {
                    let result: ::std::vec::Vec<_> = runtime.block_on(async move { request.into_stream().try_collect().await })?;
                    Ok::<_, #py_error_type>(result.into_iter().map(#item_wrapper::from).collect())
                })
            }
        }
    }

    fn emit_create(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;
        let method_name = method.binding_method_name();
        let (response_type, convert) = wrap_single_output(method);
        let param_defs = &parts.param_defs;
        let signature = pyo3_signature(method, parts.mode);
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, false, self);
        let mut_kw = mut_if_setters(&builder_calls);

        quote! {
            #signature
            pub fn #method_name(
                &self,
                py: Python,
                #(#param_defs,)*
            ) -> #py_result_type<#response_type> {
                let #mut_kw request = #client_call;
                #(#builder_calls)*
                let runtime = get_runtime(py)?;
                py.allow_threads(|| {
                    #[allow(clippy::let_unit_value)]
                    let result = runtime.block_on(request.into_future())?;
                    Ok::<_, #py_error_type>(#convert)
                })
            }
        }
    }

    fn emit_get_update(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;
        let method_name = match parts.mode {
            BindingMode::Scoped => method.plan.resource_client_method(),
            BindingMode::Flat => method.binding_method_name(),
        };
        let (response_type, convert) = wrap_single_output(method);
        let param_defs = &parts.param_defs;
        let signature = pyo3_signature(method, parts.mode);
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, false, self);
        let mut_kw = mut_if_setters(&builder_calls);

        quote! {
            #signature
            pub fn #method_name(
                &self,
                py: Python,
                #(#param_defs,)*
            ) -> #py_result_type<#response_type> {
                let #mut_kw request = #client_call;
                #(#builder_calls)*
                let runtime = get_runtime(py)?;
                py.allow_threads(|| {
                    // `let result = ...` then convert: `result` is the bare model the
                    // Rust client returns; `#convert` lifts it into the `Py` wrapper
                    // (or passes a unit through unchanged).
                    #[allow(clippy::let_unit_value)]
                    let result = runtime.block_on(request.into_future())?;
                    Ok::<_, #py_error_type>(#convert)
                })
            }
        }
    }

    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;
        let method_name = match parts.mode {
            BindingMode::Scoped => method.plan.resource_client_method(),
            BindingMode::Flat => method.binding_method_name(),
        };
        let param_defs = &parts.param_defs;
        let signature = pyo3_signature(method, parts.mode);
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, false, self);
        let mut_kw = mut_if_setters(&builder_calls);

        quote! {
            #signature
            pub fn #method_name(
                &self,
                py: Python,
                #(#param_defs,)*
            ) -> #py_result_type<()> {
                let #mut_kw request = #client_call;
                #(#builder_calls)*
                let runtime = get_runtime(py)?;
                py.allow_threads(|| {
                    runtime.block_on(request.into_future())?;
                    Ok::<_, #py_error_type>(())
                })
            }
        }
    }

    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream> {
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
}

impl SetterRender for PyBackend {
    fn supports(&self, _setter: &OptionalSetter<'_>) -> bool {
        // PyO3 accepts every parameter type; no capability filter.
        true
    }

    fn render_setter(
        &self,
        setter: &OptionalSetter<'_>,
        param_ident: &Ident,
        with_method: &Ident,
    ) -> TokenStream {
        // Maps and repeated fields are `Option`-wrapped at the binding boundary; guard before
        // forwarding. Repeated message/enum elements are also converted from their `Py`
        // wrapper to the bare model.
        if matches!(setter.field_type.base_type, BaseType::Map(_, _))
            || setter.field_type.is_repeated
        {
            let inner = if param_needs_conversion(setter.field_type) {
                quote! { #param_ident.into_iter().map(::core::convert::Into::into).collect::<::std::vec::Vec<_>>() }
            } else {
                quote! { #param_ident }
            };
            quote! {
                if let Some(#param_ident) = #param_ident {
                    request = request.#with_method(#inner);
                }
            }
        } else if param_needs_conversion(setter.field_type) {
            // Optional singular message/enum: `Option<PyWrapper>` → `Option<Inner>` for
            // the builder, which accepts `impl Into<Option<Inner>>`.
            quote! {
                request = request.#with_method(#param_ident.map(::core::convert::Into::into));
            }
        } else {
            quote! {
                request = request.#with_method(#param_ident);
            }
        }
    }
}

/// Build the Python method signature param defs from the shared [`MethodHandler::param_plan`], so
/// path/`page_token` filtering lives in one place.
///
/// Message- and enum-typed params cross the boundary as their `Py` wrapper type
/// (e.g. `Option<Catalog>` → `Option<PyCatalog>`); scalars are unchanged. The
/// matching value conversion happens at the call/setter site
/// ([`PyBackend::client_call`] / [`PyBackend::render_setter`]).
fn python_param_defs(method: &MethodHandler<'_>, mode: BindingMode) -> Vec<TokenStream> {
    method
        .param_plan(mode)
        .into_iter()
        .map(|p| {
            let param_name = p.field_ident();
            let rust_type = method.field_type(p.field_type(), RenderContext::PythonParameter);
            let py_type = wrap_param_type(p.field_type(), &rust_type);
            quote! { #param_name: #py_type }
        })
        .collect()
}

/// Whether a param's base type is a model (message/enum) that has a `Py` wrapper,
/// so its value must be converted (`.into()`) when forwarded to the Rust client.
fn param_needs_conversion(ty: &UnifiedType) -> bool {
    matches!(
        ty.base_type,
        BaseType::Message(_) | BaseType::Enum(_) | BaseType::OneOf(_)
    )
}

/// Rewrite a rendered param `syn::Type` so a message/enum base type becomes its
/// `Py` wrapper, preserving the `Option`/`Vec` cardinality wrapping. Scalars,
/// maps, and `()` are returned unchanged.
fn wrap_param_type(ty: &UnifiedType, rendered: &syn::Type) -> TokenStream {
    if !param_needs_conversion(ty) {
        return quote! { #rendered };
    }
    let wrapper = qualified_wrapper_ident(ty);
    let mut inner = quote! { #wrapper };
    if ty.is_repeated {
        inner = quote! { ::std::vec::Vec<#inner> };
    }
    // FFI rule (`should_wrap_in_option`): optional fields and FFI collections are
    // `Option`-wrapped. Repeated message/enum params follow the same rule as the
    // scalar renderer, so mirror `is_optional || is_repeated` here.
    if ty.is_optional || ty.is_repeated {
        inner = quote! { ::core::option::Option<#inner> };
    }
    inner
}

/// The expression that converts a Python-wrapper param value `expr` (of the type
/// produced by [`wrap_param_type`]) back into the bare model the Rust client
/// expects. Handles the `Option`/`Vec` cardinality wrapping. Scalars pass through.
fn convert_param_value(ty: &UnifiedType, expr: TokenStream) -> TokenStream {
    if !param_needs_conversion(ty) {
        return expr;
    }
    match (ty.is_repeated, ty.is_optional) {
        (true, _) => quote! {
            #expr.map(|v| v.into_iter().map(::core::convert::Into::into).collect())
        },
        (false, true) => quote! { #expr.map(::core::convert::Into::into) },
        (false, false) => quote! { #expr.into() },
    }
}

/// The `#[pyo3(signature = (...))]` attribute for a method, derived from its param plan.
fn pyo3_signature(method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream {
    let signature_parts = method
        .param_plan(mode)
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
