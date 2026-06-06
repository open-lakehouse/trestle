//! Generation of the Rust *aggregate root client* (e.g. `UnityCatalogClient`).
//!
//! The aggregate client is the hand-written-no-more root the language bindings call into.
//! Generated Python/Node aggregate bindings invoke `self.client.<method>(...)` where `self.client`
//! is this aggregate; the method surface emitted here is built to EXACTLY MATCH those call sites
//! (see [`super::python::bindings::collection_client_struct`] and
//! [`super::python::bindings::generate_resource_accessor_method`], which this file mirrors).
//!
//! Structure mirrors the previously hand-written root client:
//! - one field per service, named after the service `base_path`, holding the generated low-level
//!   per-service client ([`ServiceHandler::client_type`]);
//! - `new(client, base_url)` constructing each per-service client;
//! - for each service, collection-style builder constructors (and, for resource-scoped services, a
//!   resource accessor returning the hand-written scoped client); resource-less ("flat") services
//!   contribute *every* method as a flat builder constructor.

use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::config::CodeGenConfig;
use super::format_tokens;
use super::{BindingMode, MethodHandler, ServiceHandler};
use crate::analysis::{GenerationPlan, RequestParam};
use crate::parsing::CodeGenMetadata;
use crate::parsing::types::RenderContext;

/// Generate the top-level `client.rs` aggregate root client.
///
/// Returns `None` when `config.bindings` is `None` (no aggregate name configured), in which case
/// no `client.rs` should be written.
pub(crate) fn generate(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
) -> crate::error::Result<Option<String>> {
    let Some(bindings) = config.bindings.as_ref() else {
        return Ok(None);
    };

    let aggregate_ident = format_ident!("{}", bindings.aggregate_client_name);

    let mut services = plan
        .services
        .iter()
        .map(|service| ServiceHandler {
            plan: service,
            metadata,
            config,
        })
        .collect_vec();
    services.sort_by(|a, b| a.plan.service_name.cmp(&b.plan.service_name));

    // --- Struct fields: one per service, named after the service base_path. ---
    let fields = services.iter().map(|s| {
        let field = format_ident!("{}", s.plan.base_path);
        let client_ty = low_level_client_path(s);
        quote! { #field: #client_ty }
    });

    // --- Constructor body: build each per-service client, then move them into Self. ---
    let constructor_bindings = services.iter().map(|s| {
        let field = format_ident!("{}", s.plan.base_path);
        let client_ty = low_level_client_path(s);
        quote! { let #field = #client_ty::new(client.clone(), base_url.clone()); }
    });
    let field_names = services.iter().map(|s| {
        let field = format_ident!("{}", s.plan.base_path);
        quote! { #field }
    });

    // --- Methods: collection constructors, flat constructors, resource accessors. ---
    let methods = services.iter().flat_map(|s| service_methods(s));

    // --- Low-level passthrough accessors: `<base>_client(&self) -> <LowLevelClient>`. ---
    // These hand back a clone of the per-service low-level client so callers (e.g. a hybrid
    // proxy server, or hand-written ergonomic wrappers) can drive requests directly. Emitted for
    // every service since the fields are private.
    let passthrough_accessors = services.iter().map(|s| {
        let field = format_ident!("{}", s.plan.base_path);
        let accessor = format_ident!("{}_client", s.plan.base_path);
        let client_ty = low_level_client_path(s);
        let doc = format!(
            "Low-level `{}` client exposing request/response passthrough methods.",
            s.plan.base_path
        );
        quote! {
            #[doc = #doc]
            pub fn #accessor(&self) -> #client_ty {
                self.#field.clone()
            }
        }
    });

    let imports = generate_imports(&services);

    let tokens = quote! {
        #imports

        #[derive(Clone)]
        pub struct #aggregate_ident {
            #(#fields,)*
        }

        impl #aggregate_ident {
            /// Create a new aggregate client from a cloud client and base URL.
            pub fn new(client: CloudClient, mut base_url: Url) -> Self {
                if !base_url.path().ends_with('/') {
                    base_url.set_path(&format!("{}/", base_url.path()));
                }
                #(#constructor_bindings)*
                Self {
                    #(#field_names,)*
                }
            }

            /// Create a new aggregate client with no authentication.
            pub fn new_unauthenticated(base_url: Url) -> Self {
                Self::new(CloudClient::new_unauthenticated(), base_url)
            }

            /// Create a new aggregate client authenticating with a bearer token.
            pub fn new_with_token(base_url: Url, token: impl ToString) -> Self {
                Self::new(CloudClient::new_with_token(token), base_url)
            }

            #(#passthrough_accessors)*

            #(#methods)*
        }
    };

    Ok(Some(format_tokens(tokens)?))
}

