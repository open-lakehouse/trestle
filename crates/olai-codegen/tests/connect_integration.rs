//! Generation-path tests for ConnectRPC clients (`ClientProtocol::Connect`).
//!
//! Drives `generate_code` over a synthetic, **annotation-light** service (no `google.api.http`) to
//! prove the Connect path: methods are kept without HTTP annotations, the low-level `client.rs`
//! adapts the connect-rust generated client, and the shared builder layer partitions
//! required-vs-optional from `field_behavior`. All emitted Rust must parse.

use std::collections::HashMap;
use std::path::Path;

use olai_codegen::parsing::types::{BaseType, UnifiedType};
use olai_codegen::parsing::{
    CodeGenMetadata, HttpPattern, MessageField, MessageInfo, MethodMetadata, ServiceInfo,
};
use olai_codegen::{ClientProtocols, CodeGenConfig, CodeGenOutput, generate_code};
use tempfile::TempDir;

/// A string field. `required` marks it `google.api.field_behavior = REQUIRED` (→ a constructor
/// arg); otherwise it is a proto3 `optional` scalar (→ a `with_*` setter), matching how a
/// non-required scalar would actually appear.
fn string_field(name: &str, required: bool) -> MessageField {
    MessageField {
        name: name.to_string(),
        unified_type: UnifiedType {
            base_type: BaseType::String,
            is_optional: !required,
            is_repeated: false,
        },
        documentation: None,
        oneof_variants: None,
        field_behavior: if required {
            vec![olai_codegen::FieldBehavior::Required]
        } else {
            vec![]
        },
        is_sensitive: false,
        resource_reference: None,
    }
}

/// A `ReadService` with one method `GetThing(GetThingRequest) -> Thing`, where the request has a
/// REQUIRED `name` and an optional `filter`, and **no HTTP annotation**.
fn connect_metadata() -> CodeGenMetadata {
    let request = MessageInfo {
        name: "GetThingRequest".to_string(),
        fields: vec![string_field("name", true), string_field("filter", false)],
        resource_descriptor: None,
        documentation: None,
    };
    let response = MessageInfo {
        name: "Thing".to_string(),
        fields: vec![],
        resource_descriptor: None,
        documentation: None,
    };

    let mut messages = HashMap::new();
    messages.insert("GetThingRequest".to_string(), request);
    messages.insert("Thing".to_string(), response);

    let method = MethodMetadata {
        service_name: "ReadService".to_string(),
        method_name: "GetThing".to_string(),
        input_type: "GetThingRequest".to_string(),
        output_type: "Thing".to_string(),
        operation: None,
        http_rule: Default::default(),
        http_pattern: HttpPattern::default(),
        documentation: Some("Fetch one thing.".to_string()),
    };
    let service = ServiceInfo {
        name: "ReadService".to_string(),
        package: "example.read.v1".to_string(),
        documentation: None,
        methods: vec![method],
    };

    let mut services = HashMap::new();
    services.insert("ReadService".to_string(), service);

    CodeGenMetadata {
        messages,
        services,
        enums: HashMap::new(),
    }
}

