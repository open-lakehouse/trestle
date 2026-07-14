//! `trestle generate` — proto-driven code generation.
//!
//! This is the single entry point for the whole proto→code pipeline. By default
//! it drives the two `buf` steps itself and then runs trestle's own codegen:
//!
//! 1. `buf generate` — runs the model plugins via `buf.gen.yaml` (prost/buffa,
//!    Connect facade) to write the proto message types. trestle keeps its
//!    *managed* plugins in `buf.gen.yaml` in sync with `trestle.yaml` on every run
//!    while preserving any plugins an adopter added (see
//!    [`merge_buf_gen`](crate::config::merge_buf_gen)).
//! 2. `buf build` — compiles `proto/` to a `FileDescriptorSet` (a temp file).
//! 3. trestle codegen — parses the descriptor and emits handlers/clients/bindings.
//!
//! For environments where `buf` isn't available (e.g. a central/Bazel build that
//! produces the descriptor upstream), pass `--descriptors <path>` to skip the
//! `buf` steps and consume a pre-built descriptor directly.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use olai_codegen::{CodeGenConfig, generate_code, parse_file_descriptor_set};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;
use regex::Regex;
use walkdir::WalkDir;

use crate::config::{JsonFieldNames, ProtoLib, TrestleConfig, merge_buf_gen};
use crate::error::{Error, Result};

#[derive(Args, Clone)]
pub struct GenerateArgs {
    /// Path to a `trestle.yaml` config file. A few CLI flags override values from it.
    #[clap(long, short = 'c', default_value = "trestle.yaml")]
    pub config: PathBuf,

    /// Consume a pre-built protobuf `FileDescriptorSet` instead of building one.
    ///
    /// Escape hatch for environments where `buf` isn't available (e.g. a central
    /// or Bazel build produces the descriptor upstream). When set, the `buf
    /// generate` / `buf build` steps are skipped and this descriptor is used as-is.
    #[clap(long, short)]
    pub descriptors: Option<PathBuf>,

    /// Skip running language formatters (`cargo fmt` / `biome` / `ruff`) on the
    /// generated output.
    ///
    /// Formatting runs by default so a fresh regen is byte-identical to a
    /// formatted tree (no churn). Missing formatters are warned about and
    /// skipped, so this flag is only needed to suppress formatting entirely.
    #[clap(long)]
    pub no_format: bool,
}

pub fn run(args: GenerateArgs) -> Result<()> {
    let mut cfg = TrestleConfig::load(&args.config)?;
    cfg.derive_defaults();
    // `generate` never prompts or mutates the config itself. A buffa-requiring
    // selection on a prost config is a hard error here.
    cfg.validate(false)?;

    // Directory that holds `buf.yaml` / `buf.gen.yaml` — the config's parent, so
    // `trestle generate -c path/to/trestle.yaml` runs buf in the right place.
    let project_dir = args
        .config
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Obtain a descriptor: either the escape-hatch path, or build one with buf.
    let descriptor_bytes = match &args.descriptors {
        Some(path) => read_descriptor(path)?,
        None => build_with_buf(&cfg, &project_dir)?,
    };

    let file_descriptor_set = FileDescriptorSet::parse_from_bytes(&descriptor_bytes)
        .map_err(|e| Error::other(format!("failed to parse descriptor: {e}")))?;

    let metadata = parse_file_descriptor_set(&file_descriptor_set)?;
    let config = cfg.to_codegen_config()?;
    generate_code(&metadata, &config)?;

    // Rewrite buffa's camelCase JSON field names to snake_case when requested, so
    // the models serialize proto (snake_case) JSON field names (e.g. Unity Catalog
    // wire-compat). Runs after all `.rs` are on disk but before `cargo fmt`, so the
    // subsequent format pass normalizes any whitespace the edit introduces.
    if cfg.generate.proto_lib == ProtoLib::Buffa
        && cfg.generate.json_field_names == JsonFieldNames::Proto
    {
        rewrite_serde_field_names(&config)?;
    }

    // Format the emitted code so a fresh regen matches an already-formatted tree.
    // Best-effort: a missing/failing formatter is warned about, never fatal.
    if !args.no_format {
        format_generated(&config, &project_dir);
    }

    print_summary(&config);
    print_clippy_hint(&project_dir);
    Ok(())
}

