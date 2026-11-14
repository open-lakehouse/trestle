//! The authorization vocabulary: which securable each request acts on, and the emitted
//! `authz.rs`, `<service>/authz.rs` and `authz.cedarschema`.
//!
//! The fixture is `proto/example_authz.bin`, compiled from `proto/example_authz.proto`.
//! Regenerate it from the crate root with:
//!
//! ```sh
//! protoc -I proto -I tests/rest_client_e2e/proto --include_imports --include_source_info \
//!   -o proto/example_authz.bin proto/example_authz.proto
//! ```
//!
//! The emitted files are compared against `tests/golden_authz/`; re-bless them with
//! `BLESS=1 cargo test -p olai-codegen --test authz_integration`.

use std::path::{Path, PathBuf};

use olai_codegen::parsing::{AuthzRule, parse_file_descriptor_set};
use olai_codegen::{
    AuthzConfig, CodeGenConfig, CodeGenMetadata, CodeGenOutput, NameField, NameFieldKind,
    RestClientOptions, Runtime, SecurableKind, analyze_metadata, generate_code, plan_authz,
};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;
use rstest::rstest;
use tempfile::TempDir;

fn metadata() -> CodeGenMetadata {
    let bytes = include_bytes!("../proto/example_authz.bin");
    let fds = FileDescriptorSet::parse_from_bytes(bytes).expect("valid descriptor");
    parse_file_descriptor_set(&fds).expect("parse succeeds")
}

fn authz() -> AuthzConfig {
    AuthzConfig {
        cedar_namespace: "Example".into(),
        principal_types: vec!["Accounts::User".into(), "Accounts::Robot".into()],
        context_type: Some("RequestContext".into()),
        root_entity: "Service".into(),
        ..Default::default()
    }
}

fn config(tmp: &Path, runtime: Runtime, authz: Option<AuthzConfig>) -> CodeGenConfig {
    CodeGenConfig {
        context_type_path: "crate::Context".into(),
        result_type_path: "crate::Result".into(),
        models_path_template: "example_common::models::{service}::v1".into(),
        models_path_crate_template: "crate::models::{service}::v1".into(),
        resource_store_crate_name: "olai_store".into(),
        runtime,
        transport_type_path: olai_codegen::DEFAULT_TRANSPORT_TYPE_PATH.into(),
        dual_transport: false,
        client_protocols: olai_codegen::ClientProtocols::default(),
        connect_client_path: None,
        authz,
        rest_client: RestClientOptions::default(),
        output: CodeGenOutput {
            common: Some(tmp.join("common")),
            models: None,
            models_subdir: "_gen".into(),
            server: Some(tmp.join("server")),
            client: None,
            python: None,
            node: None,
            node_ts: None,
            wasm: None,
            generate_resource_clients: false,
        },
        generate_resource_enum: true,
        generate_store_integration: false,
        error_type_path: None,
        generate_object_conversions: false,
        bindings: None,
    }
}

fn field(path: &str, kind: NameFieldKind) -> NameField {
    NameField {
        path: path.split('.').map(str::to_string).collect(),
        kind,
    }
}

fn resource(label: &str) -> SecurableKind {
    SecurableKind::Resource(label.into())
}

