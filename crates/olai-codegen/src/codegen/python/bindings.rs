//! PyO3 binding generation for protobuf-defined services.
//!
//! The dispatch and "which methods land where" logic is shared with the Node emitter via
//! [`crate::codegen::bindings`]; this module supplies the PyO3-specific scaffold, marshaling, and
//! method bodies through [`PyBackend`]'s [`BindingBackend`] impl.

use std::collections::BTreeMap;

use itertools::Itertools;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::super::format_tokens;
use crate::codegen::bindings::builder::{OptionalSetter, SetterRender, generate_builder_pattern};
use crate::codegen::bindings::{BindingBackend, ShapeParts, driver};
use crate::codegen::{BindingMode, MethodHandler, PythonOptions, ServiceHandler};
use crate::parsing::types::{BaseType, JsonType, RenderContext, UnifiedType};

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

/// The Python-visible return type of a method and the expression that converts the bare client
/// `result` into it; `None` for a method that returns nothing.
fn wrap_single_output(method: &MethodHandler<'_>) -> Option<(TokenStream, TokenStream)> {
    if method.is_existence_check() {
        return Some((quote! { bool }, quote! { result }));
    }
    let w = wrapper_ident(&method.output_type()?);
    Some((quote! { #w }, quote! { #w::from(result) }))
}

/// PyO3 binding backend. Holds the configured Python error/result type idents so the method bodies
/// can wrap calls in the right `PyResult`/error conversion.
pub(crate) struct PyBackend<'a> {
    py_error_type: Ident,
    py_result_type: Ident,
    python: &'a PythonOptions,
}

impl<'a> PyBackend<'a> {
    fn from_service(service: &ServiceHandler<'a>) -> Self {
        let bindings = service
            .config
            .bindings
            .as_ref()
            .expect("bindings config required for python output");
        Self {
            py_error_type: format_ident!("{}", bindings.py_error_type),
            py_result_type: format_ident!("{}", bindings.py_result_type),
            python: service.config.python_options(),
        }
    }

    fn from_services(services: &[ServiceHandler<'a>]) -> Self {
        Self::from_service(services.first().expect("python output has a service"))
    }

    /// The extension module [`PythonOptions::module`]: a declarative `#[pymodule]`, so that
    /// PyO3's introspection sees every class. It holds the root client and the exceptions; the
    /// classes of each proto package go in a submodule, which its init also puts in
    /// `sys.modules`.
    fn extension_module(&self, services: &[&ServiceHandler<'_>], aggregate: &Ident) -> TokenStream {
        let Some(first) = services.first() else {
            return quote! {};
        };
        // Classes per package segment: the model wrappers, then the scoped clients.
        let mut groups: BTreeMap<&str, Vec<Ident>> = BTreeMap::new();
        let plans: Vec<_> = services.iter().map(|s| s.plan).collect();
        let classes = super::model_conversions::ModelClasses::new(&plans, first.metadata);
        for (segment, wrapper) in classes.pyclasses() {
            groups.entry(segment).or_default().push(wrapper);
        }
        for service in services.iter().filter(|s| s.emits_scoped_binding()) {
            groups
                .entry(super::package_segment(&service.plan.package))
                .or_default()
                .push(format_ident!("Py{}", service.client_type().to_string()));
        }

        let full_name = &self.python.module;
        let (parent, name) = match full_name.rsplit_once('.') {
            Some((parent, name)) => (Some(parent), name),
            None => (None, full_name.as_str()),
        };
        let module_option = parent.map(|parent| quote! { , module = #parent });
        let module_doc = format!(" The extension module `{full_name}`.");
        let exceptions = super::errors::class_idents(self.python);
        let segments = groups.keys();
        let submodules = groups.iter().map(|(segment, classes)| {
            let ident = crate::utils::field_ident(segment);
            let doc = format!(" The classes of the `{segment}` API.");
            quote! {
                #[doc = #doc]
                #[pymodule]
                pub mod #ident {
                    #[pymodule_export]
                    use super::super::{#(#classes),*};
                }
            }
        });
        let init_async = self
            .python
            .async_methods
            .then(|| quote! { crate::codegen::runtime::init_async_runtime(); });
        quote! {
            /// The extension module, as Python imports it.
            pub const CORE_MODULE: &str = #full_name;

            /// The submodules of the extension module.
            const SUBMODULES: &[&str] = &[#(#segments),*];

            #[doc = #module_doc]
            #[::pyo3::pymodule]
            #[pyo3(name = #name #module_option)]
            pub mod extension_module {
                use ::pyo3::prelude::*;

                /// The version of the bindings crate.
                #[pymodule_export]
                #[allow(non_upper_case_globals)]
                const __version__: &str = env!("CARGO_PKG_VERSION");

                #[pymodule_export]
                use super::#aggregate;

                #[pymodule_export]
                use crate::codegen::error::{#(#exceptions),*};

                #(#submodules)*

                /// Put the submodules in `sys.modules`. The import system puts the module there
                /// before it runs this. Not handed the module, so the stubs still describe the
                /// module in full.
                #[pymodule_init]
                fn init(py: Python<'_>) -> PyResult<()> {
                    #init_async
                    let modules = py.import("sys")?.getattr("modules")?;
                    match modules.get_item(super::CORE_MODULE) {
                        Ok(module) => super::add_submodules_to_sys_modules(module.cast()?),
                        Err(_) => Ok(()),
                    }
                }
            }

            /// Put the submodules of the extension module `module` in `sys.modules`, so that
            /// `import <module>.<submodule>` finds them. The module does this on import; call it
            /// for a module made otherwise, such as with `wrap_pymodule!`.
            pub fn add_submodules_to_sys_modules(
                module: &::pyo3::Bound<'_, ::pyo3::types::PyModule>,
            ) -> ::pyo3::PyResult<()> {
                let modules = module.py().import("sys")?.getattr("modules")?;
                let name = module.name()?;
                for submodule in SUBMODULES {
                    modules.set_item(format!("{name}.{submodule}"), module.getattr(*submodule)?)?;
                }
                Ok(())
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

impl BindingBackend for PyBackend<'_> {
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

        let module_arg = super::module_arg(
            self.python,
            Some(super::package_segment(&service.plan.package)),
        );
        let futures_import = self
            .python
            .async_methods
            .then(|| quote! { use futures::TryStreamExt; });
        let resource = service
            .resource()
            .map(|r| r.type_name.clone())
            .unwrap_or_else(|| rust_client_name.clone());
        let class_doc = format!(" The requests on one `{resource}`.");

        let tokens = quote! {
            use std::collections::HashMap;
            use pyo3::prelude::*;
            use #client_crate::#rust_client_ident;
            use #mod_path::*;
            #wrappers_import
            use crate::codegen::error::{#py_error_type, #py_result_type};
            #futures_import

            #[doc = #class_doc]
            #[pyclass(name = #rust_client_name, #module_arg)]
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
        let py_aggregate_client_ident = format_ident!("Py{}", aggregate_client_name);
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;

        let mut sorted_services = services.iter().collect_vec();
        sorted_services.sort_by(|a, b| a.plan.service_name.cmp(&b.plan.service_name));

        // Services can share a package, so dedupe the model imports.
        let mod_paths = glob_imports(sorted_services.iter().map(|s| s.models_path()));
        // Import the `Py*` model wrappers (re-exported at the models root). All
        // services share one root, so dedupe to a single glob import.
        let wrapper_imports =
            glob_imports(sorted_services.iter().filter_map(|s| s.models_root_path()));

        // Only services with a scoped binding expose a per-service scoped client to import.
        let codegen_imports = sorted_services
            .iter()
            .filter(|s| s.emits_scoped_binding())
            .map(|s| {
                let mod_name = format_ident!("{}", s.plan.base_path);
                let client_name = format_ident!("Py{}", s.client_type().to_string());
                quote! { use crate::codegen::#mod_name::#client_name; }
            });

        let methods = driver::root_client_methods(self, &sorted_services);
        let resource_accessor_methods = driver::resource_accessor_methods(self, &sorted_services);

        let root_module_arg = super::module_arg(self.python, None);
        let helper_modules = {
            let future = self
                .python
                .async_methods
                .then(|| quote! { pub mod py_future; });
            quote! { pub mod error; #future pub mod runtime; }
        };
        let user_agent = self.python.user_agent.as_ref().map(|product| {
            let prefix = format!("{product}/");
            quote! {
                let client = client.with_user_agent(concat!(#prefix, env!("CARGO_PKG_VERSION")));
            }
        });
        let extension_module = self.extension_module(&sorted_services, &py_aggregate_client_ident);

        let tokens = quote! {
            // The aggregate module file carries the root `#[pyclass]` (flat methods →
            // too_many_arguments) and a use-prelude that not every build consumes.
            #![allow(dead_code, unused_imports, clippy::too_many_arguments)]

            #(#service_modules)*
            #helper_modules

            use std::collections::HashMap;
            use futures::stream::TryStreamExt;
            use pyo3::prelude::*;
            use #client_crate::{#aggregate_client_ident};
            use crate::codegen::error::{#py_error_type, #py_result_type};
            #(#mod_paths)*
            #(#wrapper_imports)*
            #(#codegen_imports)*

            /// A client of the API at one base URL.
            #[pyclass(name = #aggregate_client_name, #root_module_arg)]
            pub struct #py_aggregate_client_ident {
                // `pub(crate)` so the binding crate can add `#[pymethods]` of its own.
                pub(crate) client: #aggregate_client_ident
            }

            #[pymethods]
            impl #py_aggregate_client_ident {
                /// A client of the API at `base_url`. `token`, when set, goes in every
                /// request as the bearer token.
                #[new]
                #[pyo3(signature = (base_url, token = None))]
                pub fn new(base_url: String, token: Option<String>) -> PyResult<Self> {
                    let base_url = base_url.parse().map_err(#py_error_type::from)?;
                    let client = match token {
                        Some(token) => #aggregate_client_ident::new_with_token(base_url, token),
                        None => #aggregate_client_ident::new_unauthenticated(base_url),
                    };
                    #user_agent
                    Ok(Self { client })
                }

                #(#methods)*

                #(#resource_accessor_methods)*
            }

            #extension_module
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
        let is_list = method.emit_shape() == crate::analysis::EmitShape::List;
        let args = method
            .required_parameters()
            .filter(|param| !(drop_path && param.is_path_param()))
            .map(|param| {
                // A list method does not take the required `page_token` (`param_plan` drops it;
                // the stream drives the pagination), so seed the builder with an empty token.
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
        let item_ident = items_field.unified_type.type_ident();
        let item_wrapper = wrapper_ident(&item_ident);
        self.methods(
            parts,
            method.binding_method_name(),
            true,
            MethodOutput {
                future: quote! {
                    async move { request.into_stream().try_collect::<::std::vec::Vec<_>>().await }
                },
                value: Some((
                    quote! { ::std::vec::Vec<#item_wrapper> },
                    quote! {
                        result.into_iter().map(#item_wrapper::from).collect::<::std::vec::Vec<_>>()
                    },
                )),
            },
        )
    }

    fn emit_create(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        self.single(
            parts,
            method.binding_method_name(),
            wrap_single_output(method),
        )
    }

    fn emit_get_update(&self, parts: &ShapeParts<'_>) -> TokenStream {
        let method = parts.method;
        self.single(parts, scoped_method_name(parts), wrap_single_output(method))
    }

    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream {
        self.single(parts, scoped_method_name(parts), None)
    }

    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream> {
        let spec = service.accessor_spec()?;
        let method_name = format_ident!("{}", spec.singular);
        let client_name = format_ident!("Py{}", service.client_type().to_string());
        let resource = service
            .resource()
            .map(|r| r.type_name.clone())
            .unwrap_or_else(|| spec.singular.clone());
        let doc = format!(" The requests on one `{resource}`. It sends no request itself.");

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
                #[doc = #doc]
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
                #[doc = #doc]
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

/// The name of a get, update or delete binding method.
fn scoped_method_name(parts: &ShapeParts<'_>) -> Ident {
    match parts.mode {
        BindingMode::Scoped => parts.method.plan.resource_client_method(),
        BindingMode::Flat => parts.method.binding_method_name(),
    }
}

/// `use <path>::*;` for each of `paths`, once each, in path order.
fn glob_imports(paths: impl Iterator<Item = syn::Path>) -> Vec<TokenStream> {
    paths
        .map(|path| (quote! { #path }.to_string(), path))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .map(|path| quote! { use #path::*; })
        .collect()
}

/// What a binding method returns: the future of the bare client result (over the `request`
/// builder), and the Python type and conversion of that `result`; `None` for Python `None`.
struct MethodOutput {
    future: TokenStream,
    value: Option<(TokenStream, TokenStream)>,
}

impl PyBackend<'_> {
    /// A method that returns one model, or nothing.
    fn single(
        &self,
        parts: &ShapeParts<'_>,
        method_name: Ident,
        value: Option<(TokenStream, TokenStream)>,
    ) -> TokenStream {
        let output = MethodOutput {
            future: quote! { request.into_future() },
            value,
        };
        self.methods(parts, method_name, false, output)
    }

    /// The sync binding method and, with [`PythonOptions::async_methods`], its `_async` twin.
    ///
    /// [`PythonOptions::async_methods`]: crate::codegen::PythonOptions::async_methods
    fn methods(
        &self,
        parts: &ShapeParts<'_>,
        method_name: Ident,
        is_list: bool,
        output: MethodOutput,
    ) -> TokenStream {
        let method = parts.method;
        let python = self.python;
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;
        let param_defs = &parts.param_defs;
        let signature = pyo3_signature(method, parts.mode, python.keyword_only);
        let client_call = &parts.client_call;
        let builder_calls = generate_builder_pattern(method, is_list, self);
        let MethodOutput { future, value } = output;
        let (py_type, convert) = match &value {
            Some((py_type, convert)) => (py_type.clone(), convert.clone()),
            None => (quote! { () }, quote! { result }),
        };
        let docs = method_doc_attrs(method);

        let sync = quote! {
            #docs
            #signature
            pub fn #method_name(
                &self,
                py: Python,
                #(#param_defs,)*
            ) -> #py_result_type<#py_type> {
                let request = #client_call #(#builder_calls)*;
                let runtime = crate::codegen::runtime::rt();
                py.detach(|| {
                    #[allow(clippy::let_unit_value)]
                    let result = runtime.block_on(#future)?;
                    Ok::<_, #py_error_type>(#convert)
                })
            }
        };
        if !python.async_methods {
            return sync;
        }

        let async_name = format_ident!("{}_async", method_name);
        let async_doc = format!(
            " Like `{method_name}`, but returns an `asyncio.Future` of the result. Call it in a \
             running event loop."
        );
        let (async_type, async_convert) = value.unwrap_or_else(|| {
            (
                quote! { crate::codegen::py_future::NoResult },
                quote! { crate::codegen::py_future::NoResult },
            )
        });
        quote! {
            #sync

            #[doc = #async_doc]
            #signature
            pub fn #async_name<'py>(
                &self,
                py: Python<'py>,
                #(#param_defs,)*
            ) -> ::pyo3::PyResult<crate::codegen::py_future::PyFuture<'py, #async_type>> {
                let request = #client_call #(#builder_calls)*;
                crate::codegen::py_future::PyFuture::new(py, async move {
                    #[allow(clippy::let_unit_value, unused_variables)]
                    let result = #future
                        .await
                        .map_err(|e| ::pyo3::PyErr::from(#py_error_type::from(e)))?;
                    Ok(#async_convert)
                })
            }
        }
    }
}

/// `#[doc]` attributes for a binding method, with the `Args:`/`Returns:` sections of the `.pyi`.
fn method_doc_attrs(method: &MethodHandler<'_>) -> TokenStream {
    let doc = super::docs::format_method_docstring_with_params(method);
    if doc.trim().is_empty() {
        return quote! {};
    }
    let lines = doc.lines().map(|l| format!(" {l}"));
    quote! { #(#[doc = #lines])* }
}

impl SetterRender for PyBackend<'_> {
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
        let ty = setter.field_type;
        let collection = ty.is_repeated || matches!(ty.base_type, BaseType::Map(..));
        if let Some(json) = json_param(ty) {
            let value = if collection {
                let items =
                    super::model_conversions::json_from_py(json, ty, quote! { #param_ident }, true);
                quote! {
                    match #param_ident {
                        ::core::option::Option::Some(#param_ident) => #items,
                        ::core::option::Option::None => ::std::vec::Vec::new(),
                    }
                }
            } else {
                let from_py = format_ident!("json_{}_from_py", json.fn_suffix());
                quote! { #param_ident.as_ref().map(#from_py).transpose()? }
            };
            return quote! { .#with_method(#value) };
        }
        // A map or list left out is the empty collection; message and enum elements are
        // converted from their `Py` wrappers.
        if collection {
            let value = quote! { #param_ident.unwrap_or_default() };
            let value = if message_map_value(ty).is_some() {
                convert_map_values(value)
            } else if param_needs_conversion(ty) {
                quote! {
                    #value
                        .into_iter()
                        .map(::core::convert::Into::into)
                        .collect::<::std::vec::Vec<_>>()
                }
            } else {
                value
            };
            return quote! { .#with_method(#value) };
        }
        if ty.is_singular_bytes() {
            let value = bytes_to_vec(quote! { #param_ident }, true);
            return quote! { .#with_method(#value) };
        }
        if param_needs_conversion(ty) {
            // Optional singular message/enum: `Option<PyWrapper>` → `Option<Inner>` for
            // the builder, which accepts `impl Into<Option<Inner>>`.
            return quote! { .#with_method(#param_ident.map(::core::convert::Into::into)) };
        }
        quote! { .#with_method(#param_ident) }
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
            if json_param(p.field_type()).is_some() {
                return if p.is_optional() {
                    quote! { #param_name: ::core::option::Option<::pyo3::Bound<'_, ::pyo3::PyAny>> }
                } else {
                    quote! { #param_name: ::pyo3::Bound<'_, ::pyo3::PyAny> }
                };
            }
            let rust_type = method.field_type(p.field_type(), RenderContext::PythonParameter);
            let py_type = wrap_param_type(p.field_type(), &rust_type);
            // A required list or map is not an `Option`.
            let py_type = if p.is_optional() || !is_collection(p.field_type()) {
                py_type
            } else {
                strip_option(py_type)
            };
            quote! { #param_name: #py_type }
        })
        .collect()
}

/// The free-form JSON type of a param, which Python passes as a plain object.
fn json_param(ty: &UnifiedType) -> Option<JsonType> {
    JsonType::of_field(ty)
}

/// The value type of a `map<K, Message>` param, whose values cross the boundary as wrappers.
fn message_map_value(ty: &UnifiedType) -> Option<&UnifiedType> {
    match &ty.base_type {
        BaseType::Map(_, value) if matches!(value.base_type, BaseType::Message(_)) => Some(value),
        _ => None,
    }
}

/// Convert the wrapper values of the map param `expr` to the bare models.
fn convert_map_values(expr: TokenStream) -> TokenStream {
    quote! {
        #expr
            .into_iter()
            .map(|(k, v)| (k, v.0))
            .collect::<::std::collections::HashMap<_, _>>()
    }
}

/// Whether a param's base type is a model (message/enum) that has a `Py` wrapper, so its value
/// must be converted (`.into()`) when forwarded to the Rust client.
fn param_needs_conversion(ty: &UnifiedType) -> bool {
    matches!(
        ty.base_type,
        BaseType::Message(_) | BaseType::Enum(_) | BaseType::OneOf(_)
    )
}

/// The type of a singular `bytes` argument: `PyBackedBytes`, which does not copy the Python
/// `bytes` element by element as a `Vec<u8>` would.
pub(super) fn py_bytes_input(optional: bool) -> TokenStream {
    match optional {
        true => quote! { ::core::option::Option<::pyo3::pybacked::PyBackedBytes> },
        false => quote! { ::pyo3::pybacked::PyBackedBytes },
    }
}

/// The `Vec<u8>` of the [`py_bytes_input`] value `expr`.
pub(super) fn bytes_to_vec(expr: TokenStream, optional: bool) -> TokenStream {
    match optional {
        true => quote! { #expr.map(|v| v.to_vec()) },
        false => quote! { #expr.to_vec() },
    }
}

/// Rewrite a rendered param `syn::Type` so a message/enum base type becomes its `Py` wrapper,
/// preserving the `Option`/`Vec` wrapping; `bytes` becomes `PyBackedBytes`. Other scalars,
/// maps and `()` are returned unchanged.
fn wrap_param_type(ty: &UnifiedType, rendered: &syn::Type) -> TokenStream {
    if ty.is_singular_bytes() {
        return py_bytes_input(ty.is_optional);
    }
    if let Some(value) = message_map_value(ty) {
        let wrapper = qualified_wrapper_ident(value);
        return quote! {
            ::core::option::Option<::std::collections::HashMap<::std::string::String, #wrapper>>
        };
    }
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

/// Whether a param is a list or a map.
fn is_collection(ty: &UnifiedType) -> bool {
    ty.is_repeated || matches!(ty.base_type, BaseType::Map(..))
}

/// `T` for a rendered `Option<T>`; any other type unchanged.
fn strip_option(ty: TokenStream) -> TokenStream {
    let Ok(syn::Type::Path(path)) = syn::parse2::<syn::Type>(ty.clone()) else {
        return ty;
    };
    let Some(last) = path.path.segments.last() else {
        return ty;
    };
    match &last.arguments {
        syn::PathArguments::AngleBracketed(args) if last.ident == "Option" => {
            match args.args.first() {
                Some(syn::GenericArgument::Type(inner)) => quote! { #inner },
                _ => ty,
            }
        }
        _ => ty,
    }
}

/// Convert the Python-side value `expr` of a required param into what the Rust client takes.
/// Scalars pass through.
fn convert_param_value(ty: &UnifiedType, expr: TokenStream) -> TokenStream {
    if let Some(json) = json_param(ty) {
        if ty.is_repeated || matches!(ty.base_type, BaseType::Map(..)) {
            return super::model_conversions::json_from_py(json, ty, expr, true);
        }
        let from_py = format_ident!("json_{}_from_py", json.fn_suffix());
        return quote! { #from_py(&#expr)? };
    }
    if message_map_value(ty).is_some() {
        return convert_map_values(expr);
    }
    if ty.is_singular_bytes() {
        return bytes_to_vec(expr, ty.is_optional);
    }
    if !param_needs_conversion(ty) {
        return expr;
    }
    match (ty.is_repeated, ty.is_optional) {
        (true, _) => quote! {
            #expr.into_iter().map(::core::convert::Into::into).collect()
        },
        (false, true) => quote! { #expr.map(::core::convert::Into::into) },
        (false, false) => quote! { #expr.into() },
    }
}

/// The `#[pyo3(signature = (...))]` attribute for a method, derived from its param plan.
/// With `keyword_only`, a `*` goes before the first optional parameter.
fn pyo3_signature(
    method: &MethodHandler<'_>,
    mode: BindingMode,
    keyword_only: bool,
) -> TokenStream {
    let mut signature_parts = Vec::new();
    let mut star = keyword_only;
    for p in method.param_plan(mode).iter() {
        let ident = p.field_ident();
        if p.is_optional() {
            if std::mem::take(&mut star) {
                signature_parts.push(quote! { * });
            }
            signature_parts.push(quote! { #ident = None });
        } else {
            signature_parts.push(quote! { #ident });
        }
    }
    if signature_parts.is_empty() {
        quote! {}
    } else {
        quote! {
            #[pyo3(signature = (#(#signature_parts),*))]
        }
    }
}