/// Read a pre-built descriptor (escape-hatch path). Reads before canonicalizing so
/// a missing file reports the path the user actually passed.
fn read_descriptor(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| {
        Error::io_at(path, e).with_hint(format!(
            "descriptor `{}` not found — build one with `buf build -o <file>`, \
             or omit --descriptors to let trestle run buf for you",
            path.display()
        ))
    })
}

/// Default path: reconcile `buf.gen.yaml`, run `buf generate` (models) and
/// `buf build` (descriptor into a temp file), and return the descriptor bytes.
fn build_with_buf(cfg: &TrestleConfig, project_dir: &Path) -> Result<Vec<u8>> {
    reconcile_buf_gen(cfg, project_dir)?;

    // 1. buf generate — model plugins write the proto message types.
    let spinner = cliclack::spinner();
    spinner.start("buf generate (models)");
    run_buf(&["generate"], project_dir)?;
    spinner.stop("buf generate (models)");

    // 2. buf build — compile proto/ to a FileDescriptorSet in a temp file.
    let tmp = tempfile::Builder::new()
        .prefix("trestle-descriptor-")
        .suffix(".bin")
        .tempfile()
        .map_err(|e| Error::other(format!("failed to create temp descriptor file: {e}")))?;
    let tmp_path = tmp.path().to_path_buf();

    let spinner = cliclack::spinner();
    spinner.start("buf build (descriptor)");
    run_buf(&["build", "-o", &tmp_path.to_string_lossy()], project_dir)?;
    spinner.stop("buf build (descriptor)");

    fs::read(&tmp_path).map_err(|e| Error::io_at(&tmp_path, e))
}

/// Rewrite the project's `buf.gen.yaml` so trestle's managed plugins match
/// `trestle.yaml`, preserving any adopter-added plugins.
fn reconcile_buf_gen(cfg: &TrestleConfig, project_dir: &Path) -> Result<()> {
    let path = project_dir.join("buf.gen.yaml");
    let (merged, current) = match fs::read_to_string(&path) {
        Ok(existing) => (merge_buf_gen(&cfg.generate, &existing)?, Some(existing)),
        // No file yet: emit the whole thing.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (crate::config::emit_buf_gen(&cfg.generate), None)
        }
        Err(e) => return Err(Error::io_at(&path, e)),
    };
    // Don't rewrite an already-correct file (avoids needless mtime churn / diffs).
    if current.as_deref() == Some(merged.as_str()) {
        return Ok(());
    }
    fs::write(&path, merged).map_err(|e| Error::io_at(&path, e))
}

/// Run `buf <args>` in `dir`, mapping a missing binary and non-zero exit to
/// actionable errors.
fn run_buf(args: &[&str], dir: &Path) -> Result<()> {
    let status = Command::new("buf")
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::other("`buf` not found on PATH").with_hint(
                    "install buf (https://buf.build/docs/installation) or pass a \
                     pre-built descriptor with `--descriptors <path>`",
                )
            } else {
                Error::other(format!("failed to run `buf {}`: {e}", args.join(" ")))
            }
        })?;
    if !status.success() {
        return Err(Error::other(format!(
            "`buf {}` exited with {status}",
            args.join(" ")
        )));
    }
    Ok(())
}

