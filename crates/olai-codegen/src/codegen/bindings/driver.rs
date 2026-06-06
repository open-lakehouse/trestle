//! The shared dispatch skeleton: which methods land where, and how each shape is assembled.
//!
//! These functions were previously copy-pasted (byte-for-byte) into `python/bindings.rs` and
//! `node/bindings.rs`. They are now the single source of truth; each backend calls them to obtain
//! the scoped-client and root-client method token vecs, then wraps those in its own module scaffold.

use proc_macro2::TokenStream;

use super::{BindingBackend, ShapeParts};
use crate::analysis::EmitShape;
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

/// The resource-accessor methods for the root client, one per service that exposes a resource.
pub(crate) fn resource_accessor_methods(
    backend: &dyn BindingBackend,
    services: &[&ServiceHandler<'_>],
) -> Vec<TokenStream> {
    services
        .iter()
        .filter_map(|s| backend.emit_resource_accessor(s))
        .collect()
}
