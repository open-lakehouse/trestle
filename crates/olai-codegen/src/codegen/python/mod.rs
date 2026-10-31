//! Python code generation module
//!
//! - `bindings`: the PyO3 client classes
//! - `model_conversions`: the `#[pyclass]` wrappers of the models
//! - `errors`, `future`, `runtime`: the helper files of the bindings crate
//! - `package`: the pure-Python files (`_enums.py`, the public modules)
//! - `docs`: the docstrings of the methods

mod bindings;
mod docs;
mod errors;
mod future;
mod model_conversions;
mod package;
mod runtime;

pub(crate) use bindings::{generate, main_module};
pub(crate) use model_conversions::generate_pyo3_impls;

/// The helper files of the Python bindings (error module, `PyFuture`, runtime), by file name.
pub(crate) fn helper_files(config: &crate::codegen::CodeGenConfig) -> Vec<(String, String)> {
    let bindings = config
        .bindings
        .as_ref()
        .expect("validated: Python output has a bindings config");
    let python = config.python_options();
    let mut files = vec![
        ("error.rs".to_string(), errors::generate(bindings, python)),
        (
            "runtime.rs".to_string(),
            runtime::generate(python.async_methods),
        ),
    ];
    if python.async_methods {
        files.push(("py_future.rs".to_string(), future::generate()));
    }
    files
}

/// The pure-Python package files (`_enums.py`, the public modules), by file name.
pub(crate) fn package_files(
    config: &crate::codegen::CodeGenConfig,
    plan: &crate::analysis::GenerationPlan,
    metadata: &crate::parsing::CodeGenMetadata,
) -> Vec<(String, String)> {
    let python = config.python_options();
    let services: Vec<_> = plan.services.iter().collect();
    let classes = model_conversions::ModelClasses::new(&services, metadata);
    let mut files = Vec::new();
    if !classes.enums.is_empty() {
        files.push((
            "_enums.py".to_string(),
            package::enums_py(metadata, &classes.enums),
        ));
    }
    if python.public_modules {
        let handlers: Vec<_> = plan
            .services
            .iter()
            .map(|plan| crate::codegen::ServiceHandler {
                plan,
                metadata,
                config,
            })
            .collect();
        let clients = handlers
            .iter()
            .filter(|s| s.emits_scoped_binding())
            .map(|s| {
                (
                    package_segment(&s.plan.package),
                    s.client_type().to_string(),
                )
            });
        files.extend(package::public_modules(
            &python.module,
            &python.package.name,
            classes.public_names(),
            clients,
        ));
    }
    files
}

/// The `module = "…",` argument of a `#[pyclass]`: [`PythonOptions::module`], or its submodule
/// `segment`. The classes are defined outside the `#[pymodule]`, which sets it only for its own.
fn module_arg(options: &PythonOptions, segment: Option<&str>) -> TokenStream {
    let root = &options.module;
    let module = match segment {
        Some(segment) => format!("{root}.{segment}"),
        None => root.clone(),
    };
    quote! { module = #module, }
}

/// The Python submodule segment of a proto package (`a.catalogs.v1` → `catalogs`).
fn package_segment(package: &str) -> &str {
    crate::codegen::package_models_segment(package).unwrap_or(package)
}

use proc_macro2::TokenStream;
use quote::quote;

use crate::analysis::RequestType;
use crate::codegen::{MethodHandler, PythonOptions};

static DOCS_TARGET_WIDTH: usize = 100;

fn is_list_method(method: &MethodHandler<'_>) -> bool {
    matches!(method.plan.request_type, RequestType::List)
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
