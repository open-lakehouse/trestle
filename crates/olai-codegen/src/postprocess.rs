//! Rewrites of the buffa model output, whose JSON choices are fixed in the buf plugin:
//!
//! - **Field names.** [`JsonFieldNames::Proto`] serializes the snake_case proto names instead of
//!   lowerCamelCase (both are still read). An explicit `json_name` is kept.
//! - **64-bit integers.** `int64_module` points buffa's string helpers at the `json_int64`
//!   module, which writes numbers.
//! - **Free-form JSON.** `json_value_module` makes `google.protobuf.Value`/`Struct`/`ListValue`
//!   fields write whole numbers as integers (`1`, not `1.0`).
//! - **Field options** ([`field_options`]): explicit `json_name`s.
//!
//! The rewrites are idempotent and skip the files trestle writes itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::error::{Error, Result};
use crate::utils::strings::lower_camel;

/// The entries of the models directory that trestle writes itself.
const TRESTLE_FILES: &[&str] = &[
    "mod.rs",
    "labels.rs",
    "pyo3_impls.rs",
    crate::codegen::EXTRACTORS_SUBDIR,
];

/// How the JSON field names of one proto package are serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JsonFieldNames {
    /// lowerCamelCase field names, buffa's proto3-JSON default.
    #[default]
    Json,
    /// snake_case (proto) field names.
    Proto,
}

/// The rewrites to apply to the buffa model output.
#[derive(Debug, Clone, Default)]
pub struct ModelRewrites {
    /// Field-name style of every package not in [`packages`](Self::packages).
    pub field_names: JsonFieldNames,
    /// Field-name style per proto package (`my.pkg.v1`), overriding
    /// [`field_names`](Self::field_names). A package file is `<package>.rs`.
    pub packages: BTreeMap<String, JsonFieldNames>,
    /// Path of the `json_int64` helper module (e.g. `crate::models::json_int64`); when set,
    /// buffa's 64-bit integer helpers are pointed at it.
    pub int64_module: Option<String>,
    /// Path of the `json_value` helper module (e.g. `crate::models::json_value`); when set, the
    /// free-form JSON fields serialize through it.
    pub json_value_module: Option<String>,
    /// The fields with options that change their JSON, by proto package; see [`field_options`].
    pub fields: BTreeMap<String, PackageFields>,
}

/// The fields of one proto package with JSON options, keyed `Message.field`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageFields {
    /// Explicit `json_name`s, by field.
    pub json_names: BTreeMap<String, String>,
}

/// The fields of `metadata` with JSON options, by package, for [`ModelRewrites::fields`].
pub fn field_options(
    metadata: &crate::parsing::CodeGenMetadata,
) -> BTreeMap<String, PackageFields> {
    let mut packages: BTreeMap<String, PackageFields> = BTreeMap::new();
    for message in metadata.messages.values() {
        let name = message.name.trim_start_matches('.');
        let package = name
            .split('.')
            .take_while(|s| s.starts_with(|c: char| c.is_ascii_lowercase()))
            .collect::<Vec<_>>()
            .join(".");
        let simple = name.rsplit('.').next().unwrap_or(name);
        for field in &message.fields {
            let key = format!("{simple}.{}", field.name);
            if let Some(json) = &field.json_name {
                packages
                    .entry(package.clone())
                    .or_default()
                    .json_names
                    .insert(key, json.clone());
            }
        }
    }
    packages
}

impl ModelRewrites {
    /// Whether any rewrite is enabled.
    pub fn is_empty(&self) -> bool {
        self.field_names == JsonFieldNames::Json
            && self.packages.values().all(|n| *n == JsonFieldNames::Json)
            && self.int64_module.is_none()
            && self.json_value_module.is_none()
            && self.fields.is_empty()
    }

