//! Shared driver for the language-binding emitters (PyO3, NAPI-RS).
//!
//! The Python and Node binding emitters are structurally isomorphic: they walk the same
//! [`ServiceHandler`]/[`MethodHandler`] IR, dispatch identically on [`EmitShape`] and
//! [`BindingMode`], and make byte-for-byte identical decisions about *which* methods land on the
//! scoped client vs the root client. They diverge only on language-specific details: the wrapper
//! class prefix (`Py`/`Napi`), per-method attributes (`#[pyo3(signature)]`/`#[napi]`), the method
//! body (sync `block_on`/`allow_threads` vs async `.await`, typed return vs serialized `Buffer`),
//! parameter marshaling, and the module preamble.
//!
//! This module owns the shared *control flow* — the dispatch and the "which methods" logic — behind
//! the [`BindingBackend`] trait. Each backend (`python::PyBackend`, `node::NapiBackend`) supplies the
//! irreducibly language-specific pieces. The method *bodies* stay per-backend (see the `emit_*`
//! trait methods); only the skeleton that selects and assembles them is shared.

pub(crate) mod builder;
pub(crate) mod driver;

use proc_macro2::TokenStream;

use crate::codegen::{BindingMode, MethodHandler, ServiceHandler};

/// The shared, pre-computed pieces of a binding method, assembled by the driver and handed to a
/// backend's `emit_*` body builder. Lets each backend write only the body tail + signature glue
/// without re-deriving the param defs / client call / builder setters.
pub(crate) struct ShapeParts<'a> {
    /// The method this binding wraps, plus its binding mode.
    pub(crate) method: &'a MethodHandler<'a>,
    pub(crate) mode: BindingMode,
    /// Typed `name: Type` parameter definitions for the signature (already capability-filtered and
    /// marshaled by the backend's [`BindingBackend::param_defs`]).
    pub(crate) param_defs: Vec<TokenStream>,
    /// The `self.client.<method>(<args>)` call into the underlying Rust client, with per-arg
    /// marshaling applied by [`BindingBackend::client_call`].
    pub(crate) client_call: TokenStream,
}

/// Language-specific hooks for a binding emitter. The driver supplies the shared dispatch and
/// scaffolding; implementors supply the bits that genuinely differ between PyO3 and NAPI-RS.
pub(crate) trait BindingBackend {
    // ---- scaffolding / preamble ----

    /// The full per-service module: `use` lines, the `#[pyclass]`/`#[napi]` wrapper struct + impl
    /// holding the scoped-client instance methods. Returns the formatted source string.
    fn generate_service(&self, service: &ServiceHandler<'_>) -> crate::error::Result<String>;

    /// The aggregate (root) module: per-service module declarations plus the root client struct
    /// with its constructor, collection methods, and resource accessors.
    fn main_module(&self, services: &[ServiceHandler<'_>]) -> crate::error::Result<String>;

    // ---- per-method assembly (consumed by the driver to build `ShapeParts`) ----

    /// The signature's typed parameter definitions (`name: Type`), capability-filtered and marshaled
    /// for this language.
    fn param_defs(&self, method: &MethodHandler<'_>, mode: BindingMode) -> Vec<TokenStream>;

    /// The `self.client.<method>(<args>)` call into the underlying Rust client, with per-arg
    /// marshaling (e.g. NAPI message-decode / enum `try_into`).
    fn client_call(&self, method: &MethodHandler<'_>, mode: BindingMode) -> TokenStream;

    // ---- method bodies, one per EmitShape (irreducibly language-specific) ----

    /// Emit the list method(s). May produce more than one method (Node emits a batch + a streaming
    /// variant; Python emits one). `EmitShape::List` always has a list output field.
    fn emit_list(&self, parts: &ShapeParts<'_>) -> TokenStream;
    fn emit_create(&self, parts: &ShapeParts<'_>) -> TokenStream;
    fn emit_get_update(&self, parts: &ShapeParts<'_>) -> TokenStream;
    fn emit_delete(&self, parts: &ShapeParts<'_>) -> TokenStream;

    // ---- resource accessor ----

    /// The resource-accessor method on the root client (`fn catalog(&self, name) -> PyCatalogClient`).
    /// The driver resolves the [`AccessorSpec`](crate::codegen::AccessorSpec); the backend renders it.
    fn emit_resource_accessor(&self, service: &ServiceHandler<'_>) -> Option<TokenStream>;
}
