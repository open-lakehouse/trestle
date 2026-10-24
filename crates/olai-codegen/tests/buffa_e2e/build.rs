//! Two-stage generation:
//!   1. `buffa-build` generates the model types (the runtime the generated code
//!      *consumes*) from the vendored protos.
//!   2. `olai-codegen` runs with `Runtime::Buffa` to emit the trestle client.
//!
//! `src/lib.rs` then `include!`s both and the test round-trips JSON through them.

use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::create_dir_all(out.join("models")).unwrap();

    // 1. buffa model types (with native serde JSON — no pbjson).
    buffa_build::Config::new()
        .files(&["proto/exported/svc.proto"])
        .includes(&["proto/exported"])
        .generate_json(true)
        .generate_views(false)
        .out_dir(out.join("models"))
        .include_file("_include.rs")
        .compile()
        .unwrap();

    // 2. trestle client codegen, buffa runtime.
    use olai_codegen::{
        BindingsConfig, CodeGenConfig, CodeGenOutput, Runtime, generate_code,
        parse_file_descriptor_set,
    };
    use protobuf::Message;
    use protobuf::descriptor::FileDescriptorSet;

    let bytes = std::fs::read("proto/svc.bin").unwrap();
    let fds = FileDescriptorSet::parse_from_bytes(&bytes).unwrap();
    let metadata = parse_file_descriptor_set(&fds).unwrap();

    let client_dir = out.join("client");
    let common_dir = out.join("common_gen");
    // The PyO3 boundary conversions (`pyo3_impls.rs`) are emitted into the models
    // output dir, alongside the (otherwise empty) generated `mod.rs`/`labels.rs`.
    // We only `include!` `pyo3_impls.rs` (under the `python` feature), so the
    // models dir doubles as where that file lands.
    let models_dir = out.join("models_gen");
    let python_dir = out.join("python");
    std::fs::create_dir_all(&client_dir).unwrap();
    std::fs::create_dir_all(&common_dir).unwrap();
    std::fs::create_dir_all(&models_dir).unwrap();
    std::fs::create_dir_all(&python_dir).unwrap();

    let config = CodeGenConfig {
        context_type_path: "crate::Context".into(),
        result_type_path: "crate::Result".into(),
        models_path_template: "crate::models::{service}::v1".into(),
        models_path_crate_template: "crate::models::{service}::v1".into(),
        resource_store_crate_name: "olai_store".into(),
        runtime: Runtime::Buffa,
        transport_type_path: olai_codegen::DEFAULT_TRANSPORT_TYPE_PATH.into(),
        client_protocols: olai_codegen::ClientProtocols::default(),
        dual_transport: false,
        connect_client_path: None,
        output: CodeGenOutput {
            common: common_dir,
            models: Some(models_dir),
            models_subdir: "_gen".into(),
            server: None,
            client: Some(client_dir),
            generate_resource_clients: true,
            // Enabling Python output makes the models block emit `pyo3_impls.rs`
            // (FromPyObject/IntoPyObject for the buffa model types) — the thing
            // this fixture proves compiles & round-trips against real buffa types.
            python: Some(python_dir.clone()),
            node: None,
            node_ts: None,
            wasm: None,
            python_typings_filename: "client.pyi".into(),
        },
        generate_resource_enum: false,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
        // Python output requires a bindings config. This crate only include!s
        // `pyo3_impls.rs` from the models dir; the PyO3 client wrappers (which
        // name this crate as `buffa_e2e`) are compiled by the `py/` member crate.
        bindings: Some(BindingsConfig {
            aggregate_client_name: "DemoClient".into(),
            client_crate_name: "buffa_e2e".into(),
            py_error_type: "PyDemoError".into(),
            py_result_type: "PyDemoResult".into(),
            // `BUFFA_E2E_PYTHON_API=sync|async` builds a single flavor (compile-check it with
            // `cargo build -p buffa-e2e-py`; the `py` tests drive both and expect the default).
            python_api: match std::env::var("BUFFA_E2E_PYTHON_API").as_deref() {
                Ok("sync") => olai_codegen::PythonApi::Sync,
                Ok("async") => olai_codegen::PythonApi::Async,
                _ => olai_codegen::PythonApi::Both,
            },
            napi_error_ext_trait: "NapiErrorExt".into(),
            typings_package_filter: Some(".demo.".into()),
            ts_error_base_class: "DemoError".into(),
            ts_error_code_prefix: "DEMO".into(),
        }),
    };
    generate_code(&metadata, &config).unwrap();

    // Hand the generated PyO3 bindings to `py/build.rs` (`DEP_BUFFA_E2E_PYTHON_DIR`).
    println!("cargo:python_dir={}", python_dir.display());

    println!("cargo:rerun-if-changed=proto/svc.bin");
    println!("cargo:rerun-if-env-changed=BUFFA_E2E_PYTHON_API");
    println!("cargo:rerun-if-changed=proto/exported/svc.proto");
}
