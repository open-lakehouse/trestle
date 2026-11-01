//! The shared dispatch skeleton: which methods land where, and how each shape is assembled.
//!
//! These functions were previously copy-pasted (byte-for-byte) into `python/bindings.rs` and
//! `node/bindings.rs`. They are now the single source of truth; each backend calls them to obtain
//! the scoped-client and root-client method token vecs, then wraps those in its own module scaffold.

use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::{BindingBackend, ShapeParts};
use crate::analysis::{EmitShape, RequestParam};
use crate::codegen::{BindingMode, MethodHandler, ServiceHandler};

/// Build the [`ShapeParts`] for `method` in `mode`, pulling the language-specific param defs and
/// client call from the backend. Borrows `method` for the lifetime of the returned struct.
fn shape_parts<'a>(
    backend: &dyn BindingBackend,
    method: &'a MethodHandler<'a>,
    mode: BindingMode,
) -> ShapeParts<'a> {
    ShapeParts {
        param_defs: backend.param_defs(method, mode),
        client_call: backend.client_call(method, mode),
        method,
        mode,
    }
}

/// Dispatch a single method to the backend's body builder for its [`EmitShape`]. The single dispatch
/// point that replaced the per-(scoped/collection/flat) `match request_type` arms — those only ever
/// varied in *which* methods they emitted (decided by [`scoped_instance_method`]/[`root_method`]),
/// not in the per-shape emit logic.
fn emit_for_shape(
    backend: &dyn BindingBackend,
    method: &MethodHandler<'_>,
    mode: BindingMode,
) -> TokenStream {
    let parts = shape_parts(backend, method, mode);
    match method.emit_shape() {
        EmitShape::List => backend.emit_list(&parts),
        EmitShape::Create => backend.emit_create(&parts),
        EmitShape::GetUpdate => backend.emit_get_update(&parts),
        EmitShape::Delete => backend.emit_delete(&parts),
    }
}

/// Emit an instance method for a resource-scoped client (`get`/`update`/`delete` and
/// resource-targeted custom POST/PATCH RPCs), or `None` if the method does not belong on the scoped
/// client. Always [`BindingMode::Scoped`].
fn scoped_instance_method(
    backend: &dyn BindingBackend,
    method: MethodHandler<'_>,
) -> Option<TokenStream> {
    method
        .is_scoped_instance_method()
        .then(|| emit_for_shape(backend, &method, BindingMode::Scoped))
}

/// Emit a method on the root (aggregate) client, or `None` if it does not belong there.
///
/// - [`BindingMode::Scoped`] services contribute only their collection-style methods (list / create);
///   their instance methods live on the scoped client.
/// - [`BindingMode::Flat`] services (resource-less) contribute **every** method, lowered flat so each
///   passes all params (including path params) directly to the root client.
fn root_method(
    backend: &dyn BindingBackend,
    method: MethodHandler<'_>,
    mode: BindingMode,
) -> Option<TokenStream> {
    match mode {
        BindingMode::Scoped => {
            let collection_shaped =
                matches!(method.emit_shape(), EmitShape::List | EmitShape::Create);
            (method.is_collection_method() && collection_shaped)
                .then(|| emit_for_shape(backend, &method, mode))
        }
        BindingMode::Flat => Some(emit_for_shape(backend, &method, mode)),
    }
}

/// All scoped-client instance methods for one service, in declaration order.
pub(crate) fn scoped_client_methods(
    backend: &dyn BindingBackend,
    service: &ServiceHandler<'_>,
) -> Vec<TokenStream> {
    service
        .methods()
        .filter_map(|m| scoped_instance_method(backend, m))
        .collect()
}