/// Run the language formatters over the emitted output so a fresh regen is
/// byte-identical to an already-formatted tree.
///
/// Best-effort: each formatter is dispatched only for languages that were
/// actually emitted, and a missing or failing formatter degrades to a warning
/// (see [`run_formatter`]) rather than aborting a successful generation.
fn format_generated(config: &CodeGenConfig, project_dir: &Path) {
    let out = &config.output;

    // Rust: run `cargo fmt` in the project. We deliberately shell out to
    // `cargo fmt` rather than invoking `rustfmt` on a hand-collected file list —
    // `cargo fmt` resolves the module tree from the crate roots (so nested `mod`s
    // format in their parent's context), honors the `// @generated` marker (so the
    // buf-plugin model files are left as the plugin emitted them), and discovers
    // the workspace edition + any `rustfmt.toml`. That makes generate's output a
    // fixed point for a developer's `cargo fmt` / pre-commit hook and CI — the
    // least-churn outcome. A bare `rustfmt <files>` pass subtly diverges from
    // `cargo fmt` on both counts.
    run_formatter(
        "cargo",
        &["fmt"],
        project_dir,
        "cargo fmt (rust)",
        "install rustfmt with `rustup component add rustfmt`",
    );

    // TypeScript: the TS client dir, plus the `.d.ts` co-located in the wasm dir.
    let ts_dirs: Vec<&PathBuf> = [&out.node_ts, &out.wasm].into_iter().flatten().collect();
    if !ts_dirs.is_empty() {
        let mut args = vec!["format", "--write"];
        let ts_paths: Vec<String> = ts_dirs.iter().map(|p| p.display().to_string()).collect();
        args.extend(ts_paths.iter().map(String::as_str));
        run_formatter(
            "biome",
            &args,
            project_dir,
            "biome format (typescript)",
            "install biome (https://biomejs.dev/guides/getting-started/)",
        );
    }

    // Python: PyO3 binding wrappers + typings. Run ruff via uvx (no local install).
    if let Some(py) = &out.python {
        run_formatter(
            "uvx",
            &["ruff", "format", &py.display().to_string()],
            project_dir,
            "ruff format (python)",
            "install uv to get uvx (https://docs.astral.sh/uv/getting-started/installation/)",
        );
    }
}

/// Rewrite buffa's generated JSON field names from camelCase to snake_case (proto
/// names) across every `.rs` file in the model output, so the models serialize
/// snake_case while still accepting camelCase on read (e.g. Unity Catalog
/// wire-compat). See [`snakeify_json_field_names`] for the exact transform.
///
/// Scoped to the buffa model output (`models_subdir_path`) — the only place these
/// plugin-emitted names live. A no-op when the config has no models dir.
fn rewrite_serde_field_names(config: &CodeGenConfig) -> Result<()> {
    let Some(gen_dir) = config.output.models_subdir_path() else {
        return Ok(());
    };
    if !gen_dir.exists() {
        return Ok(());
    }

    let mut files_changed = 0usize;
    for entry in WalkDir::new(&gen_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let src = fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
        let rewritten = snakeify_json_field_names(&src);
        if rewritten != src {
            fs::write(path, rewritten.as_bytes()).map_err(|e| Error::io_at(path, e))?;
            files_changed += 1;
        }
    }

    tracing::info!(
        "rewrote serde field names to snake_case in {files_changed} file(s) under {}",
        gen_dir.display()
    );
    Ok(())
}

/// Rewrite buffa's JSON field names in `src` from camelCase to snake_case,
/// returning the rewritten source. Pure over its input so it can be unit-tested
/// without touching the filesystem.
///
/// buffa emits the JSON name in two places, both handled here:
///
/// 1. On owned message structs (`#[derive(Serialize)]`) via
///    `#[serde(rename = "camelCase", alias = "snake_case")]` — the two values are
///    swapped so serialization emits snake_case while still accepting camelCase on
///    read. Attributes with only a `rename` (single-word fields, already snake) or
///    none (`#[serde(default)]`) are left unchanged.
/// 2. On zero-copy `*View` structs, whose hand-written `Serialize` impls call
///    `__map.serialize_entry("camelCase", …)` with the name as a string literal
///    (bypassing serde attributes) — the literal is converted to snake_case.
///
/// Everything else in each attribute/call (`with = "…"`, `skip_serializing_if`,
/// the value expression) is preserved byte-for-byte, so the hand-rolled enum/scalar
/// serde is unaffected.
fn snakeify_json_field_names(src: &str) -> String {
    let src = swap_serde_renames(src);
    snakeify_serialize_entries(&src)
}

