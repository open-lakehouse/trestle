//! `plan.json` goldens and invariants. Each `tests/fixtures/plan/<name>/env.toml` plans against
//! the baseline catalog, and the export must equal the committed `plan.json` beside it.
//!
//! Regenerate the goldens after an intended change:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test -p olai-stack-topology --test plan_export
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use olai_stack_topology::projection::export::{
    ExportMeta, Generator, PLAN_EXPORT_FORMAT, PlanExport, plan_export, plan_export_json,
};
use olai_stack_topology::{EnvManifest, PlanCtx, Secrets, baseline_catalog};

const FIXTURES: &[&str] = &[
    "baseline-uc",
    "custom-context",
    "docs-uc-aws",
    "docs-uc-default",
    "hosted",
    "uc-api-storage",
];

/// Fixed provenance, so a release's version bump doesn't rewrite the goldens.
fn meta() -> ExportMeta {
    ExportMeta {
        generator: Generator {
            name: "trestle".into(),
            version: "0.0.0-test".into(),
            revision: None,
        },
        baseline: "olai-stack-topology@test".into(),
    }
}

fn fixture_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/plan")
        .join(name)
}

fn manifest(name: &str) -> EnvManifest {
    let path = fixture_dir(name).join("env.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    EnvManifest::from_toml(&text).unwrap()
}

fn export_of(manifest: &EnvManifest) -> (PlanExport, String) {
    let catalog = baseline_catalog();
    let plan = manifest.plan(&catalog).expect("fixture plans");
    let out = plan.materialize().expect("fixture renders");
    let export = plan_export(&plan, &out, manifest, &catalog, &meta()).unwrap();
    let text = plan_export_json(&export).unwrap();
    (export, text)
}

#[test]
fn exports_match_their_goldens() {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some();
    for name in FIXTURES {
        let (_, text) = export_of(&manifest(name));
        let golden = fixture_dir(name).join("plan.json");
        if update {
            std::fs::write(&golden, &text).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden)
            .unwrap_or_else(|e| panic!("{golden:?}: {e} (run with UPDATE_GOLDENS=1)"));
        assert!(
            text == expected,
            "{name}: plan.json drifted from its golden (run with UPDATE_GOLDENS=1 and review the diff)"
        );
    }
}

/// Fixtures whose rendered files are committed beside their `plan.json`, under `rendered/`.
const RENDERED: &[&str] = &["docs-uc-aws", "docs-uc-default", "uc-api-storage"];

/// The materialized files a fixture's golden holds: everything but credentials (which inline
/// secrets make ordinary), the human-readable layout report, and `.env` files, which this
/// repository's `.gitignore` excludes (`plan.json` still lists them).
fn rendered_files(name: &str) -> BTreeMap<String, String> {
    let plan = manifest(name)
        .plan(&baseline_catalog())
        .expect("fixture plans");
    plan.materialize()
        .expect("fixture renders")
        .files
        .into_iter()
        .filter(|f| !f.sensitive && f.path != "LAYOUT.md")
        .filter(|f| !f.path.split('/').any(|part| part == ".env"))
        .map(|f| (f.path, f.contents))
        .collect()
}

#[test]
fn rendered_files_match_their_goldens() {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some();
    for name in RENDERED {
        let root = fixture_dir(name).join("rendered");
        let files = rendered_files(name);
        if update {
            let _ = std::fs::remove_dir_all(&root);
            for (path, contents) in &files {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, contents).unwrap();
            }
            continue;
        }
        let committed: BTreeMap<String, String> = walk(&root)
            .into_iter()
            .map(|path| {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                (rel, std::fs::read_to_string(&path).unwrap())
            })
            .collect();
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            committed.keys().collect::<Vec<_>>(),
            "{name}: rendered file set drifted (run with UPDATE_GOLDENS=1)"
        );
        for (path, contents) in &files {
            assert!(
                *contents == committed[path],
                "{name}: {path} drifted from its golden (run with UPDATE_GOLDENS=1)"
            );
        }
    }
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{dir:?}: {e}")) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn the_docs_aws_environment_keeps_the_docs_contract() {
    let files = rendered_files("docs-uc-aws");
    // Inline secrets: the properties are ordinary files, and match the docs' hand-written
    // server.aws.properties key for key.
    let props = &files["modules/unity-catalog/secrets/server.properties"];
    let keys: Vec<&str> = props
        .lines()
        .filter(|l| !l.starts_with('#') && l.contains('='))
        .map(|l| l.split('=').next().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "server.env",
            "server.authorization",
            "aws.accessKey",
            "aws.secretKey",
            "aws.region",
            "aws.masterRoleArn",
        ]
    );
    assert!(!files.contains_key("modules/unity-catalog/storage.txt"));

    let plan = manifest("docs-uc-aws").plan(&baseline_catalog()).unwrap();
    let host = &plan.client_env_host;
    assert_eq!(
        host.get("UNITY_CATALOG_URL"),
        Some("http://localhost:8080/api/2.1/unity-catalog")
    );
    assert_eq!(host.get("AWS_ENDPOINT_URL"), Some("http://localhost:9000"));
    assert_eq!(host.get("AWS_ALLOW_HTTP"), Some("true"));

    let compose: serde_yaml::Value =
        serde_yaml::from_str(&files["modules/unity-catalog/compose.yaml"]).unwrap();
    let uc = &compose["services"]["unitycatalog"];
    assert_eq!(uc["container_name"], "unitycatalog");
    assert_eq!(uc["build"]["context"], "./image");
    assert_eq!(uc["init"], true);
}