/// Every way of finding a securable, one RPC each.
#[rstest]
#[case::root_list("ListCatalogs", SecurableKind::Root, vec![])]
#[case::flat_create("CreateCatalog", resource("Catalog"), vec![field("name", NameFieldKind::String)])]
#[case::get_by_path("GetCatalog", resource("Catalog"), vec![field("name", NameFieldKind::String)])]
#[case::list_in_container(
    "ListSchemas",
    resource("Catalog"),
    vec![field("catalog_name", NameFieldKind::String)]
)]
#[case::nested_create(
    "CreateSchema",
    resource("Schema"),
    vec![field("catalog_name", NameFieldKind::String), field("name", NameFieldKind::String)]
)]
#[case::list_in_nested_container(
    "ListTables",
    resource("Schema"),
    vec![
        field("catalog_name", NameFieldKind::String),
        field("schema_name", NameFieldKind::String),
    ]
)]
#[case::collection_custom_method(
    "ListTableSummaries",
    resource("Catalog"),
    vec![field("catalog_name", NameFieldKind::String)]
)]
#[case::override_with_nested_fields(
    "CreateTable",
    resource("Table"),
    vec![
        field("table_info.catalog_name", NameFieldKind::String),
        field("table_info.schema_name", NameFieldKind::String),
        field("table_info.name", NameFieldKind::String),
    ]
)]
#[case::type_reference_on_a_resource_service(
    "GetTable",
    resource("Table"),
    vec![field("full_name", NameFieldKind::String)]
)]
#[case::integer_part(
    "GetTableVersion",
    resource("Table"),
    vec![field("full_name", NameFieldKind::String), field("version", NameFieldKind::Integer)]
)]
#[case::skip_is_any_resource("GetPermissions", SecurableKind::Any, vec![])]
#[case::type_reference_without_a_resource(
    "GetTablePermissions",
    resource("Table"),
    vec![field("table_name", NameFieldKind::String)]
)]
#[case::override_naming_the_parent(
    "CreateDraft",
    resource("Draft"),
    vec![field("catalog_name", NameFieldKind::String), field("name", NameFieldKind::String)]
)]
#[case::override_with_optional_field(
    "CheckTable",
    resource("Table"),
    vec![field("table_name", NameFieldKind::OptionalString)]
)]
fn securable_of_each_request(
    #[case] rpc: &str,
    #[case] kind: SecurableKind,
    #[case] name_fields: Vec<NameField>,
) {
    let metadata = metadata();
    let plan = analyze_metadata(&metadata).unwrap();
    let operations = plan_authz(&plan, &metadata).unwrap();
    let op = operations.iter().find(|op| op.name == rpc).unwrap();
    assert_eq!(op.securable.kind, kind, "{rpc}");
    assert_eq!(op.securable.name_fields, name_fields, "{rpc}");
    assert_eq!(op.emit_impl, rpc != "GetPermissions", "{rpc}");
}

/// The method `rpc` of `metadata`.
fn method_mut<'a>(
    metadata: &'a mut CodeGenMetadata,
    rpc: &str,
) -> &'a mut olai_codegen::parsing::MethodMetadata {
    metadata
        .services
        .values_mut()
        .flat_map(|s| s.methods.iter_mut())
        .find(|m| m.method_name == rpc)
        .unwrap()
}

fn plan_error(metadata: &CodeGenMetadata) -> String {
    let plan = analyze_metadata(metadata).unwrap();
    plan_authz(&plan, metadata).unwrap_err().to_string()
}

#[test]
fn a_request_without_a_securable_fails_generation() {
    let mut metadata = metadata();
    method_mut(&mut metadata, "CreateTable").authz = None;
    method_mut(&mut metadata, "CheckTable").authz = None;
    let error = plan_error(&metadata);
    assert!(
        error.contains("can't find the securable of 2 request(s)"),
        "{error}"
    );
    assert!(
        error.contains("TableService.CreateTable: a create without the parent field"),
        "{error}"
    );
    assert!(
        error.contains("PermissionsService.CheckTable: the service manages no resource"),
        "{error}"
    );
}

#[test]
fn operations_must_be_unique_across_the_api() {
    let mut metadata = metadata();
    method_mut(&mut metadata, "GetTablePermissions").method_name = "GetTable".into();
    let error = plan_error(&metadata);
    assert!(error.contains("must be unique"), "{error}");
    assert!(
        error.contains("`GetTable` (in example.authz.v1, example.authz.v1)"),
        "{error}"
    );
}

#[test]
fn a_shared_request_gets_one_impl() {
    let mut metadata = metadata();
    let request = method_mut(&mut metadata, "GetTable").input_type.clone();
    method_mut(&mut metadata, "GetTablePermissions").input_type = request;
    let error = plan_error(&metadata);
    assert!(
        error.contains("GetTablePermissions, GetTable: the request"),
        "{error}"
    );
    assert!(error.contains("is shared"), "{error}");
}

#[test]
fn a_dropped_parent_field_is_refused() {
    let mut metadata = metadata();
    method_mut(&mut metadata, "CreateDraft").authz = None;
    let error = plan_error(&metadata);
    assert!(
        error.contains(
            "DraftService.CreateDraft: the parent field(s) `catalog_name` would not be part of \
             the resource's name"
        ),
        "{error}"
    );
}

