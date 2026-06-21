use std::path::PathBuf;

use olai_codegen::parsing::parse_file_descriptor_set;
use olai_codegen::{BindingsConfig, CodeGenConfig, CodeGenOutput, generate_code};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;
use tempfile::TempDir;

fn load_descriptor() -> FileDescriptorSet {
    let bytes = include_bytes!("../proto/example.bin");
    FileDescriptorSet::parse_from_bytes(bytes).expect("valid descriptor")
}

fn make_test_config(
    common: PathBuf,
    node_ts: PathBuf,
    python: PathBuf,
    node: PathBuf,
) -> CodeGenConfig {
    CodeGenConfig {
        context_type_path: "crate::Context".to_string(),
        result_type_path: "crate::Result".to_string(),
        models_path_template: "example_common::models::{service}::v1".to_string(),
        models_path_crate_template: "crate::models::{service}::v1".to_string(),
        resource_store_crate_name: "olai_store".to_string(),
        runtime: olai_codegen::Runtime::Prost,
        transport_type_path: olai_codegen::DEFAULT_TRANSPORT_TYPE_PATH.to_string(),
        output: CodeGenOutput {
            common,
            models: None,
            models_subdir: "_gen".to_string(),
            server: None,
            client: None,
            python: Some(python),
            node: Some(node),
            node_ts: Some(node_ts),
            wasm: None,
            python_typings_filename: "example_client.pyi".to_string(),
            generate_resource_clients: false,
        },
        generate_resource_enum: false,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
        bindings: Some(BindingsConfig {
            aggregate_client_name: "ExampleClient".to_string(),
            client_crate_name: "example_client".to_string(),
            py_error_type: "PyExampleError".to_string(),
            py_result_type: "PyExampleResult".to_string(),
            napi_error_ext_trait: "NapiErrorExt".to_string(),
            typings_package_filter: Some(".example.".to_string()),
            ts_error_base_class: "ExampleError".to_string(),
            ts_error_code_prefix: "EX".to_string(),
        }),
        models_gen_dir: None,
    }
}

fn collect_generated_files(dir: &std::path::Path) -> Vec<String> {
    let mut contents = Vec::new();
    for entry in walkdir(dir) {
        if entry.is_file() {
            if let Ok(text) = std::fs::read_to_string(&entry) {
                contents.push(text);
            }
        }
    }
    contents
}

