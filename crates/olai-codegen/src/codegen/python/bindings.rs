//! PyO3 binding generation for protobuf-defined services.
//!
//! The dispatch and "which methods land where" logic is shared with the Node emitter via
//! [`crate::codegen::bindings`]; this module supplies the PyO3-specific scaffold, marshaling, and
//! method bodies through [`PyBackend`]'s [`BindingBackend`] impl.
//!
//! Each client is emitted once per enabled [`PyFlavor`] (see [`PythonApi`]): a blocking
//! `PyFooClient` (Python `FooClient`) and/or an awaitable `PyAsyncFooClient` (Python
//! `AsyncFooClient`). Both wrap the same Rust client and run on the shared runtime emitted into
//! `codegen/runtime.rs` (see [`super::generate_runtime`]).

use std::collections::BTreeSet;

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::super::format_tokens;
use super::PyFlavor;
use crate::analysis::EmitShape;
use crate::codegen::bindings::builder::{OptionalSetter, SetterRender, generate_builder_pattern};
use crate::codegen::bindings::{BindingBackend, ShapeParts, driver};
use crate::codegen::{BindingMode, MethodHandler, PythonApi, ServiceHandler};
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

/// What a binding method hands back to Python, and how the bare client result becomes it.
enum PyOutput {
    /// A single value: `ty` is the Python-facing return type, `convert` lifts `result` into it.
    Single {
        ty: TokenStream,
        convert: TokenStream,
    },
    /// No value (deletes and `Empty`-returning RPCs): Python sees `None`.
    Unit,
    /// A lazy pager over `item_wrapper` objects, fed by the builder's `into_stream()`.
    List { item_wrapper: Ident },
}