#[rstest]
#[case::missing_field(
    AuthzRule { securable_type: "example.io/Table".into(), securable_fields: vec!["nope".into()], skip: false },
    "the request has no field `nope`"
)]
#[case::unknown_type(
    AuthzRule { securable_type: "example.io/Nope".into(), securable_fields: vec!["table_name".into()], skip: false },
    "no message declares the resource type `example.io/Nope`"
)]
#[case::type_without_fields(
    AuthzRule { securable_type: "example.io/Table".into(), securable_fields: vec![], skip: false },
    "a `securable_type` without `securable_fields`"
)]
#[case::fields_without_type(
    AuthzRule { securable_type: String::new(), securable_fields: vec!["table_name".into()], skip: false },
    "`securable_fields` without a `securable_type`"
)]
#[case::fields_with_skip(
    AuthzRule { securable_type: String::new(), securable_fields: vec!["table_name".into()], skip: true },
    "`securable_fields` with `skip`"
)]
fn a_bad_override_is_an_error(#[case] rule: AuthzRule, #[case] message: &str) {
    let mut metadata = metadata();
    method_mut(&mut metadata, "CheckTable").authz = Some(rule);
    let error = plan_error(&metadata);
    assert!(
        error.contains(&format!("PermissionsService.CheckTable: {message}")),
        "{error}"
    );
}

/// The files of the authorization vocabulary under `server`, relative to it.
fn authz_files(server: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        PathBuf::from("authz.rs"),
        PathBuf::from("authz.cedarschema"),
        PathBuf::from("mod.rs"),
    ];
    for entry in std::fs::read_dir(server).unwrap().flatten() {
        if entry.path().is_dir() {
            let service = PathBuf::from(entry.file_name());
            for file in ["authz.rs", "mod.rs"] {
                if entry.path().join(file).exists() {
                    files.push(service.join(file));
                }
            }
        }
    }
    files.sort();
    files
}

/// The emitted files match `tests/golden_authz/<runtime>/`.
#[rstest]
#[case::buffa(Runtime::Buffa, "buffa")]
#[case::prost(Runtime::Prost, "prost")]
fn emitted_files_match_the_golden_tree(#[case] runtime: Runtime, #[case] dir: &str) {
    let tmp = TempDir::new().unwrap();
    generate_code(&metadata(), &config(tmp.path(), runtime, Some(authz()))).unwrap();
    let server = tmp.path().join("server");
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden_authz")
        .join(dir);
    let files = authz_files(&server);

    if std::env::var_os("BLESS").is_some() {
        let _ = std::fs::remove_dir_all(&golden);
        for file in &files {
            let target = golden.join(file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(server.join(file), target).unwrap();
        }
        return;
    }
    for file in &files {
        let actual = std::fs::read_to_string(server.join(file)).unwrap();
        let expected = std::fs::read_to_string(golden.join(file)).unwrap_or_else(|_| {
            panic!(
                "no golden {}; run `BLESS=1 cargo test -p olai-codegen --test authz_integration`",
                file.display()
            )
        });
        assert_eq!(
            actual,
            expected,
            "{} differs from the golden tree; re-bless if intended",
            file.display()
        );
    }
}

#[test]
fn without_authz_nothing_is_emitted() {
    let tmp = TempDir::new().unwrap();
    generate_code(&metadata(), &config(tmp.path(), Runtime::Buffa, None)).unwrap();
    let server = tmp.path().join("server");
    assert!(!server.join("authz.rs").exists());
    assert!(!server.join("authz.cedarschema").exists());
    assert!(!server.join("table/authz.rs").exists());
    let module = std::fs::read_to_string(server.join("table/mod.rs")).unwrap();
    assert!(!module.contains("authz"), "{module}");
}

#[rstest]
#[case::no_resource_enum(
    |c: &mut CodeGenConfig| c.generate_resource_enum = false,
    "`generate_resource_enum`"
)]
#[case::bad_namespace(
    |c: &mut CodeGenConfig| c.authz.as_mut().unwrap().cedar_namespace = "not a name".into(),
    "`AuthzConfig::cedar_namespace` `not a name` is not a Cedar name"
)]
#[case::no_principals(
    |c: &mut CodeGenConfig| c.authz.as_mut().unwrap().principal_types.clear(),
    "at least one `principal_types` entry"
)]
#[case::bad_module_path(
    |c: &mut CodeGenConfig| c.authz.as_mut().unwrap().module_path = "not a path".into(),
    "authz.module_path"
)]
fn a_bad_authz_config_is_refused(#[case] change: fn(&mut CodeGenConfig), #[case] message: &str) {
    let tmp = TempDir::new().unwrap();
    let mut config = config(tmp.path(), Runtime::Buffa, Some(authz()));
    change(&mut config);
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains(message), "{error}");
}
