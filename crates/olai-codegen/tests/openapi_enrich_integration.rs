use std::fs;

use tempfile::TempDir;

/// Test that gnostic-style `$ref` values are rewritten to valid OpenAPI component refs.
#[test]
fn test_gnostic_ref_rewriting() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("openapi.yaml");
    let jsonschema_dir = dir.path().join("jsonschema");
    fs::create_dir_all(&jsonschema_dir).unwrap();

    // Use concat to avoid Rust 2021 reserved-prefix lint on `json"` in raw strings
    let gnostic_ref = concat!("#/$defs/example.catalog.v1.Catalog.schema.strict.", "json");
    let yaml = format!(
        "openapi: \"3.0.0\"\ninfo:\n  title: Test\n  version: \"1.0\"\ncomponents:\n  schemas:\n    Catalog:\n      type: object\n      properties:\n        id:\n          $ref: \"{gnostic_ref}\"\n"
    );
    fs::write(&spec_path, yaml).unwrap();

    olai_codegen::enrich_openapi(
        &spec_path,
        &jsonschema_dir,
        false,
        None,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();

    let result = fs::read_to_string(&spec_path).unwrap();
    assert!(
        result.contains("#/components/schemas/Catalog"),
        "expected rewritten ref in output:\n{result}"
    );
    assert!(
        !result.contains("#/$defs/"),
        "expected no gnostic refs remaining:\n{result}"
    );
}

/// 1.4 — a JSON Schema bundle whose root `$ref` does not match `#/$defs/<name>` must be
/// handled gracefully (skipped with a diagnostic), not silently coerced to an empty key.
///
/// The run still completes (`Ok`) — the per-file loop is best-effort — but the malformed
/// bundle is skipped, so the target schema is left unenriched rather than mangled.
#[test]
fn test_malformed_root_ref_is_skipped_not_silently_emptied() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("openapi.yaml");
    let jsonschema_dir = dir.path().join("jsonschema");
    fs::create_dir_all(&jsonschema_dir).unwrap();

    // Bundle with a root `$ref` that is missing the expected `#/$defs/` prefix.
    let bundle = r#"{
        "$ref": "Catalog",
        "$defs": {
            "Catalog": { "type": "object", "properties": { "name": { "type": "string", "minLength": 1 } } }
        }
    }"#;
    fs::write(
        jsonschema_dir.join("example.catalog.v1.Catalog.schema.strict.bundle.json"),
        bundle,
    )
    .unwrap();

    let yaml = "openapi: \"3.0.0\"\ninfo:\n  title: Test\n  version: \"1.0\"\ncomponents:\n  schemas:\n    Catalog:\n      type: object\n      properties:\n        name:\n          type: string\n";
    fs::write(&spec_path, yaml).unwrap();

    // Best-effort run completes despite the malformed bundle.
    olai_codegen::enrich_openapi(
        &spec_path,
        &jsonschema_dir,
        false,
        None,
        &Default::default(),
        &Default::default(),
    )
    .expect("run should not error on a malformed bundle ref");

    let result = fs::read_to_string(&spec_path).unwrap();
    // The malformed bundle was skipped, so no validation keyword leaked into the spec.
    assert!(
        !result.contains("minLength"),
        "malformed bundle should be skipped, not partially applied:\n{result}"
    );
}

/// Test that a valid openapi.yaml with no gnostic refs round-trips without corruption.
#[test]
fn test_round_trip_without_jsonschema() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("openapi.yaml");
    let jsonschema_dir = dir.path().join("jsonschema");
    fs::create_dir_all(&jsonschema_dir).unwrap();

    let yaml = r#"
openapi: "3.0.0"
info:
  title: Round Trip Test
  version: "1.0"
paths:
  /catalogs:
    get:
      summary: List catalogs
      responses:
        "200":
          description: OK
components:
  schemas:
    Catalog:
      type: object
      properties:
        name:
          type: string
"#;
    fs::write(&spec_path, yaml).unwrap();

    olai_codegen::enrich_openapi(
        &spec_path,
        &jsonschema_dir,
        false,
        None,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();

    let result = fs::read_to_string(&spec_path).unwrap();
    // Must parse back as valid YAML without error
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&result).expect("output should be valid YAML after round-trip");
    assert!(
        parsed.get("openapi").is_some(),
        "output should retain 'openapi' key"
    );
}