/// All root-client methods across `services` (collection methods from scoped services + every method
/// of flat services), in the order `services` is given.
pub(crate) fn root_client_methods(
    backend: &dyn BindingBackend,
    services: &[&ServiceHandler<'_>],
) -> Vec<TokenStream> {
    services
        .iter()
        .flat_map(|s| {
            let mode = s.binding_mode();
            s.methods()
                .filter_map(move |m| root_method(backend, m, mode))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The Rust client method a binding of `method` calls in `mode`, and whether the binding drops the
/// path params (a scoped client supplies them).
pub(crate) fn client_method(method: &MethodHandler<'_>, mode: BindingMode) -> (Ident, bool) {
    match mode {
        BindingMode::Scoped => (method.plan.resource_client_method(), true),
        BindingMode::Flat => (method.plan.base_method_ident(), false),
    }
}

/// Whether `param` is a list method's required `page_token`. Bindings don't expose it (the stream
/// drives the pagination and overwrites it), so they seed the builder with an empty token.
pub(crate) fn is_seeded_page_token(method: &MethodHandler<'_>, param: &RequestParam) -> bool {
    method.emit_shape() == EmitShape::List && param.name() == "page_token"
}

/// `services` sorted by service name.
pub(crate) fn sorted_services<'s, 'a>(
    services: &'s [ServiceHandler<'a>],
) -> Vec<&'s ServiceHandler<'a>> {
    let mut sorted: Vec<_> = services.iter().collect();
    sorted.sort_by(|a, b| a.plan.service_name.cmp(&b.plan.service_name));
    sorted
}

/// `use crate::codegen::<base>::<prefix><Client>;` for each service with a scoped binding (only
/// those expose a per-service scoped client to import).
pub(crate) fn scoped_client_imports<'s>(
    services: &'s [&ServiceHandler<'_>],
    prefix: &'s str,
) -> impl Iterator<Item = TokenStream> + 's {
    services
        .iter()
        .filter(|s| s.emits_scoped_binding())
        .map(move |s| {
            let mod_name = format_ident!("{}", s.plan.base_path);
            let client_name = format_ident!("{prefix}{}", s.client_type());
            quote! { use crate::codegen::#mod_name::#client_name; }
        })
}

/// The root-client accessor `<singular>(<params>: String…) -> <prefix><Client>` wrapping the Rust
/// client's resource accessor, with `attr` on it.
///
/// A nested resource has a composite `full_name`; the underlying Rust client accessor takes a
/// single `full_name` string, so the decomposed params are joined with `.` for
/// `<singular>_from_full_name`.
pub(crate) fn resource_accessor(
    service: &ServiceHandler<'_>,
    attr: TokenStream,
    client_prefix: &str,
) -> Option<TokenStream> {
    let spec = service.accessor_spec()?;
    let method_name = format_ident!("{}", spec.singular);
    let client_name = format_ident!("{client_prefix}{}", service.client_type());

    let param_idents: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
    let param_list = param_idents.iter().map(|id| quote! { #id: String });

    let client = if spec.nested {
        let from_full_name_method = format_ident!("{}_from_full_name", spec.singular);
        let format_str = spec.join_format();
        quote! {
            let full_name = format!(#format_str, #(#param_idents),*);
            #client_name {
                client: self.client.#from_full_name_method(full_name),
            }
        }
    } else {
        quote! {
            #client_name {
                client: self.client.#method_name(#(#param_idents),*),
            }
        }
    };

    Some(quote! {
        #attr
        pub fn #method_name(&self, #(#param_list),*) -> #client_name {
            #client
        }
    })
}

/// The resource-accessor methods for the root client, one per service that exposes a resource.
pub(crate) fn resource_accessor_methods(
    backend: &dyn BindingBackend,
    services: &[&ServiceHandler<'_>],
) -> Vec<TokenStream> {
    services
        .iter()
        // Skip resources that can't be modeled by a string-component scoped client (e.g. a
        // model version keyed by a composite `{full_name}` + integer `{version}`); no scoped
        // client is generated for them, so an accessor would dangle. See
        // `ServiceHandler::supports_scoped_client`.
        .filter(|s| s.supports_scoped_client())
        .filter_map(|s| backend.emit_resource_accessor(s))
        .collect()
}
