//! Generation of the Rust *aggregate root client* (e.g. `UnityCatalogClient`).
//!
//! The aggregate client is the hand-written-no-more root the language bindings call into.
//! Generated Python/Node aggregate bindings invoke `self.client.<method>(...)` where `self.client`
//! is this aggregate; the method surface emitted here is built to EXACTLY MATCH those call sites
//! (see [`super::python::bindings::collection_client_struct`] and
//! [`super::python::bindings::generate_resource_accessor_method`], which this file mirrors).
//!
//! Structure:
//! - stores only a `CloudClient` + base `Url`; per-service low-level clients are built **on demand**
//!   (they hold just those two cheaply-cloneable values), so nothing is allocated per service in
//!   `new(client, base_url)`;
//! - for each service, collection-style builder constructors (and, for resource-scoped services, a
//!   resource accessor returning the generated scoped client — see [`super::resource_client`]);
//!   resource-less ("flat") services contribute *every* method as a flat builder constructor.

use itertools::Itertools;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::config::CodeGenConfig;
use super::{BindingMode, MethodHandler, ServiceHandler, doc_tokens, format_tokens};
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

    // The aggregate root client is REST-shaped: it stores a `CloudClient` + base `Url` and builds
    // HTTP low-level clients on demand. It is only meaningful for the REST client; when REST is not
    // emitted there is no REST per-service client for it to aggregate. (A ConnectRPC aggregate is a
    // follow-up; ConnectRPC dispatch owns its own transport.)
    if !config.client_protocols.rest {
        return Ok(None);
    }

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

    // --- Methods: collection constructors, flat constructors, resource accessors. ---
    let methods = services.iter().flat_map(|s| service_methods(s));

    // --- Low-level passthrough accessors: `<base>_client(&self) -> <LowLevelClient>`. ---
    // These build a fresh per-service low-level client (cheap: `CloudClient`/`Url` are O(1) to
    // clone) so callers (e.g. a hybrid proxy server, or hand-written ergonomic wrappers) can drive
    // requests directly.
    let passthrough_accessors = services.iter().map(|s| {
        let accessor = format_ident!("{}_client", s.plan.base_path);
        let client_ty = low_level_client_path(s);
        let ctor = low_level_client_ctor(s);
        let doc = format!(
            "Low-level `{}` client exposing request/response passthrough methods.",
            s.plan.base_path
        );
        quote! {
            #[doc = #doc]
            pub fn #accessor(&self) -> #client_ty {
                #ctor
            }
        }
    });

    let (transport_import, transport_ident) = super::client::transport_tokens(config);

    // Convenience constructors are transport-specific:
    // - `CloudClient` (native): unauthenticated + bearer-token variants.
    // - `WasmClient` (browser): the browser attaches the session, so only a
    //   parameterless `new_in_browser(base_url)`.
    // In dual-transport mode each ctor carries its own target-arch `cfg` (the
    // gate must sit on every fn, not the group); in single (default cloud) mode
    // the cloud ctors are emitted ungated.
    let dual = config.dual_transport();
    let native_cfg = if dual {
        quote! { #[cfg(not(target_arch = "wasm32"))] }
    } else {
        quote! {}
    };
    let cloud_ctors = quote! {
        #native_cfg
        /// Create a new aggregate client with no authentication.
        pub fn new_unauthenticated(base_url: Url) -> Self {
            Self::new(#transport_ident::new_unauthenticated(), base_url)
        }

        #native_cfg
        /// Create a new aggregate client authenticating with a bearer token.
        // `token` stays `impl ToString` to match `CloudClient::new_with_token`'s own signature.
        pub fn new_with_token(base_url: Url, token: impl ToString) -> Self {
            Self::new(#transport_ident::new_with_token(token), base_url)
        }
    };
    let wasm_ctor = quote! {
        #[cfg(target_arch = "wasm32")]
        /// Create a new aggregate client. The browser supplies the session
        /// (cookies / forwarded auth) on each request.
        pub fn new_in_browser(base_url: Url) -> Self {
            Self::new(#transport_ident::new(), base_url)
        }
    };

    let convenience_ctors = if dual {
        quote! {
            #cloud_ctors
            #wasm_ctor
        }
    } else if config.uses_default_transport() {
        // Single cloud transport: emit the cloud ctors bare.
        cloud_ctors
    } else {
        // Single custom transport: only the generic `new(transport, base_url)`.
        quote! {}
    };

    let imports = generate_imports(&services, &transport_import);

    let tokens = quote! {
        // The per-service (`crate::codegen::<svc>::*`) and model globs below are
        // convenience-wide; the aggregate root only references a subset directly,
        // so a model whose types it never names (common under buffa) would trip
        // `unused_imports` under `-D warnings`.
        #![allow(unused_imports)]
        #imports

        #[derive(Clone)]
        pub struct #aggregate_ident {
            client: #transport_ident,
            base_url: Url,
        }

        impl #aggregate_ident {
            /// Create a new aggregate client from a cloud client and base URL.
            ///
            /// Per-service clients are constructed on demand (they only hold a cheaply-cloneable
            /// `CloudClient` + `Url`), so nothing is allocated per service here.
            pub fn new(client: #transport_ident, mut base_url: Url) -> Self {
                if !base_url.path().ends_with('/') {
                    base_url.set_path(&format!("{}/", base_url.path()));
                }
                Self { client, base_url }
            }

            #convenience_ctors

            #(#passthrough_accessors)*

            #(#methods)*
        }
    };

    Ok(Some(format_tokens(tokens)?))
}

/// The fully-qualified path to a service's generated low-level per-service client,
/// e.g. `crate::codegen::catalogs::CatalogServiceClient`.
fn low_level_client_path(service: &ServiceHandler<'_>) -> TokenStream {
    let module = format_ident!("{}", service.plan.base_path);
    let client = service.low_level_client_type(crate::codegen::ClientProtocol::Rest);
    quote! { crate::codegen::#module::#client }
}

/// An expression constructing a fresh low-level per-service client from the aggregate's stored
/// `CloudClient` + `base_url` (both cheaply cloneable), e.g.
/// `crate::codegen::catalogs::CatalogServiceClient::new(self.client.clone(), self.base_url.clone())`.
fn low_level_client_ctor(service: &ServiceHandler<'_>) -> TokenStream {
    let path = low_level_client_path(service);
    quote! { #path::new(self.client.clone(), self.base_url.clone()) }
}

/// Emit `use` statements the aggregate needs: cloud client + url, per-service low-level clients,
/// builder types, models (for enum/message param types), and hand-written scoped clients.
fn generate_imports(
    services: &[ServiceHandler<'_>],
    transport_import: &TokenStream,
) -> TokenStream {
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

    // The generated scoped clients (`CatalogClient`, …) live alongside the low-level clients and
    // builders under `crate::codegen::<base>`, so the `codegen_imports` globs above already bring
    // them into scope — no separate import needed.
    quote! {
        #transport_import
        use url::Url;
        #(#codegen_imports)*
        #(#model_imports)*
    }
}

/// Emit all aggregate methods contributed by a single service.
fn service_methods(service: &ServiceHandler<'_>) -> Vec<TokenStream> {
    let mode = service.binding_mode();

    let mut methods = Vec::new();
    // The aggregate is REST-only, so it surfaces only routed methods.
    for method in service.rest_methods() {
        match mode {
            // Scoped services contribute only collection-style methods (list/create/factory);
            // their instance methods live on the generated scoped client.
            BindingMode::Scoped => {
                if method.is_collection_method() {
                    methods.push(collection_method(service, &method));
                }
            }
            // Resource-less services contribute every method, lowered flat (all params passed,
            // including path params).
            BindingMode::Flat => {
                methods.push(flat_method(service, &method));
            }
        }
    }

    if let Some(accessor) = resource_accessor_method(service) {
        methods.push(accessor);
    }

    methods
}

/// Build a doc-comment token stream for an aggregate method: the proto method documentation,
/// followed by a `# Arguments` section listing each required param that has a field comment.
///
/// Falls back to just the method doc (or nothing) when no params carry documentation, so methods
/// with documented protos still get useful rustdoc on the aggregate surface.
fn doc_with_arguments(method: &MethodHandler<'_>) -> TokenStream {
    let mut doc = method
        .plan
        .metadata
        .documentation
        .as_deref()
        .map(|d| d.trim_end().to_string())
        .unwrap_or_default();

    let arg_lines: Vec<String> = method
        .required_parameters()
        .filter_map(|p| {
            p.documentation()
                .map(|d| format!("* `{}` - {}", p.name(), d.trim()))
        })
        .collect();

    if !arg_lines.is_empty() {
        if !doc.is_empty() {
            doc.push('\n');
        }
        doc.push_str("\n# Arguments\n\n");
        doc.push_str(&arg_lines.join("\n"));
    }

    if doc.is_empty() {
        quote! {}
    } else {
        doc_tokens(Some(&doc))
    }
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
fn collection_method(service: &ServiceHandler<'_>, method: &MethodHandler<'_>) -> TokenStream {
    builder_constructor_method(service, method)
}

/// Emit a method for a resource-less ("flat") service: pass ALL required params (incl. path) to the
/// builder constructor. Covers list/create/get/update/delete/custom uniformly.
fn flat_method(service: &ServiceHandler<'_>, method: &MethodHandler<'_>) -> TokenStream {
    builder_constructor_method(service, method)
}

/// Shared body for aggregate builder-constructor methods: forward all required params to the
/// builder's `::new`, constructing the per-service low-level client on demand.
fn builder_constructor_method(
    service: &ServiceHandler<'_>,
    method: &MethodHandler<'_>,
) -> TokenStream {
    let doc = doc_with_arguments(method);
    let method_name = method.binding_method_name();
    let builder_ty = method.builder_type();
    let params: Vec<&RequestParam> = method.required_parameters().collect();
    let (param_defs, args) = builder_params(method, &params);
    let ctor = low_level_client_ctor(service);

    quote! {
        #doc
        pub fn #method_name(&self, #(#param_defs),*) -> #builder_ty {
            #builder_ty::new(#ctor, #(#args),*)
        }
    }
}

/// Emit the resource accessor for a resource-scoped service:
/// `<singular>(<components>: impl Into<String>…) -> <Singular>Client`.
///
/// Passes the captured name components **directly** to the generated scoped client's `new` — no
/// dot-join / split round-trip. The components come from the shared [`super::AccessorSpec`] (built
/// from `ServicePlan::resource_accessor_params`): the parent chain plus the leaf name. The
/// per-service low-level client is built on demand from the aggregate's `CloudClient` + `base_url`.
fn resource_accessor_method(service: &ServiceHandler<'_>) -> Option<TokenStream> {
    // The accessor returns the generated scoped client, which only exists when resource-client
    // generation is enabled. Without it there is nothing to hand back.
    if !service.config.output.generate_resource_clients {
        return None;
    }
    // The scoped client is skipped for resources with a non-string path parameter (see
    // `supports_scoped_client`); don't emit an accessor to a client that won't exist.
    if !service.supports_scoped_client() {
        return None;
    }
    let spec = service.accessor_spec()?;
    let client_ty = service.scoped_client_type()?;
    let method_name = format_ident!("{}", spec.singular);
    let ctor = low_level_client_ctor(service);

    let param_idents: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
    let param_defs: Vec<_> = param_idents
        .iter()
        .map(|id| quote! { #id: impl Into<String> })
        .collect();
    let args = param_idents.iter().map(|id| quote! { #id });

    let doc = format!(
        " Access the `{}` resource scoped to the given name.",
        spec.singular
    );

    // For nested resources, also offer a `<singular>_from_full_name(full_name)` convenience that
    // delegates to the scoped client's `from_full_name` (which splits the dot-joined name once).
    let from_full_name = if spec.nested {
        let ffn_method = format_ident!("{}_from_full_name", spec.singular);
        let ffn_doc = format!(
            " Access the `{}` resource from its dot-joined full name.",
            spec.singular
        );
        quote! {
            #[doc = #ffn_doc]
            pub fn #ffn_method(&self, full_name: impl Into<String>) -> #client_ty {
                #client_ty::from_full_name(full_name, #ctor)
            }
        }
    } else {
        quote! {}
    };

    Some(quote! {
        #[doc = #doc]
        pub fn #method_name(&self, #(#param_defs),*) -> #client_ty {
            #client_ty::new(#(#args,)* #ctor)
        }

        #from_full_name
    })
}
