//! Golden-snapshot regression oracle for the full generation path.
//!
//! Unlike the substring assertions in `generation_integration.rs`, this test generates **every**
//! output (common, models, server, client, python, node, node_ts) for the committed `example.bin`
//! fixture and compares the entire output tree byte-for-byte against committed expected files under
//! `tests/golden/`.
//!
//! This is the behavior-preservation oracle for the IR-enrichment refactor: a refactor that is meant
//! to leave generated output unchanged must keep this test green, and any *intended* output change
//! must show up as a reviewable diff in the committed golden tree.
//!
//! ## Updating the golden tree
//!
//! When a change *intentionally* alters generated output, re-bless the snapshot:
//!
//! ```sh
//! BLESS=1 cargo test -p olai-codegen --test golden_integration
//! ```
//!
//! This rewrites `tests/golden/` from the current generator output. Review the resulting `git diff`
//! to confirm the change is what you expected, then commit it alongside the code change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use olai_codegen::parsing::parse_file_descriptor_set;
use olai_codegen::{BindingsConfig, CodeGenConfig, CodeGenMetadata, CodeGenOutput, generate_code};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;
use tempfile::TempDir;

fn metadata() -> CodeGenMetadata {
    let bytes = include_bytes!("../proto/example.bin");
    let fds = FileDescriptorSet::parse_from_bytes(bytes).expect("valid descriptor");
    parse_file_descriptor_set(&fds).expect("parse succeeds")
}

/// Collect every file under `dir`, returned as paths relative to `dir`, sorted for determinism.
fn relative_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    out.push(path.strip_prefix(base).expect("under base").to_path_buf());
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    files
}

/// Build a config that drives **all** generators (Rust + every binding language) into `tmp`.
fn full_config(tmp: &Path) -> CodeGenConfig {
    let common = tmp.join("common");
    let models = tmp.join("models");
    let server = tmp.join("server");
    let client = tmp.join("client");
    let python = tmp.join("python");
    let node = tmp.join("node");
    let node_ts = tmp.join("node_ts");
    for d in [&common, &models, &server, &client, &python, &node, &node_ts] {
        std::fs::create_dir_all(d).expect("create dir");
    }
    CodeGenConfig {
        context_type_path: "crate::Context".into(),
        result_type_path: "crate::Result".into(),
        models_path_template: "example_common::models::{service}::v1".into(),
        models_path_crate_template: "crate::models::{service}::v1".into(),
        resource_store_crate_name: "olai_store".into(),
        output: CodeGenOutput {
            common,
            models: Some(models),
            models_subdir: "_gen".into(),
            server: Some(server),
            client: Some(client),
            python: Some(python),
            node: Some(node),
            node_ts: Some(node_ts),
            python_typings_filename: "client.pyi".into(),
        },
        generate_resource_enum: true,
        generate_store_integration: true,
        error_type_path: Some("crate::Error".into()),
        generate_object_conversions: true,
        bindings: Some(BindingsConfig {
            aggregate_client_name: "ExampleClient".into(),
            client_crate_name: "example_client".into(),
            py_error_type: "PyExampleError".into(),
            py_result_type: "PyExampleResult".into(),
            napi_error_ext_trait: "NapiErrorExt".into(),
            typings_package_filter: Some(".example.".into()),
            ts_error_base_class: "ExampleError".into(),
            ts_error_code_prefix: "EX".into(),
        }),
        models_gen_dir: None,
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// Generate the full output tree into a temp dir and compare against the committed golden tree.
///
/// Set `BLESS=1` to overwrite the golden tree from current output instead of asserting.
#[test]
fn full_output_matches_golden() {
    let tmp = TempDir::new().unwrap();
    let config = full_config(tmp.path());
    generate_code(&metadata(), &config).expect("generation succeeds");

    let golden = golden_dir();

    if std::env::var_os("BLESS").is_some() {
        // Rewrite the golden tree from scratch so deleted outputs don't linger.
        if golden.exists() {
            std::fs::remove_dir_all(&golden).expect("clear golden");
        }
        for rel in relative_files(tmp.path()) {
            let src = tmp.path().join(&rel);
            let dst = golden.join(&rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).expect("create golden dir");
            }
            std::fs::copy(&src, &dst).expect("copy to golden");
        }
        eprintln!(
            "BLESS: rewrote golden tree at {} from generated output",
            golden.display()
        );
        return;
    }

    assert!(
        golden.exists(),
        "golden tree {} does not exist — run `BLESS=1 cargo test -p olai-codegen --test golden_integration` to create it",
        golden.display()
    );

    let generated: BTreeSet<PathBuf> = relative_files(tmp.path()).into_iter().collect();
    let expected: BTreeSet<PathBuf> = relative_files(&golden).into_iter().collect();

    let missing: Vec<_> = expected.difference(&generated).collect();
    let extra: Vec<_> = generated.difference(&expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "generated file set differs from golden.\n  missing (in golden, not generated): {missing:?}\n  extra (generated, not in golden): {extra:?}\n\
         If this change is intended, re-bless with `BLESS=1 cargo test -p olai-codegen --test golden_integration`."
    );

    let mut diffs = Vec::new();
    for rel in &expected {
        let got = std::fs::read_to_string(tmp.path().join(rel)).expect("read generated");
        let want = std::fs::read_to_string(golden.join(rel)).expect("read golden");
        if got != want {
            diffs.push(rel.display().to_string());
        }
    }
    assert!(
        diffs.is_empty(),
        "generated output differs from golden in {} file(s):\n{}\n\
         If this change is intended, re-bless with `BLESS=1 cargo test -p olai-codegen --test golden_integration`.",
        diffs.len(),
        diffs.join("\n")
    );
}
