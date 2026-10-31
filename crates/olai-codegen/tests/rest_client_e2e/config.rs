// The generation config of the fixture, `include!`d by `build.rs` and `py/build.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

use olai_codegen::postprocess::{JsonFieldNames, ModelRewrites};
use olai_codegen::{
    BindingsConfig, CodeGenConfig, CodeGenMetadata, CodeGenOutput, ErrorModuleConfig,
    PyPackageConfig, PythonOptions, REQWEST_TRANSPORT_TYPE_PATH, RestClientOptions, Runtime,
};

/// The fixture directory (the one with `proto/`).
fn fixture_dir() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    if manifest.join("proto").exists() {
        manifest
    } else {
        manifest.parent().unwrap().to_path_buf()
    }
}

/// Compile the proto to a descriptor set (with source info for the doc comments).
fn compile_descriptors(out: &Path) -> PathBuf {
    let fixture = fixture_dir();
    let descriptors = out.join("svc.bin");
    let status = Command::new("protoc")
        .args(["--include_imports", "--include_source_info", "-I"])
        .arg(fixture.join("proto"))
        .arg(format!("--descriptor_set_out={}", descriptors.display()))
        .arg(fixture.join("proto/demo/v1/svc.proto"))
        .status()
        .expect("protoc on PATH");
    assert!(status.success(), "protoc failed");
    println!("cargo:rerun-if-changed={}", fixture.join("proto").display());
    println!(
        "cargo:rerun-if-changed={}",
        fixture.join("../../src").display()
    );
    descriptors
}

/// The generation config, with every option on. Everything goes under `out`.
fn demo_config(out: &Path) -> CodeGenConfig {
    let models_dir = out.join("models");
    let gen_dir = models_dir.join("_gen");
    let package_dir = out.join("pkg").join("demo");
    for dir in [
        &gen_dir,
        &out.join("client"),
        &out.join("python"),
        &package_dir,
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    CodeGenConfig {
        context_type_path: "crate::Context".into(),
        // Unused: the clients return the generated error module's `Result`.
        result_type_path: "crate::Result".into(),
        models_path_template: "rest_client_e2e::models::{service}::{version}".into(),
        models_path_crate_template: "crate::models::{service}::{version}".into(),
        resource_store_crate_name: "olai_store".into(),
        runtime: Runtime::Buffa,
        transport_type_path: REQWEST_TRANSPORT_TYPE_PATH.into(),
        dual_transport: false,
        client_protocols: olai_codegen::ClientProtocols::default(),
        connect_client_path: None,
        rest_client: RestClientOptions {
            json_int64_as_number: true,
            ignore_unknown_enum_values: true,
            error_module: Some(ErrorModuleConfig {
                code_enum: Some(".demo.v1.ErrorCode".into()),
                // `{"error": {"message", "type"}}` and OAuth's `{"error", "error_description"}`.
                more_code_fields: vec!["type".into(), "error".into()],
                more_message_fields: vec!["error_description".into()],
                envelope_fields: vec!["error".into()],
                ..Default::default()
            }),
            form_encoded_methods: vec!["demo.v1.TokenService.ExchangeToken".into()],
            path_fields_in_body_methods: vec!["demo.v1.ProbeService.RenameWidget".into()],
            json_query_packages: vec![],
            user_agent: Some("demo-client".into()),
        },
        output: CodeGenOutput {
            common: Some(gen_dir.clone()),
            models: Some(models_dir),
            models_subdir: "_gen".into(),
            server: None,
            client: Some(out.join("client")),
            generate_resource_clients: true,
            python: Some(out.join("python")),
            node: None,
            node_ts: None,
            wasm: None,
        },
        generate_resource_enum: false,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
        bindings: Some(BindingsConfig {
            aggregate_client_name: "DemoClient".into(),
            client_crate_name: "rest_client_e2e".into(),
            py_error_type: "PyDemoError".into(),
            py_result_type: "PyDemoResult".into(),
            napi_error_ext_trait: "NapiErrorExt".into(),
            ts_error_base_class: "DemoError".into(),
            ts_error_code_prefix: "DEMO".into(),
            python: Some(PythonOptions {
                package: PyPackageConfig {
                    dir: package_dir,
                    name: "demo".into(),
                },
                error_base_class: "DemoError".into(),
                module: "demo._core".into(),
                async_methods: true,
                keyword_only: true,
                public_modules: false,
                user_agent: Some("demo-python".into()),
            }),
        }),
    }
}

/// The model rewrites the `trestle` CLI would apply.
#[allow(dead_code)]
fn demo_rewrites(config: &CodeGenConfig, metadata: &CodeGenMetadata) -> ModelRewrites {
    ModelRewrites {
        field_names: JsonFieldNames::Proto,
        packages: Default::default(),
        int64_module: config.json_int64_module_path(),
        json_value_module: config.json_value_module_path(),
        fields: olai_codegen::postprocess::field_options(metadata),
    }
}
