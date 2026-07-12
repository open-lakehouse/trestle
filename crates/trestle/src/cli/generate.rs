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

use crate::config::{TrestleConfig, merge_buf_gen};
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

    print_summary(&config);
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
