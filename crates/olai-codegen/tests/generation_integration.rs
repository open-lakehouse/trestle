//! End-to-end generation-path tests driven by the committed `example.bin`
//! `FileDescriptorSet`.
//!
//! These exercise generators that previously had no integration coverage:
//! - HTTP client method generation (URL building, query params, error handling)
//! - Handler trait generation
//! - Resource-enum + conversion generation (`Resource`/`ObjectLabel`, `TryFrom`,
//!   `RESOURCE_DESCRIPTORS`)
//! - Node TypeScript bindings

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

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                files.extend(walk(&p));
            } else {
                files.push(p);
            }
        }
    }
    files
}

fn read_all(dir: &Path) -> String {
    walk(dir)
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build a config that drives the Rust server/client/models generators.
fn rust_config(tmp: &Path) -> CodeGenConfig {
    let common = tmp.join("common");
    let models = tmp.join("models");
    let server = tmp.join("server");
    let client = tmp.join("client");
    for d in [&common, &models, &server, &client] {
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
            python: None,
            node: None,
            node_ts: None,
            python_typings_filename: "client.pyi".into(),
            generate_resource_clients: false,
        },
        generate_resource_enum: true,
        generate_store_integration: true,
        error_type_path: Some("crate::Error".into()),
        generate_object_conversions: false,
        bindings: None,
        models_gen_dir: None,
    }
}

// ── Generated code is syntactically valid Rust ───────────────────────────────

/// Every emitted `.rs` file must parse as a Rust source file.
///
/// The substring assertions elsewhere in this suite can't catch structural defects like
/// duplicate `pub mod` blocks (E0428) or stray tokens — only a real parse can. This is the
/// cheap guard that keeps the generator's core invariant ("the output is valid Rust") honest
/// without standing up a full `cargo check` fixture crate.
#[test]
fn all_generated_rust_files_parse() {
    let tmp = TempDir::new().unwrap();
    let mut config = rust_config(tmp.path());
    config.generate_object_conversions = true;
    generate_code(&metadata(), &config).expect("generation succeeds");

    let rust_files: Vec<PathBuf> = ["common", "models", "server", "client"]
        .iter()
        .flat_map(|d| walk(&tmp.path().join(d)))
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();

    assert!(
        !rust_files.is_empty(),
        "expected generated .rs files, found none"
    );

    for path in &rust_files {
        let src = std::fs::read_to_string(path).expect("read generated file");
        if let Err(err) = syn::parse_file(&src) {
            panic!(
                "generated file {} is not valid Rust: {err}\n---\n{src}",
                path.display()
            );
        }
    }
}

// ── HTTP client method generation ───────────────────────────────────────────

#[test]
fn client_methods_build_urls_query_and_error_handling() {
    let tmp = TempDir::new().unwrap();
    let config = rust_config(tmp.path());
    generate_code(&metadata(), &config).expect("generation succeeds");

    let client = read_all(&tmp.path().join("client"));

    // GET-by-name builds a templated path from the request field.
    assert!(
        client.contains("format!(\"catalogs/{}\", request.name)"),
        "GET path templating missing:\n{client}"
    );
    // Collection POST hits the bare collection URL.
    assert!(
        client.contains("self.base_url.join(\"catalogs\")"),
        "collection URL join missing"
    );
    // List method appends query params.
    assert!(
        client.contains("append_pair(\"max_results\"")
            && client.contains("append_pair(\"page_token\""),
        "list query-param appends missing"
    );
    // Non-success responses are converted to errors, not unwrapped.
    assert!(
        client.contains("if !response.status().is_success()")
            && client.contains("parse_error_response(response)"),
        "client error-handling missing"
    );
    // HTTP verbs are dispatched correctly per RPC.
    assert!(client.contains(".post(url)"), "POST verb missing");
    assert!(client.contains(".patch(url)"), "PATCH verb missing");
    assert!(client.contains(".delete(url)"), "DELETE verb missing");
}

// ── Handler trait generation ─────────────────────────────────────────────────

#[test]
fn handler_trait_has_async_methods_per_rpc() {
    let tmp = TempDir::new().unwrap();
    let config = rust_config(tmp.path());
    generate_code(&metadata(), &config).expect("generation succeeds");

    let server = read_all(&tmp.path().join("server"));

    assert!(
        server.contains("pub trait CatalogHandler"),
        "handler trait missing:\n{server}"
    );
    assert!(
        server.contains("#[async_trait]"),
        "async_trait attr missing"
    );
    // One async fn per RPC, taking the request + context and returning the result type.
    for method in [
        "async fn create_catalog",
        "async fn get_catalog",
        "async fn list_catalogs",
        "async fn update_catalog",
        "async fn delete_catalog",
    ] {
        assert!(server.contains(method), "handler missing `{method}`");
    }
    assert!(
        server.contains("context: Cx") && server.contains("-> Result<Catalog>"),
        "handler method signature shape unexpected"
    );
}

