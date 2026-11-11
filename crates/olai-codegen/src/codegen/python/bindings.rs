//! PyO3 binding generation for protobuf-defined services.
//!
//! The dispatch and "which methods land where" logic is shared with the Node emitter via
//! [`crate::codegen::bindings`]; this module supplies the PyO3-specific scaffold, marshaling, and
//! method bodies through [`PyBackend`]'s [`BindingBackend`] impl.

use std::collections::BTreeMap;

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
    if method.plan.metadata.server_streaming {
        Some((
            quote! { Vec<#w> },
            quote! { result.into_iter().map(#w::from).collect::<Vec<_>>() },
        ))
    } else {
        Some((quote! { #w }, quote! { #w::from(result) }))
    }
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
        let bindings = service.config.bindings_config();
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
        // The `TokenProvider` class of `token_provider=`, with `CloudClient`.
        let token_provider_export = first.config.uses_cloud_client().then(|| {
            quote! {
                #[pymodule_export]
                use super::RustTokenProvider;
            }
        });
        // Paths are validated by `CodeGenConfig::validate`.
        let export = |path: &String| {
            let path: syn::Path = syn::parse_str(path).expect("validated export path");
            quote! {
                #[pymodule_export]
                use #path;
            }
        };
        let exports = self.python.exports.iter().map(export);
        // The hand-written submodules of the binding crate, under their last segment.
        let extra_submodules = self.python.submodules.iter().map(export);
        let segments = groups.keys().map(|segment| segment.to_string()).chain(
            (self.python.submodules.iter())
                .map(|path| path.rsplit("::").next().unwrap_or(path).to_string()),
        );
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

                #token_provider_export

                #[pymodule_export]
                use crate::codegen::error::{#(#exceptions),*};

                #(#exports)*

                #(#submodules)*

                #(#extra_submodules)*

                /// Put the submodules in `sys.modules`. The import system puts the module there
                /// before it runs this. Not handed the module, so the stubs still describe the
                /// module in full.
                #[pymodule_init]
                fn init(py: Python<'_>) -> PyResult<()> {
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

impl PyBackend<'_> {
    /// The constructor of the root client with `CloudClient`: a token, a `token_provider`, or
    /// else the [`default_signer`](PythonOptions::default_signer).
    fn cloud_constructor(&self, client_crate: &Ident, aggregate: &Ident) -> TokenStream {
        let py_error_type = &self.py_error_type;
        let to_py_error = quote! {
            |e: ::olai_http::Error| #py_error_type::from(#client_crate::error::Error::from(e))
        };
        // Paths are validated by `CodeGenConfig::validate`.
        let default_signer = match self.python.default_signer.as_deref() {
            Some(path) => {
                let hook: syn::Path = syn::parse_str(path).expect("validated signer path");
                quote! {
                    py.detach(|| crate::codegen::runtime::rt().block_on(#hook(&base_url)))
                        .map_err(#to_py_error)?
                }
            }
            None => quote! { None },
        };
        let user_agent = self.python.user_agent.as_ref().map(|product| {
            let prefix = format!("{product}/");
            quote! {
                let options = options.with_config(
                    ::olai_http::ClientConfigKey::UserAgent,
                    concat!(#prefix, env!("CARGO_PKG_VERSION")),
                );
            }
        });
        quote! {
            /// A client of the API at `base_url`. `token`, when set, goes in every request as
            /// the bearer token. `token_provider` is asked for the token instead: a
            /// `TokenProvider`, or a callable that returns the token, or a pair of the token
            /// and the time it expires, in seconds since the epoch; a token with its expiry is
            /// kept until a minute before it, a bare token is asked for at every request.
            #[new]
            #[pyo3(signature = (base_url, token = None, token_provider = None))]
            pub fn new(
                py: Python<'_>,
                base_url: String,
                token: Option<String>,
                token_provider: Option<::pyo3::Py<::pyo3::PyAny>>,
            ) -> PyResult<Self> {
                let base_url: ::url::Url = base_url.parse().map_err(#py_error_type::from)?;
                let signer: Option<::std::sync::Arc<dyn ::olai_http::RequestSigner>> =
                    match (token, token_provider) {
                        (Some(_), Some(_)) => {
                            return Err(::pyo3::exceptions::PyValueError::new_err(
                                "token and token_provider cannot both be set",
                            ));
                        }
                        (Some(token), None) => Some(::std::sync::Arc::new(
                            ::olai_http::bearer::BearerProviderSigner(
                                ::olai_http::bearer::BearerAuth::static_token(token),
                            ),
                        )),
                        (None, Some(provider)) => {
                            Some(match provider.bind(py).cast::<RustTokenProvider>() {
                                Ok(provider) => provider.get().signer.clone(),
                                Err(_) => ::std::sync::Arc::new(PyTokenProvider::new(provider)),
                            })
                        }
                        (None, None) => #default_signer,
                    };
                let options = #aggregate::client_options();
                #user_agent
                let retry = ::olai_http::RetryConfig::default();
                let client = match signer {
                    Some(signer) => ::olai_http::CloudClient::new_with_request_signer(
                        signer, &options, retry, None,
                    ),
                    None => ::olai_http::CloudClient::new_unauthenticated_with_options(
                        &options, retry, None,
                    ),
                }
                .map_err(#to_py_error)?;
                Ok(Self { client: #aggregate::new(client, base_url) })
            }
        }
    }
}

/// The classes of `token_provider=` with `CloudClient`: `TokenProvider`, a signer made in
/// Rust, and the signer of a Python callable.
fn token_provider_classes(root_module_arg: &TokenStream) -> TokenStream {
    quote! {
        /// A token provider made in Rust, for `token_provider=`. The binding crate makes one
        /// from any `olai_http::RequestSigner`, such as a bearer provider in a
        /// `BearerProviderSigner`.
        #[pyclass(name = "TokenProvider", frozen, #root_module_arg)]
        pub struct RustTokenProvider {
            pub(crate) signer: ::std::sync::Arc<dyn ::olai_http::RequestSigner>,
        }

        impl RustTokenProvider {
            pub fn new(signer: ::std::sync::Arc<dyn ::olai_http::RequestSigner>) -> Self {
                Self { signer }
            }
        }

        /// The bearer tokens of a `token_provider` callable: the token, or a
        /// `(token, expires_at)` pair with the expiry in seconds since the epoch, kept until a
        /// minute before it. The callable runs on a blocking thread, one call at a time: it may
        /// block, and call the blocking methods of this module.
        pub struct PyTokenProvider {
            provider: ::std::sync::Arc<::pyo3::Py<::pyo3::PyAny>>,
            kept: ::futures::lock::Mutex<Option<(String, ::std::time::Instant)>>,
        }

        impl PyTokenProvider {
            fn new(provider: ::pyo3::Py<::pyo3::PyAny>) -> Self {
                Self {
                    provider: ::std::sync::Arc::new(provider),
                    kept: ::futures::lock::Mutex::new(None),
                }
            }

            /// Ask the callable: the token, and its expiry when it gives one.
            fn ask(provider: &::pyo3::Py<::pyo3::PyAny>) -> PyResult<(String, Option<f64>)> {
                ::pyo3::Python::attach(|py| {
                    let value = provider.bind(py).call0()?;
                    if let Ok(token) = value.extract::<String>() {
                        return Ok((token, None));
                    }
                    let (token, expires_at) =
                        value.extract::<(String, f64)>().map_err(|_| {
                            ::pyo3::exceptions::PyTypeError::new_err(
                                "token_provider must return the token, or a (token, \
                                 expires_at) pair with the expiry in seconds since the epoch",
                            )
                        })?;
                    Ok((token, Some(expires_at)))
                })
            }

            /// The token: the kept one, else the callable's.
            async fn token(&self) -> ::olai_http::Result<String> {
                let failed = |source: Box<dyn ::std::error::Error + Send + Sync>| {
                    ::olai_http::Error::Generic { source }
                };
                // The requests that wait take the token of the one that asked.
                let mut kept = self.kept.lock().await;
                if let Some((token, until)) = kept.as_ref()
                    && ::std::time::Instant::now() < *until
                {
                    return Ok(token.clone());
                }
                // Not on the thread of the request: a blocking method of this module that the
                // callable calls starts a runtime there.
                let provider = self.provider.clone();
                let (token, expires_at) =
                    ::tokio::task::spawn_blocking(move || Self::ask(&provider))
                        .await
                        .map_err(|e| failed(Box::new(e)))?
                        .map_err(|e| failed(Box::new(e)))?;
                if let Some(expires_at) = expires_at {
                    let now = ::std::time::SystemTime::now()
                        .duration_since(::std::time::UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64());
                    let left = expires_at - now - 60.0;
                    let left = if left.is_finite() { left.clamp(0.0, 1e9) } else { 0.0 };
                    let until =
                        ::std::time::Instant::now() + ::std::time::Duration::from_secs_f64(left);
                    *kept = Some((token.clone(), until));
                }
                Ok(token)
            }
        }

        impl ::std::fmt::Debug for PyTokenProvider {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct("PyTokenProvider").finish_non_exhaustive()
            }
        }

        impl ::olai_http::RequestSigner for PyTokenProvider {
            fn sign<'a>(
                &'a self,
                request: ::olai_http::reqwest::RequestBuilder,
            ) -> ::futures::future::BoxFuture<
                'a,
                ::olai_http::Result<::olai_http::reqwest::RequestBuilder>,
            >
            {
                Box::pin(async move { Ok(request.bearer_auth(self.token().await?)) })
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
        let bindings = service.config.bindings_config();

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
            .expect("binding output has a service")
            .config
            .bindings_config();

        let aggregate_client_name = &bindings.aggregate_client_name;
        let client_crate = format_ident!("{}", bindings.client_crate_name);
        let aggregate_client_ident = format_ident!("{}", aggregate_client_name);
        let py_aggregate_client_ident = format_ident!("Py{}", aggregate_client_name);
        let py_error_type = &self.py_error_type;
        let py_result_type = &self.py_result_type;

        let sorted_services = driver::sorted_services(services);

        // Services can share a package, so dedupe the model imports.
        let mod_paths = glob_imports(sorted_services.iter().map(|s| s.models_path()));
        // Import the `Py*` model wrappers (re-exported at the models root). All
        // services share one root, so dedupe to a single glob import.
        let wrapper_imports =
            glob_imports(sorted_services.iter().filter_map(|s| s.models_root_path()));

        let codegen_imports = driver::scoped_client_imports(&sorted_services, "Py");

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
        let cloud_client = sorted_services
            .first()
            .is_some_and(|s| s.config.uses_cloud_client());
        let constructor = if cloud_client {
            self.cloud_constructor(&client_crate, &aggregate_client_ident)
        } else {
            quote! {
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
            }
        };
        let token_provider = cloud_client.then(|| token_provider_classes(&root_module_arg));

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
                #constructor

                #(#methods)*

                #(#resource_accessor_methods)*
            }

            #token_provider

            #extension_module
        };

        format_tokens(tokens)
    }

    fn param_defs(&self, method: &MethodHandler<'_>, mode: BindingMode) -> Vec<TokenStream> {
        python_param_defs(method, mode)
    }

    fn client_call(&self, method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream {
        let (method_name, drop_path) = driver::client_method(method, mode);
        let args = method
            .required_parameters()
            .filter(|param| !(drop_path && param.is_path_param()))
            .map(|param| {
                // `param_plan` drops it too.
                if driver::is_seeded_page_token(method, param) {
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
        self.single(parts, parts.unary_method_name(), wrap_single_output(method))
    }

    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream {
        self.single(parts, parts.unary_method_name(), None)
    }

    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream> {
        let spec = service.accessor_spec()?;
        let resource = service
            .resource()
            .map(|r| r.type_name.clone())
            .unwrap_or_else(|| spec.singular.clone());
        let doc = format!(" The requests on one `{resource}`. It sends no request itself.");
        driver::resource_accessor(service, quote! { #[doc = #doc] }, "Py")
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
                #[allow(clippy::let_unit_value)]
                let result = crate::codegen::runtime::block_on(py, #future)??;
                Ok(#convert)
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
        let collection = is_collection(ty);
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
                let from_py = super::model_conversions::json_from_py_fn(json);
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
        if is_collection(ty) {
            return super::model_conversions::json_from_py(json, ty, expr, true);
        }
        let from_py = super::model_conversions::json_from_py_fn(json);
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
