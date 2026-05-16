//! Python code generation module
//!
//! Split into two submodules:
//! - `bindings`: PyO3 binding generation (Rust → Python wrapper structs)
//! - `typings`: `.pyi` type stub generation for Python IDE support

mod bindings;
mod typings;

pub(crate) use bindings::{generate, main_module};
pub(crate) use typings::generate_typings;

use crate::analysis::RequestType;
use crate::codegen::{MethodHandler, ServiceHandler};
use crate::parsing::types::{BaseType, UnifiedType};
use crate::utils::extract_simple_type_name;

static DOCS_TARGET_WIDTH: usize = 100;

/// Extract parameter names from a `ResourceDescriptor` pattern string.
///
/// For example, `"catalogs/{catalog}/schemas/{schema}"` yields
/// `["catalog_name", "schema_name"]`.
///
/// A single-parameter resource returns `["name"]` for brevity.
pub(super) fn resource_pattern_params(pattern: &str) -> Vec<String> {
    let params: Vec<String> = pattern
        .split('/')
        .filter(|seg| seg.starts_with('{') && seg.ends_with('}'))
        .map(|seg| {
            let inner = &seg[1..seg.len() - 1];
            format!("{}_name", inner)
        })
        .collect();

    if params.len() == 1 {
        vec!["name".to_string()]
    } else {
        params
    }
}

/// Derive the parameter names for a resource accessor method on the main client.
///
/// **Annotation-driven path** (preferred): when the service has `hierarchy` entries
/// from `resource_reference { child_type }` annotations, the parent field names are
/// taken directly from those entries (in List method query param order).
///
/// **Heuristic fallback** (when no annotations present): uses two signals:
/// 1. `name_field` non-empty on the `ResourceDescriptor` → resource has a composite name.
/// 2. Get method path param `"name"` + required string params on List method.
pub(super) fn derive_resource_accessor_params(service: &ServiceHandler<'_>) -> Vec<String> {
    let resource = match service.resource() {
        Some(r) => r,
        None => return vec!["name".to_string()],
    };

    // --- Annotation-driven path ---
    // Find hierarchy entries for this resource's type string
    let resource_type = &resource.descriptor.r#type;
    if !service.plan.hierarchy.is_empty() && !resource_type.is_empty() {
        let annotation_parents: Vec<String> = service
            .plan
            .hierarchy
            .iter()
            .filter(|h| &h.child_resource_type == resource_type)
            .map(|h| h.parent_field_name.clone())
            .collect();

        if !annotation_parents.is_empty() {
            let mut params = annotation_parents;
            params.push(format!("{}_name", resource.descriptor.singular));
            return params;
        }
    }

    // --- Heuristic fallback ---
    let has_explicit_name_field = !resource.descriptor.name_field.is_empty();

    let get_path_param_name = service
        .methods()
        .find(|m| matches!(m.plan.request_type, RequestType::Get))
        .and_then(|m| m.plan.path_parameters().next().map(|p| p.name.clone()));

    let parent_params: Vec<String> = service
        .methods()
        .find(|m| matches!(m.plan.request_type, RequestType::List))
        .map(|m| {
            m.required_parameters()
                .filter(|p| !p.is_path_param())
                .filter(|p| matches!(p.field_type().base_type, BaseType::String))
                .map(|p| p.name().to_string())
                .collect()
        })
        .unwrap_or_default();

    let should_decompose = has_explicit_name_field
        || (get_path_param_name.as_deref() == Some("name") && !parent_params.is_empty());

    if should_decompose {
        let mut params = parent_params;
        params.push(format!("{}_name", resource.descriptor.singular));
        params
    } else {
        vec!["name".to_string()]
    }
}

fn is_list_method(method: &MethodHandler<'_>) -> bool {
    matches!(method.plan.request_type, RequestType::List)
}

fn python_type_annotation(unified_type: &UnifiedType) -> String {
    crate::parsing::types::unified_to_python_type(unified_type)
}

fn python_type_annotation_from_ident(ident: &syn::Ident) -> String {
    let type_str = ident.to_string();
    match type_str.as_str() {
        "String" => "str".to_string(),
        "i32" | "i64" => "int".to_string(),
        "f32" | "f64" => "float".to_string(),
        "bool" => "bool".to_string(),
        "Vec < u8 >" => "bytes".to_string(),
        "()" => "None".to_string(),
        _ => {
            if let Some(simple_name) = type_str.split("::").last() {
                simple_name.trim().to_string()
            } else {
                type_str
            }
        }
    }
}

fn sanitize_python_field_name(field_name: &str) -> String {
    match field_name {
        "not" | "and" | "or" | "is" | "in" | "def" | "class" | "if" | "else" | "for" | "while"
        | "try" | "except" | "finally" | "with" | "as" | "import" | "from" | "pass" | "break"
        | "continue" | "return" | "yield" | "raise" | "assert" | "del" | "global" | "nonlocal"
        | "lambda" | "None" | "True" | "False" | "async" | "await" => {
            format!("{}_", field_name)
        }
        _ => field_name.to_string(),
    }
}

fn clean_and_format_description(text: &str) -> String {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return String::new();
    }

    // Split on blank lines to preserve paragraph structure
    let paragraphs: Vec<String> = cleaned
        .split("\n\n")
        .map(|para| {
            para.lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|para| !para.is_empty())
        .map(|para| textwrap::fill(&para, DOCS_TARGET_WIDTH))
        .collect();

    paragraphs.join("\n\n")
}