fn connect_config(client_dir: &Path) -> CodeGenConfig {
    CodeGenConfig {
        context_type_path: "crate::Context".into(),
        result_type_path: "crate::Result".into(),
        models_path_template: "myproto::models::{service}::v1".into(),
        models_path_crate_template: "crate::models::{service}::v1".into(),
        resource_store_crate_name: "olai_store".into(),
        runtime: olai_codegen::Runtime::Buffa,
        transport_type_path: olai_codegen::DEFAULT_TRANSPORT_TYPE_PATH.into(),
        dual_transport: false,
        client_protocols: ClientProtocols {
            rest: false,
            connect: true,
        },
        connect_client_path: Some("myproto::connect_gen::example::read::v1".into()),
        output: CodeGenOutput {
            common: client_dir.join("common"),
            models: None,
            models_subdir: "_gen".into(),
            server: None,
            client: Some(client_dir.to_path_buf()),
            python: None,
            node: None,
            node_ts: None,
            wasm: None,
            python_typings_filename: "client.pyi".into(),
            generate_resource_clients: false,
        },
        generate_resource_enum: false,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
        bindings: None,
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn connect_client_is_generated_without_http_annotations() {
    let tmp = TempDir::new().unwrap();
    let client = tmp.path().join("client");
    std::fs::create_dir_all(client.join("common")).unwrap();

    let metadata = connect_metadata();
    let config = connect_config(&client);
    generate_code(&metadata, &config).expect("connect generation succeeds");

    let client_rs = read(&client.join("read/client.rs"));
    let builders_rs = read(&client.join("read/builders.rs"));

    // The low-level client is the Connect adapter, generic over the transport and wrapping the
    // connect-rust generated `ReadServiceClient`.
    assert!(
        client_rs.contains("ReadServiceConnectClient"),
        "missing Connect adapter type:\n{client_rs}"
    );
    assert!(
        client_rs.contains("myproto :: connect_gen :: example :: read :: v1 :: ReadServiceClient")
            || client_rs.contains("ReadServiceClient"),
        "adapter must wrap the connect-rust client:\n{client_rs}"
    );
    assert!(
        client_rs.contains("CloudTransport"),
        "adapter should default to the cloud transport:\n{client_rs}"
    );
    // The adapter delegates to the connect-rust method and maps errors / unwraps the view.
    assert!(client_rs.contains("from_connect_error"), "{client_rs}");
    assert!(client_rs.contains("into_owned"), "{client_rs}");
    assert!(
        client_rs.contains("async fn get_thing"),
        "adapter method should be snake_case:\n{client_rs}"
    );

    // The builder partitions required vs optional from field_behavior: `name` is a constructor arg,
    // `filter` is a `with_*` setter.
    assert!(builders_rs.contains("GetThingBuilder"), "{builders_rs}");
    assert!(
        builders_rs.contains("name") && builders_rs.contains("fn with_filter"),
        "required `name` as ctor arg + optional `filter` as with_*:\n{builders_rs}"
    );
    // `filter` must NOT have become a constructor-only required arg.
    assert!(
        !builders_rs.contains("fn with_name"),
        "required field must not get a with_* setter:\n{builders_rs}"
    );

    // Everything emitted must be syntactically valid Rust.
    for f in [
        "read/client.rs",
        "read/builders.rs",
        "read/mod.rs",
        "mod.rs",
    ] {
        let src = read(&client.join(f));
        syn::parse_file(&src).unwrap_or_else(|e| panic!("{f} does not parse: {e}\n{src}"));
    }
}

#[test]
fn both_protocols_emit_side_by_side_under_subdirs() {
    let tmp = TempDir::new().unwrap();
    let client = tmp.path().join("client");
    std::fs::create_dir_all(client.join("common")).unwrap();

    let metadata = connect_metadata();
    let mut config = connect_config(&client);
    config.client_protocols = ClientProtocols {
        rest: true,
        connect: true,
    };
    // Give the method an HTTP route so the REST client has something to emit. (The Connect client
    // emits it regardless.) We re-tag the request message's method by re-parsing isn't needed —
    // instead drive a routed service through the same generator below.
    // For this layout test it is enough that BOTH protocol dirs are produced for the service.
    generate_code(&metadata, &config).expect("dual generation succeeds");

    // Connect always lands under `read/connect/`.
    let connect_client = read(&client.join("read/connect/client.rs"));
    assert!(
        connect_client.contains("ReadServiceConnectClient"),
        "{connect_client}"
    );

    // REST lands under `read/rest/`. The single method here is routeless, so the REST client body
    // has no methods — but the files and module wiring must still be emitted and parse.
    for f in [
        "read/connect/client.rs",
        "read/connect/builders.rs",
        "read/connect/mod.rs",
        "read/rest/client.rs",
        "read/rest/mod.rs",
        "read/mod.rs",
        "mod.rs",
    ] {
        let path = client.join(f);
        assert!(path.exists(), "expected generated file {f}");
        let src = read(&path);
        syn::parse_file(&src).unwrap_or_else(|e| panic!("{f} does not parse: {e}\n{src}"));
    }

    // The service `mod.rs` namespaces the two protocols rather than glob-merging them.
    let service_mod = read(&client.join("read/mod.rs"));
    assert!(service_mod.contains("pub mod rest"), "{service_mod}");
    assert!(service_mod.contains("pub mod connect"), "{service_mod}");
}