fn walkdir(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn test_codegen_produces_no_unitycatalog_strings() {
    let descriptor = load_descriptor();
    let metadata = parse_file_descriptor_set(&descriptor).expect("parse succeeded");

    let tmp = TempDir::new().expect("tempdir");
    let common_dir = tmp.path().join("common");
    let node_ts_dir = tmp.path().join("node_ts");
    let python_dir = tmp.path().join("python");
    let node_dir = tmp.path().join("node");

    for dir in &[&common_dir, &node_ts_dir, &python_dir, &node_dir] {
        std::fs::create_dir_all(dir).expect("create dir");
    }

    let config = make_test_config(
        common_dir.clone(),
        node_ts_dir.clone(),
        python_dir.clone(),
        node_dir.clone(),
    );

    generate_code(&metadata, &config).expect("generate_code succeeded");

    let all_dirs = [&common_dir, &node_ts_dir, &python_dir, &node_dir];
    let all_files: Vec<String> = all_dirs
        .iter()
        .flat_map(|d| collect_generated_files(d))
        .collect();

    assert!(
        !all_files.is_empty(),
        "expected generated files to be written"
    );

    for content in &all_files {
        let lower = content.to_lowercase();
        assert!(
            !lower.contains("unitycatalog"),
            "generated output contains 'unitycatalog': {:.200}",
            content
        );
        assert!(
            !lower.contains("pyunitycatalog"),
            "generated output contains 'pyunitycatalog'"
        );
        assert!(
            !lower.contains("napiunitycatalog"),
            "generated output contains 'napiunitycatalog'"
        );
    }
}

#[test]
fn test_codegen_uses_configured_error_base_class() {
    let descriptor = load_descriptor();
    let metadata = parse_file_descriptor_set(&descriptor).expect("parse succeeded");

    let tmp = TempDir::new().expect("tempdir");
    let common_dir = tmp.path().join("common");
    let node_ts_dir = tmp.path().join("node_ts");
    let python_dir = tmp.path().join("python");
    let node_dir = tmp.path().join("node");

    for dir in &[&common_dir, &node_ts_dir, &python_dir, &node_dir] {
        std::fs::create_dir_all(dir).expect("create dir");
    }

    let config = make_test_config(common_dir, node_ts_dir.clone(), python_dir, node_dir);

    generate_code(&metadata, &config).expect("generate_code succeeded");

    let ts_files = collect_generated_files(&node_ts_dir);
    assert!(!ts_files.is_empty(), "expected TypeScript files");

    let client_ts = ts_files
        .iter()
        .find(|f| f.contains("ExampleError"))
        .expect("expected ExampleError in TypeScript output");

    assert!(
        client_ts.contains("ExampleError"),
        "TypeScript output should use configured error base class"
    );
    assert!(
        client_ts.contains("EX:"),
        "TypeScript output should use configured error code prefix in regex"
    );
}

#[test]
fn test_codegen_uses_configured_aggregate_client_name() {
    let descriptor = load_descriptor();
    let metadata = parse_file_descriptor_set(&descriptor).expect("parse succeeded");

    let tmp = TempDir::new().expect("tempdir");
    let common_dir = tmp.path().join("common");
    let node_ts_dir = tmp.path().join("node_ts");
    let python_dir = tmp.path().join("python");
    let node_dir = tmp.path().join("node");

    for dir in &[&common_dir, &node_ts_dir, &python_dir, &node_dir] {
        std::fs::create_dir_all(dir).expect("create dir");
    }

    let config = make_test_config(
        common_dir,
        node_ts_dir.clone(),
        python_dir.clone(),
        node_dir,
    );

    generate_code(&metadata, &config).expect("generate_code succeeded");

    let ts_files = collect_generated_files(&node_ts_dir);
    let all_ts = ts_files.join("\n");
    assert!(
        all_ts.contains("ExampleClient"),
        "TypeScript output should use configured aggregate client name"
    );

    let py_files = collect_generated_files(&python_dir);
    let all_py = py_files.join("\n");
    assert!(
        all_py.contains("ExampleClient"),
        "Python output should use configured aggregate client name"
    );
}

/// Regression test for the Python emitter silently dropping
/// `RequestType::Custom(Pattern::Post | Pattern::Patch)` RPCs.
///
/// `GenerateCatalogToken` is a factory-style POST RPC without path
/// parameters — `is_collection_method` classifies it as a collection
/// method (alongside `List` and `Create`), and the emitter should emit
/// a `generate_catalog_token(...)` method on the aggregate client.
#[test]
fn test_python_emitter_handles_custom_post_rpcs() {
    let descriptor = load_descriptor();
    let metadata = parse_file_descriptor_set(&descriptor).expect("parse succeeded");

    let tmp = TempDir::new().expect("tempdir");
    let common_dir = tmp.path().join("common");
    let node_ts_dir = tmp.path().join("node_ts");
    let python_dir = tmp.path().join("python");
    let node_dir = tmp.path().join("node");

    for dir in &[&common_dir, &node_ts_dir, &python_dir, &node_dir] {
        std::fs::create_dir_all(dir).expect("create dir");
    }

    let config = make_test_config(common_dir, node_ts_dir, python_dir.clone(), node_dir);

    generate_code(&metadata, &config).expect("generate_code succeeded");

    let py_files = collect_generated_files(&python_dir);
    let all_py = py_files.join("\n");

    assert!(
        all_py.contains("generate_catalog_token"),
        "Python emitter dropped a `Custom(Post)` RPC; output should expose \
         `generate_catalog_token`"
    );

    // The `.pyi` typings emitter must also include `Custom(Post|Patch)`
    // RPCs — the runtime bindings and the type stubs must stay in sync.
    let pyi = py_files
        .iter()
        .find(|f| f.contains("class ExampleClient"))
        .expect("expected ExampleClient stub in generated Python output");
    assert!(
        pyi.contains("generate_catalog_token"),
        "Python `.pyi` emitter dropped a `Custom(Post)` RPC; expected \
         `generate_catalog_token` in the stub"
    );
}

/// Resource-less, composite-key services (e.g. tag assignments) must produce usable bindings:
/// every method lives on the root client and accepts ALL path params, and `Empty`-returning custom
/// RPCs lower to a unit/void result. This is the "flat binding lowering" regression guard.
#[test]
fn test_resource_less_service_emits_flat_bindings_with_path_params() {
    let descriptor = load_descriptor();
    let metadata = parse_file_descriptor_set(&descriptor).expect("parse succeeded");
    let tmp = TempDir::new().expect("tempdir");
    let common_dir = tmp.path().join("common");
    let node_ts_dir = tmp.path().join("node_ts");
    let python_dir = tmp.path().join("python");
    let node_dir = tmp.path().join("node");
    for dir in &[&common_dir, &node_ts_dir, &python_dir, &node_dir] {
        std::fs::create_dir_all(dir).expect("create dir");
    }
    let config = make_test_config(
        common_dir.clone(),
        node_ts_dir.clone(),
        python_dir.clone(),
        node_dir.clone(),
    );
    generate_code(&metadata, &config).expect("generate_code succeeded");

    // The `.pyi` stub must mirror the flat bindings: resource-less methods land on the aggregate
    // `ExampleClient` with all path params, and there is no phantom scoped class.
    let pyi = std::fs::read_to_string(python_dir.join("example_client.pyi")).expect("pyi");
    assert!(
        pyi.contains(
            "def fetch_tag_assignment(\n\
             \n            self,\n\
             \x20           entity_type: str,\n\
             \x20           entity_name: str,\n\
             \x20           tag_key: str\n\
             \x20       ) -> TagAssignment:"
        ),
        "pyi: fetch_tag_assignment must expose all composite path params -> TagAssignment:\n{pyi}"
    );
    assert!(
        pyi.contains(
            "def touch_tag_assignment(\n\
             \n            self,\n\
             \x20           entity_type: str,\n\
             \x20           entity_name: str,\n\
             \x20           tag_key: str\n\
             \x20       ) -> None:"
        ),
        "pyi: touch_tag_assignment must take all path params and return None:\n{pyi}"
    );
    assert!(
        !pyi.contains("class TagAssignmentsServiceClient")
            && !pyi.contains("class TagAssignmentsClient")
            && !pyi.contains("class TagsClient"),
        "pyi: resource-less service must not get a phantom scoped class:\n{pyi}"
    );

    let py = std::fs::read_to_string(python_dir.join("mod.rs")).expect("python mod.rs");
    // Composite-key getter exposes ALL path params on the root client (not a scoped client).
    // The Get RPC carries a gnostic `operation_id`, so the binding method is named
    // `fetch_tag_assignment` (annotation-driven naming), not `get_tag_assignment`.
    assert!(
        py.contains("pub fn fetch_tag_assignment(")
            && py.contains("entity_type: String")
            && py.contains("entity_name: String")
            && py.contains("tag_key: String"),
        "python: fetch_tag_assignment must take all composite path params:\n{py:.4000}"
    );
    // Empty-returning custom RPC lowers to `<()>` (regression for the missing type arg).
    assert!(
        py.contains("pub fn touch_tag_assignment(") && py.contains("PyExampleResult<()>"),
        "python: touch_tag_assignment must return PyExampleResult<()>:\n{py:.4000}"
    );
    // Resource-less service has no scoped per-service module emitted.
    assert!(
        !python_dir.join("tags.rs").exists(),
        "python: resource-less service must not get a scoped module"
    );

    let ts = std::fs::read_to_string(node_ts_dir.join("client.ts")).expect("ts client.ts");
    assert!(
        ts.contains(
            "async fetchTagAssignment(entityType: string, entityName: string, tagKey: string)"
        ),
        "ts: fetchTagAssignment must take all composite path params:\n{ts:.4000}"
    );
    assert!(
        ts.contains("async touchTagAssignment(") && ts.contains("tagKey: string): Promise<void>"),
        "ts: touchTagAssignment must return Promise<void>:\n{ts:.4000}"
    );
}

/// The generated Rust aggregate root client (`client.rs`) must expose flat builder constructors for
/// resource-less services, matching the surface the language bindings call. Regression guard for the
/// aggregate generator.
#[test]
fn test_aggregate_client_emits_flat_builders_for_resource_less_service() {
    let descriptor = load_descriptor();
    let metadata = parse_file_descriptor_set(&descriptor).expect("parse succeeded");
    let tmp = TempDir::new().expect("tempdir");
    let common_dir = tmp.path().join("common");
    let node_ts_dir = tmp.path().join("node_ts");
    let python_dir = tmp.path().join("python");
    let node_dir = tmp.path().join("node");
    let client_dir = tmp.path().join("client");
    for dir in &[
        &common_dir,
        &node_ts_dir,
        &python_dir,
        &node_dir,
        &client_dir,
    ] {
        std::fs::create_dir_all(dir).expect("create dir");
    }
    let mut config = make_test_config(
        common_dir.clone(),
        node_ts_dir.clone(),
        python_dir.clone(),
        node_dir.clone(),
    );
    config.output.client = Some(client_dir.clone());
    generate_code(&metadata, &config).expect("generate_code succeeded");

    let client = std::fs::read_to_string(client_dir.join("client.rs")).expect("client.rs");

    // The aggregate struct uses the configured name.
    assert!(
        client.contains("pub struct ExampleClient"),
        "client.rs must define the configured aggregate struct:\n{client:.4000}"
    );
    // Resource-less list method takes the composite path params (entity_type, entity_name) and
    // returns the builder.
    assert!(
        client.contains("pub fn list_tag_assignments(")
            && client.contains("entity_type: impl Into<String>")
            && client.contains("entity_name: impl Into<String>")
            && client.contains("-> ListTagAssignmentsBuilder"),
        "client.rs: list_tag_assignments must take composite path params and return its builder:\n{client:.4000}"
    );
    // Custom POST RPC (Empty return) emits a flat builder constructor under its operation name.
    assert!(
        client.contains("pub fn touch_tag_assignment(")
            && client.contains("tag_key: impl Into<String>")
            && client.contains("-> TouchTagAssignmentBuilder"),
        "client.rs: touch_tag_assignment must take all path params and return its builder:\n{client:.4000}"
    );
    // The top-level client module must re-export the aggregate.
    let client_mod = std::fs::read_to_string(client_dir.join("mod.rs")).expect("client mod.rs");
    assert!(
        client_mod.contains("pub mod client") && client_mod.contains("pub use client::*"),
        "client mod.rs must export the aggregate module:\n{client_mod}"
    );
}