impl PyOutput {
    /// The single-value output for `method`: its `Py` wrapper, or [`PyOutput::Unit`] for an
    /// `Empty`-returning method ([`MethodHandler::output_type`] is `None`).
    fn single(method: &MethodHandler<'_>) -> Self {
        match method.output_type() {
            Some(ident) => {
                let w = wrapper_ident(&ident);
                Self::Single {
                    ty: quote! { #w },
                    convert: quote! { #w::from(result) },
                }
            }
            None => Self::Unit,
        }
    }
}

/// PyO3 binding backend for one [`PyFlavor`]. Holds the configured Python error type ident (client
/// errors are mapped through it before surfacing as `PyErr`) and the flavors enabled overall.
pub(crate) struct PyBackend {
    py_error_type: Ident,
    api: PythonApi,
    flavor: PyFlavor,
}

impl PyBackend {
    fn from_bindings(bindings: &crate::codegen::BindingsConfig) -> Self {
        let api = bindings.python_api;
        Self {
            py_error_type: format_ident!("{}", bindings.py_error_type),
            api,
            flavor: PyFlavor::enabled(api)
                .next()
                .expect("PythonApi enables at least one flavor"),
        }
    }

    fn from_service(service: &ServiceHandler<'_>) -> Self {
        Self::from_bindings(
            service
                .config
                .bindings
                .as_ref()
                .expect("bindings config required for python output"),
        )
    }

    fn from_services(services: &[ServiceHandler<'_>]) -> Self {
        Self::from_bindings(
            services
                .first()
                .and_then(|s| s.config.bindings.as_ref())
                .expect("bindings config required for python output"),
        )
    }

    /// This backend re-targeted at each enabled flavor.
    fn flavors(&self) -> impl Iterator<Item = Self> + '_ {
        PyFlavor::enabled(self.api).map(|flavor| Self {
            py_error_type: self.py_error_type.clone(),
            api: self.api,
            flavor,
        })
    }

    /// The `use` lines every generated binding module needs, independent of flavor.
    fn common_imports(&self) -> TokenStream {
        let py_error_type = &self.py_error_type;
        let async_imports = self.api.has_async().then(|| {
            quote! { use crate::codegen::runtime::AsyncListIterator; }
        });
        quote! {
            use std::collections::HashMap;
            use futures::stream::StreamExt;
            use pyo3::IntoPyObjectExt;
            use pyo3::prelude::*;
            use crate::error::#py_error_type;
            use crate::codegen::runtime::{block_on, ListIterator};
            #async_imports
        }
    }

    /// Assemble a binding method: signature, builder setup, and the flavor-specific body that runs
    /// the request and hands `output` back to Python.
    fn emit_method(
        &self,
        parts: &ShapeParts<'_>,
        method_name: &Ident,
        is_list: bool,
        output: PyOutput,
    ) -> TokenStream {
        let method = parts.method;
        let py_error_type = &self.py_error_type;
        let param_defs = &parts.param_defs;
        let signature = pyo3_signature(method, parts.mode);
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, is_list, self);
        let mut_kw = mut_if_setters(&builder_calls);
        let setup = quote! {
            let #mut_kw request = #client_call;
            #(#builder_calls)*
        };

        match (self.flavor, output) {
            // Lists return a lazy pager in both flavors; only the pager type differs. Each item is
            // converted to its wrapper object as it is pulled, so no `py` token is needed here.
            (flavor, PyOutput::List { item_wrapper }) => {
                let pager = match flavor {
                    PyFlavor::Sync => quote! { ListIterator },
                    PyFlavor::Async => quote! { AsyncListIterator },
                };
                quote! {
                    #signature
                    pub fn #method_name(&self, #(#param_defs,)*) -> PyResult<#pager> {
                        #setup
                        let items = request.into_stream().map(|item| {
                            let item = item.map_err(#py_error_type::from)?;
                            Python::attach(|py| #item_wrapper::from(item).into_py_any(py))
                        });
                        Ok(#pager::new(items))
                    }
                }
            }
            (PyFlavor::Sync, output) => {
                let (return_type, tail) = match output {
                    PyOutput::Single { ty, convert } => (
                        ty,
                        quote! {
                            // `result` is the bare model the Rust client returns; `#convert` lifts
                            // it into the `Py` wrapper.
                            let result = block_on(py, request.into_future())?
                                .map_err(#py_error_type::from)?;
                            Ok(#convert)
                        },
                    ),
                    PyOutput::Unit => (
                        quote! { () },
                        quote! {
                            block_on(py, request.into_future())?.map_err(#py_error_type::from)?;
                            Ok(())
                        },
                    ),
                    PyOutput::List { .. } => unreachable!("handled above"),
                };
                quote! {
                    #signature
                    pub fn #method_name(
                        &self,
                        py: Python<'_>,
                        #(#param_defs,)*
                    ) -> PyResult<#return_type> {
                        #setup
                        #tail
                    }
                }
            }
            (PyFlavor::Async, output) => {
                let tail = match output {
                    PyOutput::Single { convert, .. } => quote! {
                        let result = request.await.map_err(#py_error_type::from)?;
                        Ok::<_, PyErr>(#convert)
                    },
                    // Resolve to `None`: a bare `()` would convert to an empty tuple, unlike a
                    // sync `#[pymethods]` return of `()`.
                    PyOutput::Unit => quote! {
                        request.await.map_err(#py_error_type::from)?;
                        Ok::<_, PyErr>(None::<()>)
                    },
                    PyOutput::List { .. } => unreachable!("handled above"),
                };
                // Params are moved into the builder before the future is created, so the future
                // owns everything it touches (`'static + Send`, as `future_into_py` requires).
                quote! {
                    #signature
                    pub fn #method_name<'py>(
                        &self,
                        py: Python<'py>,
                        #(#param_defs,)*
                    ) -> PyResult<Bound<'py, PyAny>> {
                        #setup
                        ::pyo3_async_runtimes::tokio::future_into_py(py, async move { #tail })
                    }
                }
            }
        }
    }

    /// One `#[pyclass]` wrapper (struct + `#[pymethods]`) for a scoped service client, in this
    /// backend's flavor.
    fn service_class(&self, service: &ServiceHandler<'_>) -> TokenStream {
        let rust_client_ident = service.client_type();
        let rust_client_name = rust_client_ident.to_string();
        let client_ident = self.flavor.class_ident(&rust_client_name);
        let class_name = self.flavor.class_name(&rust_client_name);
        let methods = driver::scoped_client_methods(self, service);

        quote! {
            #[pyclass(name = #class_name)]
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
        }
    }

    /// The root (aggregate) `#[pyclass]` in this backend's flavor: constructor, collection methods,
    /// and resource accessors.
    fn root_class(
        &self,
        sorted_services: &[&ServiceHandler<'_>],
        aggregate_client_name: &str,
    ) -> TokenStream {
        let py_error_type = &self.py_error_type;
        let aggregate_client_ident = format_ident!("{}", aggregate_client_name);
        let py_aggregate_client_ident = self.flavor.class_ident(aggregate_client_name);
        let class_name = self.flavor.class_name(aggregate_client_name);
        let methods = driver::root_client_methods(self, sorted_services);
        let resource_accessor_methods = driver::resource_accessor_methods(self, sorted_services);

        quote! {
            #[pyclass(name = #class_name)]
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
        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let common_imports = self.common_imports();

        let classes = self
            .flavors()
            .map(|backend| backend.service_class(service))
            .collect_vec();
        let mod_path = service.models_path();
        // The `Py*` model wrappers are re-exported at the models root; import them so
        // the wrapper return/param types resolve.
        let wrappers_import = service
            .models_root_path()
            .map(|root| quote! { use #root::*; });

        let tokens = quote! {
            #common_imports
            use #client_crate::#rust_client_ident;
            use #mod_path::*;
            #wrappers_import

            #(#classes)*
        };

        format_tokens(tokens)
    }

    fn main_module(&self, services: &[ServiceHandler<'_>]) -> crate::error::Result<String> {
        // Only resource-scoped services have a per-service scoped module; resource-less services'
        // methods live on the root client.
        // Generated PyO3 modules structurally trip dead_code (full surface
        // emitted), too_many_arguments (flat `#[pymethods]`), and unused_imports
        // (per-module use-prelude). Allow them so the binding crate builds clean.
        let service_modules = services
            .iter()
            .filter(|s| s.emits_scoped_binding())
            .map(|s| {
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
        let common_imports = self.common_imports();

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

        // Only services with a scoped binding expose a per-service scoped client to import, once
        // per enabled flavor.
        let codegen_imports = sorted_services
            .iter()
            .filter(|s| s.emits_scoped_binding())
            .flat_map(|s| {
                let mod_name = format_ident!("{}", s.plan.base_path);
                let rust_client = s.client_type().to_string();
                PyFlavor::enabled(self.api)
                    .map(|flavor| {
                        let client_name = flavor.class_ident(&rust_client);
                        quote! { use crate::codegen::#mod_name::#client_name; }
                    })
                    .collect_vec()
            });

        let root_classes = self
            .flavors()
            .map(|backend| backend.root_class(&sorted_services, aggregate_client_name))
            .collect_vec();

        // Every class the consumer's `#[pymodule]` should expose: the root and scoped clients per
        // flavor, plus the pagers their list methods return.
        let registered_classes = PyFlavor::enabled(self.api)
            .flat_map(|flavor| {
                std::iter::once(flavor.class_ident(aggregate_client_name)).chain(
                    sorted_services
                        .iter()
                        .filter(|s| s.emits_scoped_binding())
                        .map(move |s| flavor.class_ident(&s.client_type().to_string())),
                )
            })
            .chain(self.api.has_sync().then(|| format_ident!("ListIterator")))
            .chain(
                self.api
                    .has_async()
                    .then(|| format_ident!("AsyncListIterator")),
            )
            .collect_vec();

        let tokens = quote! {
            // The aggregate module file carries the root `#[pyclass]` (flat methods →
            // too_many_arguments) and a use-prelude that not every build consumes.
            #![allow(dead_code, unused_imports, clippy::too_many_arguments)]

            pub mod runtime;
            #(#service_modules)*

            #common_imports
            use #client_crate::{#aggregate_client_ident};
            #(#mod_paths)*
            #(#wrapper_imports)*
            #(#codegen_imports)*

            #(#root_classes)*

            /// Add every generated client class (and the list pagers) to the extension module `m`.
            ///
            /// Call this from the crate's `#[pymodule]` so newly generated clients are exposed
            /// without hand-maintaining an `add_class` list.
            pub fn register_clients(m: &Bound<'_, PyModule>) -> PyResult<()> {
                #(m.add_class::<#registered_classes>()?;)*
                Ok(())
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
        let is_list = method.emit_shape() == EmitShape::List;
        let args = method
            .required_parameters()
            .filter(|param| !(drop_path && param.is_path_param()))
            .map(|param| {
                // A required `page_token` isn't exposed to Python on list methods (the stream drives
                // pagination and overwrites it), so seed the builder with an empty token.
                if is_list && param.name() == "page_token" {
                    return quote! { ::std::string::String::new() };
                }
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
        let items_field = method
            .list_output_field()
            .expect("List-shaped method must have a list output field");
        // The list item type, and the wrapper it crosses the boundary as. List
        // items are always message types, so `type_ident()` is the model name.
        let item_wrapper = wrapper_ident(&items_field.unified_type.type_ident());
        self.emit_method(
            parts,
            &method.binding_method_name(),
            true,
            PyOutput::List { item_wrapper },
        )
    }

    fn emit_create(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        self.emit_method(
            parts,
            &method.binding_method_name(),
            false,
            PyOutput::single(method),
        )
    }

    fn emit_get_update(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let method_name = match parts.mode {
            BindingMode::Scoped => method.plan.resource_client_method(),
            BindingMode::Flat => method.binding_method_name(),
        };
        self.emit_method(parts, &method_name, false, PyOutput::single(method))
    }

    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        let method_name = match parts.mode {
            BindingMode::Scoped => method.plan.resource_client_method(),
            BindingMode::Flat => method.binding_method_name(),
        };
        self.emit_method(parts, &method_name, false, PyOutput::Unit)
    }

    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream> {
        let spec = service.accessor_spec()?;
        let method_name = format_ident!("{}", spec.singular);
        let client_name = self.flavor.class_ident(&service.client_type().to_string());

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
