//! The exported module catalog: a golden of the baseline, checked against its schema.
//!
//! Regenerate the golden after an intended change:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p olai-stack-topology --test catalog_export
//! ```

use std::path::PathBuf;

use olai_stack_topology::baseline_catalog;
use olai_stack_topology::projection::export::{
    CATALOG_EXPORT_FORMAT, ExportMeta, Generator, catalog_export, catalog_export_json,
};

/// Fixed provenance, so a release's version bump doesn't rewrite the golden.
fn meta() -> ExportMeta {
    ExportMeta {
        generator: Generator {
            name: "trestle".into(),
            version: "0.0.0-test".into(),
            revision: Some("0000000000000000000000000000000000000000".into()),
        },
        baseline: "olai-stack-topology@test".into(),
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn baseline_json() -> serde_json::Value {
    let export = catalog_export(&baseline_catalog(), &[], &meta()).unwrap();
    serde_json::from_str(&catalog_export_json(&export).unwrap()).unwrap()
}

fn validator() -> jsonschema::Validator {
    let path = manifest_dir().join("schema/catalog.v1.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    jsonschema::validator_for(&schema).expect("the schema is valid JSON Schema")
}

#[test]
fn the_baseline_matches_its_golden() {
    let export = catalog_export(&baseline_catalog(), &[], &meta()).unwrap();
    let text = catalog_export_json(&export).unwrap();
    let golden = manifest_dir().join("tests/fixtures/catalog/baseline.json");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&golden, &text).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("{golden:?}: {e} (run with UPDATE_GOLDENS=1)"));
    assert!(
        text == expected,
        "the catalog drifted from its golden (run with UPDATE_GOLDENS=1 and review the diff)"
    );
}

#[test]
fn the_envelope_matches_the_schema() {
    let instance = baseline_json();
    assert_eq!(instance["format"], CATALOG_EXPORT_FORMAT);
    let errors: Vec<String> = validator()
        .iter_errors(&instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn the_schema_rejects_a_malformed_catalog() {
    let validator = validator();
    let good = baseline_json();
    let mut unversioned = good.clone();
    unversioned.as_object_mut().unwrap().remove("version");
    let mut bad_model = good.clone();
    bad_model["modules"][0]["model"] = "unity-catalog".into();
    let mut bad_helper = good.clone();
    bad_helper["modules"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|m| m["id"] == "rustfs")
        .unwrap()["helpers"][0]["kind"] = "daemon".into();
    let mut future = good;
    future["version"] = 2.into();
    for (what, instance) in [
        ("no version", unversioned),
        ("kebab-case model ref", bad_model),
        ("unknown helper kind", bad_helper),
        ("unknown version", future),
    ] {
        assert!(!validator.is_valid(&instance), "{what} was accepted");
    }
}