/// Query-param extractors must only mark optional/repeated fields `#[serde(default)]`.
///
/// Required query params (proto3 fields not marked `optional`) must have NO default, so omitting
/// one is a deserialization error rather than silently defaulting — regression guard for the
/// extractor `serde(default)` policy.
#[test]
fn required_query_params_have_no_serde_default() {
    let tmp = TempDir::new().unwrap();
    let config = rust_config(tmp.path());
    generate_code(&metadata(), &config).expect("generation succeeds");

    let common = read_all(&tmp.path().join("common"));

    // `max_results`/`page_token` on ListCatalogs are required scalars → emitted with no default.
    assert!(
        common.contains("max_results: i32,") && common.contains("page_token: String,"),
        "required query params should be plain fields:\n{common}"
    );
    assert!(
        !common.contains("#[serde(default)]\n            max_results")
            && !common.contains("#[serde(default)] max_results"),
        "required query param `max_results` must NOT carry #[serde(default)]:\n{common}"
    );
    // Repeated `tags` (Vec) still gets a default so an absent key → empty Vec.
    assert!(
        common.contains("tags: Vec<String>"),
        "repeated query param `tags` missing:\n{common}"
    );
}

// ── Resource-enum + conversions ──────────────────────────────────────────────

#[test]
fn resource_enum_and_conversions_generated() {
    let tmp = TempDir::new().unwrap();
    let config = rust_config(tmp.path());
    generate_code(&metadata(), &config).expect("generation succeeds");

    let labels = read_all(&tmp.path().join("models"));

    // Resource enum variant wraps the fully-qualified model path.
    assert!(
        labels.contains("pub enum Resource")
            && labels.contains("Catalog(super::catalog::v1::Catalog)"),
        "Resource enum variant missing:\n{labels}"
    );
    // ObjectLabel discriminant enum.
    assert!(
        labels.contains("pub enum ObjectLabel") && labels.contains("Catalog,"),
        "ObjectLabel enum missing"
    );
    // From + TryFrom<Resource> conversions (gated on error_type_path).
    assert!(
        labels.contains("impl From<super::catalog::v1::Catalog> for Resource"),
        "From<Catalog> for Resource missing"
    );
    assert!(
        labels.contains("impl TryFrom<Resource> for super::catalog::v1::Catalog"),
        "TryFrom<Resource> missing"
    );
    // resource_label() discriminant accessor.
    assert!(
        labels.contains("fn resource_label(&self) -> &ObjectLabel"),
        "resource_label accessor missing"
    );
}

#[test]
fn resource_registry_descriptors_generated() {
    let tmp = TempDir::new().unwrap();
    let config = rust_config(tmp.path());
    generate_code(&metadata(), &config).expect("generation succeeds");

    let labels = read_all(&tmp.path().join("models"));

    assert!(
        labels.contains("impl ::olai_store::Label for ObjectLabel"),
        "Label impl missing:\n{labels}"
    );
    assert!(
        labels.contains("pub static RESOURCE_DESCRIPTORS"),
        "RESOURCE_DESCRIPTORS static missing"
    );
    assert!(
        labels.contains("ObjectLabel::Catalog") && labels.contains("FieldRole::"),
        "descriptor entry contents missing"
    );
    // A flat resource's name decomposes to just `["name"]` — standard List fields like
    // `page_token` must not leak into the path components (regression guard for the
    // proto3-presence heuristic).
    assert!(
        labels.contains("path_names: &[\"name\"]"),
        "Catalog path_names should be [\"name\"], not include pagination fields:\n{labels}"
    );
    assert!(
        !labels.contains("page_token\", \"name"),
        "pagination field leaked into path_names:\n{labels}"
    );
}

#[test]
fn resource_enum_skipped_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let mut config = rust_config(tmp.path());
    config.generate_resource_enum = false;
    config.generate_store_integration = false;
    config.error_type_path = None;
    generate_code(&metadata(), &config).expect("generation succeeds");

    // labels.rs should not be written when the resource enum is disabled.
    let labels_path = tmp.path().join("models").join("_gen").join("labels.rs");
    assert!(
        !labels_path.exists(),
        "labels.rs should not be generated when generate_resource_enum is false"
    );
}

// ── Node TypeScript bindings ──────────────────────────────────────────────────

#[test]
fn node_ts_bindings_generated() {
    let tmp = TempDir::new().unwrap();
    let common = tmp.path().join("common");
    let node_ts = tmp.path().join("node_ts");
    for d in [&common, &node_ts] {
        std::fs::create_dir_all(d).expect("create dir");
    }
    let config = CodeGenConfig {
        context_type_path: "crate::Context".into(),
        result_type_path: "crate::Result".into(),
        models_path_template: "example_common::models::{service}::v1".into(),
        models_path_crate_template: "crate::models::{service}::v1".into(),
        resource_store_crate_name: "olai_store".into(),
        output: CodeGenOutput {
            common,
            models: None,
            models_subdir: "_gen".into(),
            server: None,
            client: None,
            python: None,
            node: None,
            node_ts: Some(node_ts.clone()),
            python_typings_filename: "client.pyi".into(),
            generate_resource_clients: false,
        },
        generate_resource_enum: false,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
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
    };
    generate_code(&metadata(), &config).expect("generation succeeds");

    let ts = std::fs::read_to_string(node_ts.join("client.ts")).expect("client.ts written");
    assert!(
        ts.contains("ExampleClient"),
        "aggregate client name missing"
    );
    assert!(ts.contains("ExampleError"), "error base class missing");
}