/// The fully-qualified path to a service's generated low-level per-service client,
/// e.g. `crate::codegen::catalogs::CatalogClient`.
fn low_level_client_path(service: &ServiceHandler<'_>) -> TokenStream {
    let module = format_ident!("{}", service.plan.base_path);
    let client = service.client_type();
    quote! { crate::codegen::#module::#client }
}

/// Emit `use` statements the aggregate needs: cloud client + url, per-service low-level clients,
/// builder types, models (for enum/message param types), and hand-written scoped clients.
fn generate_imports(services: &[ServiceHandler<'_>]) -> TokenStream {
    // Per-service low-level clients and their builders both live under `crate::codegen::<base>`.
    let codegen_imports = services.iter().map(|s| {
        let module = format_ident!("{}", s.plan.base_path);
        quote! {
            use crate::codegen::#module::*;
        }
    });

    // Models, for enum/message parameter types that appear in builder `::new` signatures.
    let model_imports = services
        .iter()
        .map(|s| s.models_path())
        .unique_by(|p| quote! { #p }.to_string())
        .map(|mod_path| quote! { use #mod_path::*; });

    // Hand-written scoped clients live in the consuming crate. The crate's `lib.rs` re-exports
    // them at crate root (`pub use catalogs::*` brings `CatalogClient` into `crate::`), so we
    // reference them as `crate::<Singular>Client`.
    let scoped_client_imports = services
        .iter()
        .filter(|s| s.is_resource_scoped())
        .map(|s| {
            let client = s.client_type();
            quote! { use crate::#client; }
        })
        .unique_by(|t| t.to_string());

    quote! {
        use olai_http::CloudClient;
        use url::Url;
        #(#codegen_imports)*
        #(#model_imports)*
        #(#scoped_client_imports)*
    }
}

/// Emit all aggregate methods contributed by a single service.
fn service_methods(service: &ServiceHandler<'_>) -> Vec<TokenStream> {
    let field = format_ident!("{}", service.plan.base_path);
    let mode = service.binding_mode();

    let mut methods = Vec::new();
    for method in service.methods() {
        match mode {
            // Scoped services contribute only collection-style methods (list/create/factory);
            // their instance methods live on the hand-written scoped client.
            BindingMode::Scoped => {
                if method.is_collection_method() {
                    methods.push(collection_method(&method, &field));
                }
            }
            // Resource-less services contribute every method, lowered flat (all params passed,
            // including path params).
            BindingMode::Flat => {
                methods.push(flat_method(&method, &field));
            }
        }
    }

    if let Some(accessor) = resource_accessor_method(service, &field) {
        methods.push(accessor);
    }

    methods
}

/// Render the constructor parameter list (name: type) and the forwarding arg list for a builder
/// constructor, given the params to forward (in `Builder::new` order).
fn builder_params(
    method: &MethodHandler<'_>,
    params: &[&RequestParam],
) -> (Vec<TokenStream>, Vec<TokenStream>) {
    let param_defs = params
        .iter()
        .map(|p| {
            let ident = p.field_ident();
            // Match the builder's `::new` param types (RenderContext::Constructor) so the forward
            // call type-checks: `impl Into<String>` for required strings, concrete types otherwise.
            let ty = method.field_type(p.field_type(), RenderContext::Constructor);
            quote! { #ident: #ty }
        })
        .collect();
    let args = params
        .iter()
        .map(|p| {
            let ident = p.field_ident();
            quote! { #ident }
        })
        .collect();
    (param_defs, args)
}

/// Emit a collection-style method (list / create / factory) on a resource-scoped service.
///
/// Forwards all required params (in `required_parameters()` order) to the builder's `::new`, which
/// takes exactly those params. The binding method name mirrors what the Python aggregate calls.
fn collection_method(method: &MethodHandler<'_>, field: &proc_macro2::Ident) -> TokenStream {
    let method_name = method.binding_method_name();
    let builder_ty = method.builder_type();
    let params: Vec<&RequestParam> = method.required_parameters().collect();
    let (param_defs, args) = builder_params(method, &params);

    quote! {
        pub fn #method_name(&self, #(#param_defs),*) -> #builder_ty {
            #builder_ty::new(self.#field.clone(), #(#args),*)
        }
    }
}

/// Emit a method for a resource-less ("flat") service: pass ALL required params (incl. path) to the
/// builder constructor. Covers list/create/get/update/delete/custom uniformly.
fn flat_method(method: &MethodHandler<'_>, field: &proc_macro2::Ident) -> TokenStream {
    let method_name = method.binding_method_name();
    let builder_ty = method.builder_type();
    let params: Vec<&RequestParam> = method.required_parameters().collect();
    let (param_defs, args) = builder_params(method, &params);

    quote! {
        pub fn #method_name(&self, #(#param_defs),*) -> #builder_ty {
            #builder_ty::new(self.#field.clone(), #(#args),*)
        }
    }
}

/// Emit the resource accessor for a resource-scoped service, mirroring
/// [`super::python::bindings::generate_resource_accessor_method`].
///
/// Returns `<singular>(<params>: impl Into<String>...) -> <Singular>Client`, constructing the
/// hand-written scoped client. When the resource is **nested** (its accessor takes parent
/// components in addition to its own name), also emits `<singular>_from_full_name` and joins the
/// params with `.` to build the full name.
///
/// Nesting comes from the shared [`super::AccessorSpec`] (built from
/// `ServicePlan::resource_accessor_params`): the param list is the parent chain plus the leaf name,
/// so `params.len() > 1` means the resource has ancestors and its full name is splittable. A
/// top-level resource (e.g. catalog, tag policy) has a single name component with nothing to
/// decompose, so no `from_full_name` accessor is emitted. This is the single converged nesting rule
/// shared by the Rust aggregate and all language bindings.
fn resource_accessor_method(
    service: &ServiceHandler<'_>,
    field: &proc_macro2::Ident,
) -> Option<TokenStream> {
    let spec = service.accessor_spec()?;
    let method_name = format_ident!("{}", spec.singular);
    let client_ty = service.client_type();

    let param_idents: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
    let param_defs: Vec<_> = param_idents
        .iter()
        .map(|id| quote! { #id: impl ToString })
        .collect();

    if spec.nested {
        let from_full_name = format_ident!("{}_from_full_name", spec.singular);
        let format_str = spec.join_format();
        Some(quote! {
            pub fn #method_name(&self, #(#param_defs),*) -> #client_ty {
                let full_name = format!(#format_str, #(#param_idents.to_string()),*);
                self.#from_full_name(full_name)
            }

            pub fn #from_full_name(&self, full_name: impl ToString) -> #client_ty {
                #client_ty::new_from_full_name(full_name, self.#field.clone())
            }
        })
    } else {
        let args = param_idents.iter().map(|id| quote! { #id });
        Some(quote! {
            pub fn #method_name(&self, #(#param_defs),*) -> #client_ty {
                #client_ty::new(#(#args),*, self.#field.clone())
            }
        })
    }
}