#[test]
fn the_docs_default_environment_runs_only_the_server() {
    let plan = manifest("docs-uc-default")
        .plan(&baseline_catalog())
        .unwrap();
    let mut ids: Vec<&str> = plan.graph.nodes.iter().map(|m| m.id().as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["envoy", "unity-catalog"]);
}

#[test]
fn identical_inputs_give_identical_bytes() {
    for name in FIXTURES {
        let (_, first) = export_of(&manifest(name));
        let (_, second) = export_of(&manifest(name));
        assert_eq!(first, second, "{name}");
    }
}

#[test]
fn the_envelope_names_its_format_and_provenance() {
    let (export, _) = export_of(&manifest("baseline-uc"));
    assert_eq!(export.format, PLAN_EXPORT_FORMAT);
    assert_eq!(export.version, 1);
    assert!(export.recipe.digest.starts_with("sha256:"));
    assert_eq!(export.catalog.digest.len(), "sha256:".len() + 64);
    assert!(export.catalog.manifests.is_empty());
}

#[test]
fn the_recipe_digest_tracks_the_recipe() {
    let base = manifest("baseline-uc");
    let mut changed = base.clone();
    changed.context.env_name = "other".into();
    assert_ne!(
        export_of(&base).0.recipe.digest,
        export_of(&changed).0.recipe.digest
    );
}

#[test]
fn a_custom_context_survives_into_the_export() {
    let (export, _) = export_of(&manifest("custom-context"));
    assert_eq!(export.context.env_name, "docs-uc");
    assert_eq!(export.context.gateway_host_port, 8080);
    let gateway = export.graph.nodes.iter().find(|n| n.id == "envoy").unwrap();
    assert!(gateway.surfaces.iter().any(|s| s.host_port == 8080));
    // The preferred provider replaced the default object store.
    assert!(export.graph.nodes.iter().any(|n| n.id == "azurite"));
    assert!(!export.graph.nodes.iter().any(|n| n.id == "rustfs"));
}

#[test]
fn containers_are_services_and_helpers_with_their_component() {
    let (export, _) = export_of(&manifest("baseline-uc"));
    let find = |name: &str| export.containers.iter().find(|c| c.name == name);
    assert_eq!(find("unitycatalog").unwrap().component, "unity-catalog");
    let init = find("rustfs-init").expect("rustfs-init listed");
    assert_eq!(init.component, "rustfs");
    assert!(export.files.iter().any(|f| f.path == "compose.yaml"));
}

#[test]
fn hosted_nodes_carry_a_model_ref_and_run_no_container() {
    let (export, _) = export_of(&manifest("hosted"));
    let s3 = export
        .graph
        .nodes
        .iter()
        .find(|n| n.id == "aws-s3")
        .unwrap();
    assert_eq!(s3.model.as_deref(), Some("amazonS3"));
    assert!(!export.containers.iter().any(|c| c.component == "aws-s3"));
}

#[test]
fn no_secret_value_reaches_the_export() {
    let mut manifest = manifest("hosted");
    let secrets: Secrets = [
        ("AWS_ACCESS_KEY_ID", "AKIA-EXPORT-CANARY"),
        ("AWS_SECRET_ACCESS_KEY", "secret-export-canary"),
    ]
    .into_iter()
    .collect();
    manifest.context = PlanCtx {
        secrets: Some(secrets),
        ..manifest.context
    };
    let catalog = baseline_catalog();
    let plan = manifest.plan(&catalog).unwrap();
    let out = plan.materialize().unwrap();
    // The canaries really were resolved into the rendered files…
    assert!(
        out.files
            .iter()
            .any(|f| f.sensitive && f.contents.contains("secret-export-canary"))
    );
    // …and none of them reach the export, sensitive file contents included.
    let text =
        plan_export_json(&plan_export(&plan, &out, &manifest, &catalog, &meta()).unwrap()).unwrap();
    for canary in ["AKIA-EXPORT-CANARY", "secret-export-canary"] {
        assert!(!text.contains(canary), "{canary} leaked into plan.json");
    }
    // File entries carry paths only.
    assert!(
        !text.contains("\"contents\""),
        "plan.json carries file contents"
    );
}

fn schema_validator() -> jsonschema::Validator {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/plan.v1.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    jsonschema::validator_for(&schema).expect("the schema is valid JSON Schema")
}

#[test]
fn every_golden_matches_the_schema() {
    let validator = schema_validator();
    for name in FIXTURES {
        let (_, text) = export_of(&manifest(name));
        let instance: serde_json::Value = serde_json::from_str(&text).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:#?}");
    }
}

#[test]
fn the_schema_rejects_a_malformed_export() {
    let validator = schema_validator();
    let (_, text) = export_of(&manifest("baseline-uc"));
    let good: serde_json::Value = serde_json::from_str(&text).unwrap();

    let mut no_key = good.clone();
    no_key["graph"]["edges"][0]
        .as_object_mut()
        .unwrap()
        .remove("key");
    let mut bad_model = good.clone();
    bad_model["graph"]["nodes"][1]["model"] = "unity-catalog".into();
    let mut leaked_body = good.clone();
    leaked_body["files"][0]["contents"] = "x".into();
    let mut future = good;
    future["version"] = 2.into();

    for (what, instance) in [
        ("edge without a key", no_key),
        ("kebab-case model ref", bad_model),
        ("file with contents", leaked_body),
        ("unknown version", future),
    ] {
        assert!(!validator.is_valid(&instance), "accepted {what}");
    }
}
