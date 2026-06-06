//! Shared `with_*` builder-call generation for the binding emitters.
//!
//! Both backends walk a method's optional query params and optional body fields and push a
//! `request = request.with_<field>(<value>)` setter for each. The *iteration + filter skeleton* is
//! identical; only the per-field rendering (NAPI's capability filter and enum/optional wrapping)
//! differs. The skeleton lives here; the backend supplies the two hooks via [`SetterRender`].

use proc_macro2::{Ident, TokenStream};
use quote::format_ident;

use crate::analysis::QueryParam;
use crate::codegen::MethodHandler;
use crate::parsing::types::UnifiedType;
use crate::utils::strings;

/// A field eligible for a `with_*` builder setter, normalized across query params and body fields.
pub(crate) struct OptionalSetter<'a> {
    /// The proto field name (used to build the `with_<name>` method and the local binding ident).
    pub(crate) name: &'a str,
    pub(crate) field_type: &'a UnifiedType,
}

/// Backend hooks for rendering a single optional setter and deciding which fields are eligible.
pub(crate) trait SetterRender {
    /// Whether this language can pass `setter` across its binding boundary at all (NAPI filters
    /// unsupported types; PyO3 accepts everything).
    fn supports(&self, setter: &OptionalSetter<'_>) -> bool;

    /// Render the single `request = request.with_<name>(<value>);` statement, applying any
    /// language-specific value wrapping (e.g. NAPI enum `try_into`, or an `if let Some` guard for
    /// maps/repeated fields).
    fn render_setter(
        &self,
        setter: &OptionalSetter<'_>,
        param_ident: &Ident,
        with_method: &Ident,
    ) -> TokenStream;
}

/// Generate the `with_*` builder calls for a method's optional query params and body fields.
///
/// `is_list` drops the `page_token` query param for list methods (pagination is driven by the
/// streaming/collecting wrapper, not a caller-supplied token) — matching the prior behavior in both
/// emitters.
pub(crate) fn generate_builder_pattern<R: SetterRender>(
    method: &MethodHandler<'_>,
    is_list: bool,
    render: &R,
) -> Vec<TokenStream> {
    let mut calls = Vec::new();

    for query_param in method.plan.query_parameters() {
        if is_optional_query(query_param, is_list) {
            let setter = OptionalSetter {
                name: &query_param.name,
                field_type: &query_param.field_type,
            };
            if render.supports(&setter) {
                calls.push(render_one(&setter, render));
            }
        }
    }

    for body_field in method.plan.body_fields() {
        if body_field.is_optional() {
            let setter = OptionalSetter {
                name: &body_field.name,
                field_type: &body_field.field_type,
            };
            if render.supports(&setter) {
                calls.push(render_one(&setter, render));
            }
        }
    }

    calls
}

fn is_optional_query(param: &QueryParam, is_list: bool) -> bool {
    param.is_optional() && !(is_list && param.name == "page_token")
}

fn render_one<R: SetterRender>(setter: &OptionalSetter<'_>, render: &R) -> TokenStream {
    let param_ident = format_ident!("{}", strings::operation_to_method_name(setter.name));
    let with_method = format_ident!("with_{}", setter.name);
    render.render_setter(setter, &param_ident, &with_method)
}