/// Swap the `rename`/`alias` values in `#[serde(...)]` attributes that have both.
fn swap_serde_renames(src: &str) -> String {
    // Match one `#[serde( ... )]` attribute; the body never contains `)]`.
    let serde_attr = Regex::new(r"#\[serde\((?P<body>[^)]*)\)\]").expect("valid regex");
    let rename_re = Regex::new(r#"rename = "(?P<v>[^"]*)""#).expect("valid regex");
    let alias_re = Regex::new(r#"alias = "(?P<v>[^"]*)""#).expect("valid regex");

    serde_attr
        .replace_all(src, |caps: &regex::Captures| {
            let whole = &caps[0];
            let body = &caps["body"];
            let (Some(rn), Some(al)) = (rename_re.captures(body), alias_re.captures(body)) else {
                return whole.to_string();
            };
            let rename_val = rn["v"].to_string();
            let alias_val = al["v"].to_string();
            if rename_val == alias_val {
                return whole.to_string();
            }
            // Idempotency guard: only swap when `rename` is still the camelCase name
            // buffa emits (i.e. it differs from its own snake_case form). If `rename`
            // is already snake_case, this attribute was processed on a prior run —
            // swapping again would flip it *back* to camelCase and break wire-compat.
            // This matters on the `--descriptors` path, where buffa does not overwrite
            // the on-disk models first, so the rewrite may see already-snake output.
            if rename_val == camel_to_snake(&rename_val) {
                return whole.to_string();
            }
            // rename ⇄ alias: rename takes the old (snake) alias, alias takes the
            // old (camel) rename.
            let body = alias_re.replace(body, format!(r#"alias = "{rename_val}""#));
            let body = rename_re.replace(&body, format!(r#"rename = "{alias_val}""#));
            format!("#[serde({body})]")
        })
        .into_owned()
}

/// Convert `serialize_entry("camelCase", …)` JSON-name literals (emitted in the
/// `*View` `Serialize` impls) to snake_case.
fn snakeify_serialize_entries(src: &str) -> String {
    let entry_re = Regex::new(r#"serialize_entry\("(?P<name>[^"]*)""#).expect("valid regex");
    entry_re
        .replace_all(src, |caps: &regex::Captures| {
            let snake = camel_to_snake(&caps["name"]);
            format!(r#"serialize_entry("{snake}""#)
        })
        .into_owned()
}

/// Convert a lowerCamelCase proto3-JSON field name to its snake_case proto name.
///
/// Inverts the proto3 JSON mapping (`my_field` → `myField`): each uppercase letter
/// becomes `_` + its lowercase. Already-snake names (no uppercase) are returned
/// unchanged, so it is a safe identity on single-word and already-converted names.
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

/// Run a formatter, degrading a missing binary or non-zero exit to a warning.
///
/// Unlike [`run_buf`], formatting is best-effort: generation has already
/// succeeded, so a formatter that is absent or fails must not abort the run. When
/// the binary isn't on `PATH`, `install_hint` tells the user how to get it.
fn run_formatter(program: &str, args: &[&str], dir: &Path, label: &str, install_hint: &str) {
    let spinner = cliclack::spinner();
    spinner.start(label);
    match Command::new(program).args(args).current_dir(dir).status() {
        Ok(status) if status.success() => spinner.stop(label),
        Ok(status) => {
            spinner.stop(label);
            tracing::warn!("`{program}` exited with {status}; leaving output unformatted");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            spinner.stop(label);
            tracing::warn!("`{program}` not found on PATH; skipping {label} — {install_hint}");
        }
        Err(e) => {
            spinner.stop(label);
            tracing::warn!("failed to run `{program}`: {e}; skipping {label}");
        }
    }
}

/// Print the directories code was generated into, always (not log-gated), so a
/// successful run gives visible feedback.
fn print_summary(config: &CodeGenConfig) {
    let out = &config.output;
    let mut dirs: Vec<String> = Vec::new();
    let mut push = |p: &Path| dirs.push(p.display().to_string());
    if let Some(m) = &out.models {
        push(&m.join(&out.models_subdir));
    }
    push(&out.common);
    for p in [&out.server, &out.client, &out.python, &out.node]
        .into_iter()
        .flatten()
    {
        push(p);
    }
    dirs.sort();
    dirs.dedup();

    println!("Generated code into:");
    for d in dirs {
        println!("  {d}");
    }
}

/// Print a copy-pasteable `clippy --fix` command as a hint.
///
/// We deliberately don't run `clippy --fix` in-pipeline (it's semantic and slow),
/// but a lint-fix pass over the generated code is still worth doing. Surfacing the
/// exact command — always, even under `--no-format` — lets consuming agents/users
/// run it without guessing. Scoped to `project_dir` so it targets the generated
/// crates rather than an unrelated workspace.
fn print_clippy_hint(project_dir: &Path) {
    let clippy = "cargo clippy --fix --allow-dirty --all-targets -- -D warnings";
    println!("\nTip: lint the generated code with:");
    // When generating in-place (project_dir is "."), the plain command is enough;
    // otherwise wrap it in a `cd` so it's copy-pasteable from anywhere.
    if project_dir.as_os_str() == "." {
        println!("  {clippy}");
    } else {
        println!("  (cd {} && {clippy})", project_dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::{camel_to_snake, snakeify_json_field_names, swap_serde_renames};

    #[test]
    fn camel_to_snake_conversions() {
        assert_eq!(camel_to_snake("displayName"), "display_name");
        assert_eq!(camel_to_snake("storageRoot"), "storage_root");
        assert_eq!(camel_to_snake("nextPageToken"), "next_page_token");
        // Already snake / single-word: identity.
        assert_eq!(camel_to_snake("name"), "name");
        assert_eq!(camel_to_snake("id"), "id");
    }

    #[test]
    fn snakeifies_view_serialize_entry_literal() {
        // The zero-copy View `Serialize` impl hardcodes the JSON name.
        let src = r#"                __map.serialize_entry("displayName", self.display_name)?;"#;
        let out = snakeify_json_field_names(src);
        assert!(out.contains(r#"serialize_entry("display_name", self.display_name)"#));
    }

    #[test]
    fn view_serialize_entry_single_word_untouched() {
        let src = r#"__map.serialize_entry("name", self.name)?;"#.to_string();
        assert_eq!(snakeify_json_field_names(&src), src);
    }

    #[test]
    fn swaps_multiword_rename_alias_pair() {
        // Multi-line attribute as buffa actually emits it.
        let src = r#"    #[serde(
        rename = "storageRoot",
        alias = "storage_root",
        skip_serializing_if = "::core::option::Option::is_none"
    )]
    pub storage_root: Option<String>,"#;
        let out = swap_serde_renames(src);
        assert!(out.contains(r#"rename = "storage_root""#));
        assert!(out.contains(r#"alias = "storageRoot""#));
        // Untouched options are preserved.
        assert!(out.contains(r#"skip_serializing_if = "::core::option::Option::is_none""#));
    }

    #[test]
    fn single_word_rename_only_is_untouched() {
        // Single-word field: buffa emits `rename` with no `alias` (snake == camel).
        let src =
            r#"    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]"#.to_string();
        assert_eq!(swap_serde_renames(&src), src);
    }

    #[test]
    fn enum_and_scalar_with_helpers_preserved() {
        // The `with = "..."` helper (hand-rolled enum/scalar serde) must survive the
        // swap unchanged; only the rename/alias values move.
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
        // On the `--descriptors` path buffa does not overwrite the on-disk models, so a
        // second `generate` run sees already-swapped output. Re-running must be a no-op:
        // the snake `rename` must NOT flip back to camelCase (which would break wire-compat).
        let camel = r#"    #[serde(rename = "storageRoot", alias = "storage_root", skip_serializing_if = "x")]"#;
        let once = swap_serde_renames(camel);
        assert!(once.contains(r#"rename = "storage_root", alias = "storageRoot""#));
        // Second application is a fixed point.
        assert_eq!(swap_serde_renames(&once), once);
    }

    #[test]
    fn non_field_serde_attr_is_untouched() {
        // Struct-level `#[serde(default)]` has neither rename nor alias.
        let src = "#[serde(default)]".to_string();
        assert_eq!(swap_serde_renames(&src), src);
    }

    #[test]
    fn swaps_only_within_one_attribute_across_a_file() {
        // Two fields in one blob: a multi-word pair swaps, a single-word rename does
        // not, and each `#[serde(...)]` is handled independently.
        let src = r#"    #[serde(rename = "pageToken", alias = "page_token", skip_serializing_if = "x")]
    pub page_token: Option<String>,
    #[serde(rename = "name")]
    pub name: String,"#;
        let out = swap_serde_renames(src);
        assert!(out.contains(r#"rename = "page_token", alias = "pageToken""#));
        assert!(out.contains(r#"#[serde(rename = "name")]"#));
    }
}