    /// The field-name style of the model file `file_name`: `<package>.rs` (the buf plugin) or
    /// `<package>.<proto file>.rs` (buffa-build).
    fn field_names_for(&self, file_name: &str) -> JsonFieldNames {
        let Some(stem) = file_name.strip_suffix(".rs") else {
            return self.field_names;
        };
        let parent = stem.rsplit_once('.').map(|(package, _)| package);
        self.packages
            .get(stem)
            .or_else(|| parent.and_then(|package| self.packages.get(package)))
            .copied()
            .unwrap_or(self.field_names)
    }

    /// The [`PackageFields`] of the model file `file_name` (see [`Self::field_names_for`]).
    fn fields_for(&self, file_name: &str) -> Option<&PackageFields> {
        let stem = file_name.strip_suffix(".rs")?;
        let parent = stem.rsplit_once('.').map(|(package, _)| package);
        self.fields
            .get(stem)
            .or_else(|| parent.and_then(|package| self.fields.get(package)))
    }

    /// Rewrite the source of the model file `file_name`.
    pub fn rewrite(&self, file_name: &str, src: &str) -> String {
        let mut out = src.to_string();
        if self.field_names_for(file_name) == JsonFieldNames::Proto {
            out = swap_serde_renames(&out);
            out = snakeify_serialize_entries(&out);
        }
        if let Some(fields) = self.fields_for(file_name) {
            out = apply_field_options(&out, fields);
        }
        if let Some(module) = &self.int64_module {
            out = int64_helpers_to(&out, module);
            out = int64_containers_to(&out, module);
        }
        if let Some(module) = &self.json_value_module {
            out = json_value_serializers(&out, module);
        }
        out
    }

    /// Rewrite the model files under `dir` in place. Returns the number of files changed.
    pub fn rewrite_dir(&self, dir: &Path) -> Result<usize> {
        if self.is_empty() || !dir.exists() {
            return Ok(0);
        }
        let mut changed = 0;
        for path in model_files(dir)? {
            let src = std::fs::read_to_string(&path).map_err(Error::Io)?;
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let rewritten = self.rewrite(file_name, &src);
            if rewritten != src {
                std::fs::write(&path, rewritten).map_err(Error::Io)?;
                changed += 1;
            }
        }
        Ok(changed)
    }
}