/// Pass 3 — `schema_renames` renames a component key AND rewrites every `$ref`
/// that targets it, while leaving unrelated schemas and refs untouched. A source
/// key absent from the spec is ignored (no panic, no spurious key).
#[test]
fn test_schema_rename_rewrites_key_and_refs() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("openapi.yaml");
    let jsonschema_dir = dir.path().join("jsonschema");
    fs::create_dir_all(&jsonschema_dir).unwrap();

    // `ListCatalogsResponse.catalogs[]` refs `Catalog`; `Schema` is unrelated;
    // `Absent` is named in the rename table but not present in the spec.
    let yaml = concat!(
        "openapi: \"3.0.0\"\n",
        "info:\n  title: Test\n  version: \"1.0\"\n",
        "components:\n",
        "  schemas:\n",
        "    Catalog:\n      type: object\n      properties:\n        name:\n          type: string\n",
        "    Schema:\n      type: object\n",
        "    ListCatalogsResponse:\n      type: object\n      properties:\n",
        "        catalogs:\n          type: array\n          items:\n            $ref: \"#/components/schemas/Catalog\"\n",
    );
    fs::write(&spec_path, yaml).unwrap();

    let mut renames = std::collections::BTreeMap::new();
    renames.insert("Catalog".to_string(), "CatalogInfo".to_string());
    renames.insert("Absent".to_string(), "AbsentInfo".to_string());

    olai_codegen::enrich_openapi(
        &spec_path,
        &jsonschema_dir,
        false,
        None,
        &renames,
        &Default::default(),
    )
    .unwrap();

    let parsed: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(&spec_path).unwrap())
        .expect("valid YAML after rename");
    let schemas = parsed
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_mapping())
        .unwrap();

    // Key renamed.
    assert!(schemas.contains_key(serde_yaml::Value::String("CatalogInfo".into())));
    assert!(!schemas.contains_key(serde_yaml::Value::String("Catalog".into())));
    // Unrelated schema untouched.
    assert!(schemas.contains_key(serde_yaml::Value::String("Schema".into())));
    // Absent source key did not create a spurious entry.
    assert!(!schemas.contains_key(serde_yaml::Value::String("AbsentInfo".into())));

    // The $ref was rewritten to the new key.
    let ref_str = parsed
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.get("ListCatalogsResponse"))
        .and_then(|r| r.get("properties"))
        .and_then(|p| p.get("catalogs"))
        .and_then(|c| c.get("items"))
        .and_then(|i| i.get("$ref"))
        .and_then(|r| r.as_str())
        .unwrap();
    assert_eq!(ref_str, "#/components/schemas/CatalogInfo");
}

/// `schema_overrides` rebuilds a clobbered component from the authoritative JSON
/// Schema instead of merge-enriching it. Simulates the gnostic meta-model
/// collision: the spec's `Schema` carries meta-object fields, but the JSON Schema
/// bundle has the real entity — the override must replace, not merge.
#[test]
fn test_schema_override_rebuilds_from_jsonschema() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("openapi.yaml");
    let jsonschema_dir = dir.path().join("jsonschema");
    fs::create_dir_all(&jsonschema_dir).unwrap();

    // Authoritative bundle: the REAL Schema entity.
    let bundle = concat!(
        r#"{"$ref":"#,
        "\"#/$defs/example.schemas.v1.Schema.schema.strict.",
        "json\"",
        r#","$defs":{"example.schemas.v1.Schema.schema.strict."#,
        "json",
        r#"":{"type":"object","required":["name","full_name"],"properties":{"#,
        r#""name":{"type":"string"},"full_name":{"type":"string"},"catalog_name":{"type":"string"}}}}}"#,
    );
    fs::write(
        jsonschema_dir.join("example.schemas.v1.Schema.schema.strict.bundle.json"),
        bundle,
    )
    .unwrap();

    // Spec's `Schema` is the CLOBBERED gnostic meta-object (wrong fields).
    let yaml = concat!(
        "openapi: \"3.0.0\"\n",
        "info:\n  title: Test\n  version: \"1.0\"\n",
        "components:\n  schemas:\n",
        "    Schema:\n      type: object\n      properties:\n",
        "        nullable:\n          type: boolean\n",
        "        discriminator:\n          type: string\n",
    );
    fs::write(&spec_path, yaml).unwrap();

    let mut overrides = std::collections::HashSet::new();
    overrides.insert("Schema".to_string());

    olai_codegen::enrich_openapi(
        &spec_path,
        &jsonschema_dir,
        false,
        None,
        &Default::default(),
        &overrides,
    )
    .unwrap();

    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(&spec_path).unwrap()).unwrap();
    let props = parsed
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.get("Schema"))
        .and_then(|s| s.get("properties"))
        .and_then(|p| p.as_mapping())
        .unwrap();

    // Real entity fields present; meta-object fields gone.
    assert!(props.contains_key(serde_yaml::Value::String("full_name".into())));
    assert!(props.contains_key(serde_yaml::Value::String("catalog_name".into())));
    assert!(!props.contains_key(serde_yaml::Value::String("discriminator".into())));
    assert!(!props.contains_key(serde_yaml::Value::String("nullable".into())));
    // No JSON-Schema-only key leaked.
    let schema = parsed
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.get("Schema"))
        .and_then(|s| s.as_mapping())
        .unwrap();
    assert!(!schema.contains_key(serde_yaml::Value::String("$schema".into())));
}
