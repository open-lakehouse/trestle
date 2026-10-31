//! The pure-Python files of the generated package: `_enums.py`, and the public modules that
//! re-export the submodules of the extension module.
//!
//! Enums are Python source because a PyO3 class cannot subclass `str`: an `enum.StrEnum` member
//! equals its name, works as a dict key and goes into JSON as is. The file needs no stubs.

use std::collections::{BTreeMap, BTreeSet};

use crate::parsing::{CodeGenMetadata, EnumInfo};

use super::model_conversions::{hidden_value, python_class_name};

/// The `_enums.py` file: one `enum.StrEnum` class per enum in `enum_names`.
pub(crate) fn enums_py(metadata: &CodeGenMetadata, enum_names: &[&String]) -> String {
    let mut out = String::from(
        "\"\"\"The enums of the API.\n\n\
         Each member is a `str` equal to its name, so a member and its name compare equal, and a\n\
         parameter takes either.\n\"\"\"\n\n\
         import enum\n",
    );
    for name in enum_names {
        let info = &metadata.enums[*name];
        out.push_str("\n\n");
        out.push_str(&enum_class(name, info));
    }
    out
}

fn enum_class(fq_name: &str, info: &EnumInfo) -> String {
    let class = python_class_name(fq_name);
    let mut out = format!("class {class}(enum.StrEnum):\n");
    if let Some(doc) = docstring(info.documentation.as_deref(), "    ") {
        out.push_str(&doc);
        out.push('\n');
    }
    for value in &info.values {
        if hidden_value(value) {
            continue;
        }
        // Not `enum.auto()`, which lowercases the name in a `StrEnum`.
        out.push_str(&format!("    {0} = \"{0}\"\n", value.name));
        if let Some(doc) = docstring(value.documentation.as_deref(), "    ") {
            out.push_str(&doc);
        }
    }
    out
}

/// The public modules, `<segment>.py` by file name: one per submodule `<module>.<segment>`,
/// re-exporting its classes and clients, and its enums from `<package>._enums`.
pub(crate) fn public_modules<'a>(
    module: &str,
    package: &str,
    names: impl Iterator<Item = (&'a str, String, bool)>,
    clients: impl Iterator<Item = (&'a str, String)>,
) -> Vec<(String, String)> {
    let mut segments: BTreeMap<&str, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for (segment, name, is_enum) in names {
        let (classes, enums) = segments.entry(segment).or_default();
        if is_enum {
            enums.insert(name);
        } else {
            classes.insert(name);
        }
    }
    for (segment, name) in clients {
        segments.entry(segment).or_default().0.insert(name);
    }
    segments
        .into_iter()
        .map(|(segment, (classes, enums))| {
            let mut imports = vec![
                (format!("{module}.{segment}"), &classes),
                (format!("{package}._enums"), &enums),
            ];
            imports.retain(|(_, names)| !names.is_empty());
            imports.sort();
            let mut out = format!("\"\"\"The classes of the `{segment}` API.\"\"\"\n\n");
            for (from, names) in &imports {
                out.push_str(&format!("from {from} import (\n"));
                for name in isort_order(names) {
                    out.push_str(&format!("    {name},\n"));
                }
                out.push_str(")\n");
            }
            let all: BTreeSet<&String> = classes.iter().chain(&enums).collect();
            out.push_str("\n__all__ = [\n");
            for name in all {
                out.push_str(&format!("    \"{name}\",\n"));
            }
            out.push_str("]\n");
            (format!("{segment}.py"), out)
        })
        .collect()
}

/// `names` in isort order: constants (`ALL_CAPS`) first, then the rest case-insensitively.
fn isort_order(names: &BTreeSet<String>) -> Vec<&String> {
    let mut names: Vec<&String> = names.iter().collect();
    names.sort_by_key(|name| {
        let constant = name.len() > 1 && !name.chars().any(|c| c.is_lowercase());
        (!constant, name.to_lowercase())
    });
    names
}

/// A Python docstring at `indent`, or `None` when `doc` is empty.
fn docstring(doc: Option<&str>, indent: &str) -> Option<String> {
    let doc = doc?.trim();
    if doc.is_empty() {
        return None;
    }
    let lines: Vec<&str> = doc.lines().map(str::trim_end).collect();
    let text = lines
        .iter()
        .map(|l| l.trim_start())
        .collect::<Vec<_>>()
        .join(&format!("\n{indent}"))
        .replace("\"\"\"", "\\\"\\\"\\\"");
    if lines.len() == 1 {
        Some(format!("{indent}\"\"\"{text}\"\"\"\n"))
    } else {
        Some(format!("{indent}\"\"\"{text}\n{indent}\"\"\"\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docstring_keeps_lines_and_escapes_quotes() {
        assert_eq!(
            docstring(Some("One line."), "    ").as_deref(),
            Some("    \"\"\"One line.\"\"\"\n")
        );
        assert_eq!(
            docstring(Some(" First.\n Second \"\"\" here."), "  ").as_deref(),
            Some("  \"\"\"First.\n  Second \\\"\\\"\\\" here.\n  \"\"\"\n")
        );
        assert_eq!(docstring(Some("  \n "), "    "), None);
    }

    #[test]
    fn isort_order_puts_constants_first_and_ignores_case() {
        let names: BTreeSet<String> = ["ShareInfo", "SharedDataObject", "ACL", "Share"]
            .map(String::from)
            .into();
        assert_eq!(
            isort_order(&names),
            ["ACL", "Share", "SharedDataObject", "ShareInfo"]
        );
    }
}