/// Every `.rs` file under `dir`, sorted, without the [`TRESTLE_FILES`].
fn model_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(Error::Io)? {
            let path = entry.map_err(Error::Io)?.path();
            let trestle_file = d == dir
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| TRESTLE_FILES.contains(&n));
            if trestle_file {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Swap `rename`/`alias` in each `#[serde(...)]` whose `rename` is the default proto3-JSON name
/// of its `alias`, so serialization uses the proto name. An explicit `json_name` is kept.
fn swap_serde_renames(src: &str) -> String {
    // One `#[serde( ... )]` attribute; the body never contains `)]`.
    static SERDE_ATTR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"#\[serde\((?P<body>[^)]*)\)\]").expect("valid regex"));
    static RENAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"rename = "(?P<v>[^"]*)""#).expect("valid regex"));
    static ALIAS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"alias = "(?P<v>[^"]*)""#).expect("valid regex"));

    SERDE_ATTR
        .replace_all(src, |caps: &regex::Captures| {
            let whole = &caps[0];
            let body = &caps["body"];
            let (Some(rn), Some(al)) = (RENAME.captures(body), ALIAS.captures(body)) else {
                return whole.to_string();
            };
            let rename_val = rn["v"].to_string();
            let alias_val = al["v"].to_string();
            // Swap only the default JSON name; this also keeps a second run a no-op.
            if rename_val == alias_val || rename_val != lower_camel(&alias_val) {
                return whole.to_string();
            }
            let body = ALIAS.replace(body, format!(r#"alias = "{rename_val}""#));
            let body = RENAME.replace(&body, format!(r#"rename = "{alias_val}""#));
            format!("#[serde({body})]")
        })
        .into_owned()
}

/// Convert the default `serialize_entry("camelCase", …)` names of the `*View` `Serialize` impls
/// to snake_case.
fn snakeify_serialize_entries(src: &str) -> String {
    static ENTRY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"serialize_entry\("(?P<name>[^"]*)""#).expect("valid regex"));
    ENTRY
        .replace_all(src, |caps: &regex::Captures| {
            let name = &caps["name"];
            let snake = camel_to_snake(name);
            let is_default_json_name =
                name.starts_with(|c: char| c.is_ascii_lowercase()) && lower_camel(&snake) == name;
            if is_default_json_name {
                format!(r#"serialize_entry("{snake}""#)
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

/// Point buffa's `int64`, `opt_int64`, `uint64` and `opt_uint64` helpers at `module`.
fn int64_helpers_to(src: &str, module: &str) -> String {
    static HELPER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"::buffa::json_helpers::(?P<name>(?:opt_)?u?int64)\b").expect("valid regex")
    });
    HELPER
        .replace_all(src, |caps: &regex::Captures| {
            format!("{module}::{}", &caps["name"])
        })
        .into_owned()
}

/// Apply the field options: an explicit `json_name` becomes the `rename` (restoring what
/// [`swap_serde_renames`] may have swapped).
fn apply_field_options(src: &str, fields: &PackageFields) -> String {
    static STRUCT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*pub struct (\w+)").expect("valid regex"));
    static FIELD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*pub (?:r#)?(\w+):").expect("valid regex"));
    static RENAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"rename = "[^"]*""#).expect("valid regex"));
    static ALIAS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"alias = "[^"]*""#).expect("valid regex"));

    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut message = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(caps) = STRUCT.captures(line) {
            message = Some(caps[1].to_string());
        } else if let (Some(message), Some(caps)) = (&message, FIELD.captures(line)) {
            let name = &caps[1];
            let key = format!("{message}.{name}");
            if let Some(json) = fields.json_names.get(&key) {
                let attrs = attribute_lines(&lines, i);
                let text = lines[attrs.clone()].join("\n");
                let text = RENAME
                    .replace(&text, format!(r#"rename = "{json}""#))
                    .into_owned();
                let text = ALIAS
                    .replace(&text, format!(r#"alias = "{name}""#))
                    .into_owned();
                out.truncate(out.len() - attrs.len());
                out.extend(text.lines().map(str::to_string));
            }
        }
        out.push(line.to_string());
    }
    let mut out = out.join("\n");
    if src.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Point repeated `int64`/`uint64` fields and maps of them at the `repeated_int64` and
/// `map_int64` helpers of `module`, instead of buffa's `proto_seq`/`proto_map`.
fn int64_containers_to(src: &str, module: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for i in 0..lines.len() {
        let trimmed = lines[i].trim_start();
        if !trimmed.starts_with("pub ") || !trimmed.contains(':') && !trimmed.ends_with('<') {
            continue;
        }
        let (decl, _) = declaration(&lines, i);
        let int64 = |ty: &str| ty == "i64" || ty == "u64";
        let (from, to) = if decl
            .split_once("Vec<")
            .is_some_and(|(_, rest)| int64(rest.split('>').next().unwrap_or_default()))
        {
            ("::buffa::json_helpers::proto_seq", "repeated_int64")
        } else if decl
            .split_once("HashMap<")
            .and_then(|(_, rest)| rest.rsplit_once('>'))
            .and_then(|(args, _)| args.rsplit_once(','))
            .is_some_and(|(_, value)| int64(value.trim()))
        {
            ("::buffa::json_helpers::proto_map", "map_int64")
        } else {
            continue;
        };
        for j in attribute_lines(&lines, i) {
            out[j] = out[j].replace(from, &format!("{module}::{to}"));
        }
    }
    let mut out = out.join("\n");
    if src.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The field declaration at line `start`, whitespace collapsed (`pub a: Vec<i64>,`), and the
/// line it ends on.
fn declaration(lines: &[&str], start: usize) -> (String, usize) {
    let mut end = start;
    let mut decl = String::new();
    loop {
        decl.push_str(lines[end].trim());
        decl.push(' ');
        let depth = decl.matches('<').count() as isize - decl.matches('>').count() as isize;
        if (depth <= 0 && lines[end].trim_end().ends_with(',')) || end + 1 == lines.len() {
            break;
        }
        end += 1;
    }
    let decl = decl
        .replace(", >", ">")
        .replace("< ", "<")
        .replace(" >", ">");
    (decl, end)
}

/// Give each free-form JSON field (singular, repeated or map value) a `serialize_with` of
/// `module`, unless it has one already.
fn json_value_serializers(src: &str, module: &str) -> String {
    const JSON: &str = "::buffa_types::google::protobuf::";
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::with_capacity(src.len() + 256);
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if trimmed.starts_with("pub ") && trimmed.contains(": ") && line.contains(JSON) || {
            // A type that rustfmt broke onto the next lines.
            trimmed.starts_with("pub ") && trimmed.ends_with('<')
        } {
            let (decl, end) = declaration(&lines, i);
            let kind = if !decl.contains(JSON) {
                None
            } else if decl.contains(": ::buffa::MessageField<") {
                Some("message_field")
            } else if decl.contains(": ::buffa::alloc::vec::Vec<") {
                Some("repeated")
            } else if decl.contains("HashMap<") {
                Some("map")
            } else {
                None
            };
            let attrs = field_attributes(&lines, i);
            if let Some(kind) = kind
                && !has_serde_key(&attrs, "serialize_with")
                && !has_serde_key(&attrs, "with")
            {
                let indent = &line[..line.len() - trimmed.len()];
                out.push_str(&format!(
                    "{indent}#[serde(serialize_with = \"{module}::{kind}\")]\n"
                ));
            }
            for l in &lines[i..=end] {
                out.push_str(l);
                out.push('\n');
            }
            i = end + 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
        i += 1;
    }
    if !src.ends_with('\n') {
        out.pop();
    }
    out
}

/// Whether the serde attributes `attrs` set `key` as a whole word (`deserialize_with` does not
/// set `with`).
fn has_serde_key(attrs: &str, key: &str) -> bool {
    attrs.match_indices(key).any(|(at, _)| {
        let before = attrs[..at].chars().next_back();
        let after = attrs[at + key.len()..].trim_start();
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && after.starts_with('=')
    })
}

/// The attributes above the field declaration at line `field`, as one string.
fn field_attributes(lines: &[&str], field: usize) -> String {
    attribute_lines(lines, field)
        .map(|j| lines[j].trim())
        .collect()
}

/// The lines of the attributes and doc comments above the field declaration at line `field`.
fn attribute_lines(lines: &[&str], field: usize) -> std::ops::Range<usize> {
    let mut j = field;
    while j > 0 {
        let prev = lines[j - 1].trim();
        if prev.starts_with("//") || prev.starts_with("#[") && prev.ends_with(']') {
            j -= 1;
        } else if prev.ends_with(")]") {
            // A multi-line attribute: back up to its `#[`.
            let mut k = j - 1;
            while k > 0 && !lines[k].trim_start().starts_with("#[") {
                k -= 1;
            }
            j = k;
        } else {
            break;
        }
    }
    j..field
}

/// Invert the proto3 JSON mapping: each uppercase letter becomes `_` + its lowercase.
fn camel_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Put blank lines between the items of a pretty-printed Rust file (prettyplease prints none;
/// rustfmt keeps them but never adds them): before an item that follows a `}` or a `use` group
/// at the same indentation, and after the file's inner attributes. Idempotent.
pub fn space_items(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::with_capacity(src.len() + src.len() / 16);
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line);
        out.push('\n');
        if lines
            .get(i + 1)
            .is_some_and(|next| blank_between(line, next))
        {
            out.push('\n');
        }
    }
    out
}

/// Whether a blank line goes between the lines `line` and `next`.
fn blank_between(line: &str, next: &str) -> bool {
    let indent = |l: &str| l.len() - l.trim_start().len();
    if line.trim().is_empty() || next.trim().is_empty() || indent(line) != indent(next) {
        return false;
    }
    let (line, next) = (line.trim(), next.trim());
    let is_use = |l: &str| {
        ["use ", "pub use ", "pub(crate) use ", "pub(super) use "]
            .iter()
            .any(|p| l.starts_with(p))
    };
    let is_inner = |l: &str| l.starts_with("//!") || l.starts_with("#![");
    if is_inner(line) {
        return !is_inner(next);
    }
    if !starts_item(next) {
        return false;
    }
    match line {
        "}" => true,
        // A multi-line `use` ends with `};`: the next `use` stays in the group.
        "};" => !is_use(next),
        _ => is_use(line) && line.ends_with(';') && !is_use(next),
    }
}

/// Whether `line` (trimmed) starts an item, or the attributes or docs of one.
fn starts_item(line: &str) -> bool {
    const STARTS: &[&str] = &[
        "#[",
        "///",
        "/**",
        "pub ",
        "pub(",
        "fn ",
        "async fn ",
        "const fn ",
        "unsafe ",
        "extern ",
        "impl ",
        "impl<",
        "struct ",
        "enum ",
        "trait ",
        "mod ",
        "type ",
        "const ",
        "static ",
        "use ",
        "macro_rules!",
        "include!",
    ];
    STARTS.iter().any(|p| line.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proto() -> ModelRewrites {
        ModelRewrites {
            field_names: JsonFieldNames::Proto,
            ..Default::default()
        }
    }

    #[test]
    fn camel_to_snake_conversions() {
        assert_eq!(camel_to_snake("displayName"), "display_name");
        assert_eq!(camel_to_snake("nextPageToken"), "next_page_token");
        assert_eq!(camel_to_snake("name"), "name");
    }

    #[test]
    fn snakeifies_view_serialize_entry_literal() {
        let src = r#"                __map.serialize_entry("displayName", self.display_name)?;"#;
        let out = proto().rewrite("demo.v1.rs", src);
        assert!(out.contains(r#"serialize_entry("display_name", self.display_name)"#));
    }

    #[test]
    fn view_serialize_entry_explicit_names_untouched() {
        for src in [
            r#"__map.serialize_entry("name", self.name)?;"#,
            r#"__map.serialize_entry("metadata-location", self.metadata_location)?;"#,
            r#"__map.serialize_entry("Resources", self.resources)?;"#,
        ] {
            assert_eq!(proto().rewrite("demo.v1.rs", src), src);
        }
    }

    #[test]
    fn swaps_multiword_rename_alias_pair() {
        let src = r#"    #[serde(
        rename = "storageRoot",
        alias = "storage_root",
        skip_serializing_if = "::core::option::Option::is_none"
    )]
    pub storage_root: Option<String>,"#;
        let out = swap_serde_renames(src);
        assert!(out.contains(r#"rename = "storage_root""#));
        assert!(out.contains(r#"alias = "storageRoot""#));
        assert!(out.contains(r#"skip_serializing_if = "::core::option::Option::is_none""#));
    }

    #[test]
    fn single_word_rename_only_is_untouched() {
        let src = r#"    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]"#;
        assert_eq!(swap_serde_renames(src), src);
    }

    #[test]
    fn explicit_json_name_is_kept() {
        // `json_name = "metadata-location"`: not the default JSON name of `metadata_location`.
        let src = r#"#[serde(rename = "metadata-location", alias = "metadata_location")]"#;
        assert_eq!(swap_serde_renames(src), src);
        let src = r#"#[serde(rename = "Resources", alias = "resources")]"#;
        assert_eq!(swap_serde_renames(src), src);
    }

    #[test]
    fn enum_and_scalar_with_helpers_preserved() {
        let src = r#"    #[serde(
        rename = "catalogType",
        alias = "catalog_type",
        with = "::buffa::json_helpers::opt_enum",
        skip_serializing_if = "::core::option::Option::is_none"
    )]"#;
        let out = swap_serde_renames(src);
        assert!(out.contains(r#"rename = "catalog_type""#));
        assert!(out.contains(r#"alias = "catalogType""#));
        assert!(out.contains(r#"with = "::buffa::json_helpers::opt_enum""#));
    }

    #[test]
    fn swap_is_idempotent_on_already_swapped_output() {
        let camel = r#"    #[serde(rename = "storageRoot", alias = "storage_root", skip_serializing_if = "x")]"#;
        let once = swap_serde_renames(camel);
        assert!(once.contains(r#"rename = "storage_root", alias = "storageRoot""#));
        assert_eq!(swap_serde_renames(&once), once);
    }

    #[test]
    fn non_field_serde_attr_is_untouched() {
        assert_eq!(swap_serde_renames("#[serde(default)]"), "#[serde(default)]");
    }

    #[test]
    fn package_override_keeps_camel_case() {
        let mut rewrites = proto();
        rewrites
            .packages
            .insert("demo.scim.v2".into(), JsonFieldNames::Json);
        let src = r#"#[serde(rename = "userName", alias = "user_name")]"#;
        assert_eq!(rewrites.rewrite("demo.scim.v2.rs", src), src);
        assert_eq!(rewrites.rewrite("demo.scim.v2.users.rs", src), src);
        assert!(
            rewrites
                .rewrite("demo.v1.rs", src)
                .contains(r#"rename = "user_name""#)
        );
    }

    #[test]
    fn field_options_set_json_names() {
        let src = r#"pub struct Spec {
    #[serde(rename = "spec_id", alias = "specId")]
    pub spec_id: i32,
}
pub struct Other {
    #[serde(rename = "spec_id", alias = "specId")]
    pub spec_id: i32,
}
"#;
        let fields = PackageFields {
            json_names: [("Spec.spec_id".to_string(), "specId".to_string())].into(),
        };
        let out = apply_field_options(src, &fields);
        assert!(
            out.contains(r#"rename = "specId", alias = "spec_id""#),
            "{out}"
        );
        // Only `Spec.spec_id` is renamed.
        assert_eq!(out.matches(r#"rename = "specId""#).count(), 1, "{out}");
    }

    #[test]
    fn int64_containers_write_numbers() {
        let src = r#"    #[serde(rename = "sizes", with = "::buffa::json_helpers::proto_seq")]
    pub sizes: ::buffa::alloc::vec::Vec<i64>,
    #[serde(
        rename = "counts",
        with = "::buffa::json_helpers::proto_map"
    )]
    pub counts: ::buffa::__private::HashMap<
        ::buffa::alloc::string::String,
        i64,
    >,
    #[serde(rename = "ratios", with = "::buffa::json_helpers::proto_seq")]
    pub ratios: ::buffa::alloc::vec::Vec<f64>,
"#;
        let out = int64_containers_to(src, "crate::models::json_int64");
        assert!(out.contains(r#"with = "crate::models::json_int64::repeated_int64""#));
        assert!(out.contains(r#"with = "crate::models::json_int64::map_int64""#));
        assert_eq!(
            out.matches("::buffa::json_helpers::proto_seq").count(),
            1,
            "{out}"
        );
        assert!(out.ends_with(",\n"));
    }

    #[test]
    fn int64_helpers_point_at_module() {
        let rewrites = ModelRewrites {
            int64_module: Some("crate::models::json_int64".into()),
            ..Default::default()
        };
        let src = r#"with = "::buffa::json_helpers::opt_int64",
            ::buffa::json_helpers::int64::serialize(&self.0, __s)
            with = "::buffa::json_helpers::uint64",
            with = "::buffa::json_helpers::int32","#;
        let out = rewrites.rewrite("demo.v1.rs", src);
        assert!(out.contains(r#"with = "crate::models::json_int64::opt_int64""#));
        assert!(out.contains("crate::models::json_int64::int64::serialize(&self.0, __s)"));
        assert!(out.contains(r#"with = "crate::models::json_int64::uint64""#));
        assert!(out.contains(r#"with = "::buffa::json_helpers::int32""#));
        // Idempotent.
        assert_eq!(rewrites.rewrite("demo.v1.rs", &out), out);
    }

    #[test]
    fn rewrite_dir_leaves_trestle_files_alone() {
        let rewrites = ModelRewrites {
            int64_module: Some("crate::models::json_int64".into()),
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let src = r#"with = "::buffa::json_helpers::int64","#;
        std::fs::create_dir(dir.path().join("extractors")).unwrap();
        for file in ["demo.v1.rs", "mod.rs", "extractors/mod.rs"] {
            std::fs::write(dir.path().join(file), src).unwrap();
        }
        assert_eq!(rewrites.rewrite_dir(dir.path()).unwrap(), 1);
        let read = |file: &str| std::fs::read_to_string(dir.path().join(file)).unwrap();
        assert!(read("demo.v1.rs").contains("crate::models::json_int64::int64"));
        assert_eq!(read("mod.rs"), src);
        assert_eq!(read("extractors/mod.rs"), src);
    }

    #[test]
    fn space_items_separates_items_and_functions() {
        let src = "//! Docs.
use a::B;
use c::D;
/// A thing.
pub struct S {
    x: u8,
}
impl S {
    fn a(&self) {
        if true {
            f();
        }
        g();
    }
    #[inline]
    fn b(&self) -> u8 {
        match self.x {
            0 => {
                1
            }
            _ => 2,
        }
    }
}
";
        let spaced = space_items(src);
        assert_eq!(
            spaced,
            "//! Docs.

use a::B;
use c::D;

/// A thing.
pub struct S {
    x: u8,
}

impl S {
    fn a(&self) {
        if true {
            f();
        }
        g();
    }

    #[inline]
    fn b(&self) -> u8 {
        match self.x {
            0 => {
                1
            }
            _ => 2,
        }
    }
}
"
        );
        assert_eq!(space_items(&spaced), spaced);
    }

    #[test]
    fn json_value_fields_serialize_through_the_module() {
        let rewrites = ModelRewrites {
            json_value_module: Some("crate::models::json_value".into()),
            ..Default::default()
        };
        let src = r#"    #[serde(rename = "attributes")]
    pub attributes: ::buffa::MessageField<::buffa_types::google::protobuf::Struct>,
    #[serde(rename = "notes", deserialize_with = "::buffa::json_helpers::null_as_default")]
    pub notes: ::buffa::alloc::vec::Vec<::buffa_types::google::protobuf::Value>,
    #[serde(rename = "rows")]
    pub rows: ::buffa::__private::HashMap<
        ::buffa::alloc::string::String,
        ::buffa_types::google::protobuf::ListValue,
    >,
    #[serde(rename = "name")]
    pub name: ::buffa::alloc::string::String,
"#;
        let out = rewrites.rewrite("demo.v1.rs", src);
        for kind in ["message_field", "repeated", "map"] {
            assert!(
                out.contains(&format!(
                    "#[serde(serialize_with = \"crate::models::json_value::{kind}\")]"
                )),
                "{out}"
            );
        }
        assert_eq!(
            out.matches("serialize_with = \"crate::models").count(),
            3,
            "{out}"
        );
        assert!(out.contains("    pub rows: ::buffa::__private::HashMap<\n"));
    }
}
