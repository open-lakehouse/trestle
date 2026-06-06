//! NAPI capability predicates shared by the Node emitters.
//!
//! Both the NAPI binding emitter (`bindings.rs`, `TokenStream`-based) and the TypeScript client
//! emitter (`typescript.rs`, string-template-based) need to agree on what crosses the NAPI boundary
//! natively vs what must be passed as serialized protobuf bytes. These are pure functions over the
//! IR (`RequestParam` / `BaseType`) with no token output, so both emitters share one source of truth.

use crate::analysis::RequestParam;
use crate::parsing::types::BaseType;

/// Check if a parameter type is supported across the NAPI boundary.
///
/// NAPI-RS supports: primitives, String, bool, Buffer, HashMap<String, String>,
/// Vec<T> of supported types, Option<T> of supported types.
/// Enums are supported as i32 values. Complex messages/oneofs are not.
///
/// NOTE: Enums should be annotated with `#[napi]` via `buf.gen.yaml` `enum_attribute`
/// and the `napi` feature gate on the common crate. When that feature is active,
/// napi-rs v3 handles the enum type directly; when it is not, the `i32` fallback is used.
pub(crate) fn is_napi_supported(param: &RequestParam) -> bool {
    is_napi_supported_type(&param.field_type().base_type)
}

pub(crate) fn is_napi_supported_type(base_type: &BaseType) -> bool {
    match base_type {
        BaseType::String
        | BaseType::Int32
        | BaseType::Int64
        | BaseType::Bool
        | BaseType::Float32
        | BaseType::Float64
        | BaseType::Bytes
        | BaseType::Unit
        | BaseType::Enum(_) => true,
        BaseType::Map(k, v) => {
            is_napi_supported_type(&k.base_type) && is_napi_supported_type(&v.base_type)
        }
        BaseType::Message(_) | BaseType::OneOf(_) => false,
    }
}

/// Whether a param is a required, singular protobuf message — passed across the NAPI boundary as
/// serialized bytes (a `Buffer`/`toBinary(<Type>Schema, value)`) and accepted as a typed object in
/// the method signature.
pub(crate) fn is_required_message_body(param: &RequestParam) -> bool {
    !param.is_optional()
        && !param.field_type().is_repeated
        && matches!(
            param.field_type().base_type,
            BaseType::Message(_) | BaseType::OneOf(_)
        )
}

/// The simple message type name of a param's type (e.g. `TagPolicy`), for `toBinary`/type rendering.
pub(crate) fn message_type_name(param: &RequestParam) -> Option<String> {
    match &param.field_type().base_type {
        BaseType::Message(n) | BaseType::OneOf(n) => {
            Some(crate::utils::extract_simple_type_name(n))
        }
        _ => None,
    }
}
