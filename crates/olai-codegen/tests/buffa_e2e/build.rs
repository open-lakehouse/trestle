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
        CodeGenConfig, CodeGenOutput, Runtime, generate_code, parse_file_descriptor_set,
    };
    use protobuf::Message;
    use protobuf::descriptor::FileDescriptorSet;

    let bytes = std::fs::read("proto/svc.bin").unwrap();
    let fds = FileDescriptorSet::parse_from_bytes(&bytes).unwrap();
    let metadata = parse_file_descriptor_set(&fds).unwrap();

    let client_dir = out.join("client");
    let common_dir = out.join("common_gen");
    std::fs::create_dir_all(&client_dir).unwrap();
    std::fs::create_dir_all(&common_dir).unwrap();

    let config = CodeGenConfig {
        context_type_path: "crate::Context".into(),
        result_type_path: "crate::Result".into(),
        models_path_template: "crate::models::{service}::v1".into(),
        models_path_crate_template: "crate::models::{service}::v1".into(),
        resource_store_crate_name: "olai_store".into(),
        runtime: Runtime::Buffa,
        output: CodeGenOutput {
            common: common_dir,
            models: None,
            models_subdir: "_gen".into(),
            server: None,
            client: Some(client_dir),
            generate_resource_clients: false,
            python: None,
            node: None,
            node_ts: None,
            python_typings_filename: "client.pyi".into(),
        },
        generate_resource_enum: false,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
        bindings: None,
        models_gen_dir: None,
    };
    generate_code(&metadata, &config).unwrap();

    println!("cargo:rerun-if-changed=proto/svc.bin");
    println!("cargo:rerun-if-changed=proto/exported/svc.proto");
}
