//! The docstrings of the generated Python methods: the proto comment of the RPC with `Args:`
//! and `Returns:` sections, which the bindings carry as `#[doc]` attributes for the stubs.

use textwrap::{Options, fill, refill};

use super::{DOCS_TARGET_WIDTH, clean_and_format_description, is_list_method};
use crate::analysis::RequestType;
use crate::codegen::MethodHandler;

pub(super) fn format_method_docstring_with_params(method: &MethodHandler<'_>) -> String {
    let mut docstring_parts = Vec::new();

    if let Some(doc) = method.plan.metadata.documentation.as_ref() {
        let cleaned_doc = clean_and_format_description(doc);
        if !cleaned_doc.is_empty() {
            docstring_parts.push(cleaned_doc);
        }
    }

    let param_docs = collect_parameter_documentation(method);
    if !param_docs.is_empty() {
        if !docstring_parts.is_empty() {
            docstring_parts.push("".to_string());
        }
        docstring_parts.push("Args:".to_string());
        for (param_name, param_doc) in param_docs {
            let cleaned_param_doc = clean_and_format_description(&param_doc);
            let wrapped_param = format_parameter_description(&param_name, &cleaned_param_doc);
            docstring_parts.push(wrapped_param);
        }
    }

    let return_doc = get_return_type_documentation(method);
    if !return_doc.is_empty() {
        if !docstring_parts.is_empty() {
            docstring_parts.push("".to_string());
        }
        docstring_parts.push("Returns:".to_string());
        let cleaned_return_doc = clean_and_format_description(&return_doc);
        let wrapped_return = format_return_description(&cleaned_return_doc);
        docstring_parts.push(wrapped_return);
    }

    if docstring_parts.is_empty() {
        return String::new();
    }

    let mut result: Vec<String> = Vec::new();
    for (i, part) in docstring_parts.iter().enumerate() {
        if i > 0
            && !part.is_empty()
            && !result.is_empty()
            && (part.starts_with("Args:") || part.starts_with("Returns:"))
            && !result.last().unwrap().is_empty()
        {
            result.push(String::new());
        }
        result.push(part.clone());
    }
    result.join("\n")
}

fn format_parameter_description(param_name: &str, description: &str) -> String {
    let first_line_prefix = format!("    {param_name}: ");
    let continuation_prefix = " ".repeat(first_line_prefix.len());

    let first_line_width = DOCS_TARGET_WIDTH - first_line_prefix.len();
    let continuation_width = DOCS_TARGET_WIDTH - continuation_prefix.len();

    if first_line_prefix.len() >= 90 {
        let optimally_filled = refill(description, continuation_width);
        let options = Options::new(continuation_width)
            .initial_indent(&continuation_prefix)
            .subsequent_indent(&continuation_prefix);
        return format!(
            "{}\n{}",
            first_line_prefix,
            fill(&optimally_filled, &options)
        );
    }

    let options = Options::new(first_line_width)
        .initial_indent("")
        .subsequent_indent(&continuation_prefix);

    let initial_wrapped = fill(description, &options);

    if initial_wrapped.contains('\n') {
        let refilled_desc = refill(description, continuation_width);

        let lines: Vec<&str> = refilled_desc.lines().collect();
        if lines.is_empty() {
            return first_line_prefix;
        }

        let mut result = format!("{}{}", first_line_prefix, lines[0]);
        for line in &lines[1..] {
            result.push_str(&format!("\n{continuation_prefix}{line}"));
        }
        result
    } else {
        format!("{first_line_prefix}{initial_wrapped}")
    }
}

fn format_return_description(description: &str) -> String {
    let prefix = "    ";

    let available_width = DOCS_TARGET_WIDTH - prefix.len();
    let refilled_desc = refill(description, available_width);

    let options = Options::new(available_width)
        .initial_indent(prefix)
        .subsequent_indent(prefix);

    fill(&refilled_desc, &options)
}

fn collect_parameter_documentation(method: &MethodHandler<'_>) -> Vec<(String, String)> {
    let mut param_docs = Vec::new();

    for param in method.required_parameters() {
        if !param.is_path_param()
            && let Some(doc) = param.documentation()
        {
            let cleaned_doc = clean_and_format_description(doc);
            if !cleaned_doc.is_empty() {
                param_docs.push((param.name().to_string(), cleaned_doc));
            }
        }
    }

    for param in method.optional_parameters() {
        if !(is_list_method(method) && param.name() == "page_token")
            && let Some(doc) = param.documentation()
        {
            let cleaned_doc = clean_and_format_description(doc);
            if !cleaned_doc.is_empty() {
                param_docs.push((param.name().to_string(), cleaned_doc));
            }
        }
    }

    param_docs
}

/// The `Returns:` text of a method's docstring; empty for a method that returns nothing.
fn get_return_type_documentation(method: &MethodHandler<'_>) -> String {
    if method.output_type().is_none() {
        return String::new();
    }
    match &method.plan.request_type {
        RequestType::Delete => String::new(),
        // The list method returns the items themselves; the items field says what they are.
        RequestType::List => method
            .list_output_field()
            .and_then(|field| field.documentation.as_deref())
            .map(clean_and_format_description)
            .filter(|doc| !doc.is_empty())
            .unwrap_or_else(|| "The items.".to_string()),
        _ => {
            if let Some(output_message) = method.output_message() {
                if let Some(doc) = output_message.info.documentation.as_ref() {
                    let cleaned_doc = clean_and_format_description(doc);
                    if cleaned_doc.is_empty() {
                        "The requested resource".to_string()
                    } else {
                        cleaned_doc
                    }
                } else {
                    "The requested resource".to_string()
                }
            } else {
                "The requested resource".to_string()
            }
        }
    }
}
