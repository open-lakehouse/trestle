//! Python code generation module
//!
//! Split into submodules:
//! - `bindings`: PyO3 binding generation (Rust → Python wrapper structs)
//! - `runtime`: the shared Tokio runtime, `block_on`, and list pagers (`codegen/runtime.rs`)
//! - `typings`: `.pyi` type stub generation for Python IDE support

mod bindings;
mod model_conversions;
mod runtime;
mod typings;

pub(crate) use bindings::{generate, main_module};
pub(crate) use model_conversions::generate_pyo3_impls;
pub(crate) use runtime::generate_runtime;
pub(crate) use typings::generate_typings;

use proc_macro2::Ident;
use quote::format_ident;

use crate::analysis::RequestType;
use crate::codegen::{MethodHandler, PythonApi};
use crate::parsing::types::UnifiedType;
use crate::utils::extract_simple_type_name;

static DOCS_TARGET_WIDTH: usize = 100;

/// Whether a generated client class blocks the calling thread or returns awaitables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PyFlavor {
    /// Methods block (with the GIL released) and return the result.
    Sync,
    /// Methods return an `asyncio` awaitable (via `pyo3_async_runtimes::tokio::future_into_py`).
    Async,
}

impl PyFlavor {
    /// The flavors enabled by `api`, sync first.
    pub(super) fn enabled(api: PythonApi) -> impl Iterator<Item = Self> {
        [
            api.has_sync().then_some(Self::Sync),
            api.has_async().then_some(Self::Async),
        ]
        .into_iter()
        .flatten()
    }

    /// The Python-visible class-name prefix (`""` / `"Async"`).
    pub(super) fn prefix(self) -> &'static str {
        match self {
            Self::Sync => "",
            Self::Async => "Async",
        }
    }

    /// The Rust wrapper struct ident for a Rust client name (`CatalogClient` → `PyCatalogClient` /
    /// `PyAsyncCatalogClient`).
    pub(super) fn class_ident(self, rust_client: &str) -> Ident {
        format_ident!("Py{}{}", self.prefix(), rust_client)
    }

    /// The Python-visible class name (`CatalogClient` / `AsyncCatalogClient`).
    pub(super) fn class_name(self, rust_client: &str) -> String {
        format!("{}{}", self.prefix(), rust_client)
    }
}

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
            format!("{inner}_name")
        })
        .collect();

    if params.len() == 1 {
        vec!["name".to_string()]
    } else {
        params
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
            format!("{field_name}_")
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
