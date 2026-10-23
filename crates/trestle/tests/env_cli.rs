//! `trestle env` through the real binary: `plan.json` beside every render, custom module
//! manifests, and `list-modules --json`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn trestle(args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_trestle"))
        .args(args)
        .output()
        .expect("run trestle");
    assert!(
        out.status.success(),
        "trestle {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn new_env(dir: &Path) {
    trestle(&[
        "env",
        "new",
        "demo",
        "--out-dir",
        dir.to_str().unwrap(),
        "--select",
        "unity-catalog",
        "--defaults",
    ]);
}

fn json(text: &[u8]) -> serde_json::Value {
    serde_json::from_slice(text).expect("valid JSON")
}

/// A module manifest that adds `probe` (not selected, so it needs no rendering).
const PROBE: &str = "id: probe\nmodel: lakehouse.catalog\nprovider_of: probe_role\n";

fn add_probe_manifest(env: &Path) {
    fs::write(env.join("probe.module.yaml"), PROBE).unwrap();
    let toml = fs::read_to_string(env.join("env.toml")).unwrap();
    fs::write(
        env.join("env.toml"),
        format!("{toml}\n[catalog]\nmanifests = [\"probe.module.yaml\"]\n"),
    )
    .unwrap();
}

#[test]
fn every_render_writes_the_same_plan_json() {
    let tmp = tempfile::tempdir().unwrap();
    let env = tmp.path().join("env");
    new_env(&env);
    let first = fs::read(env.join("plan.json")).expect("`env new` writes plan.json");
    trestle(&["env", "render", env.to_str().unwrap()]);
    let second = fs::read(env.join("plan.json")).unwrap();
    trestle(&["env", "render", env.to_str().unwrap()]);
    let third = fs::read(env.join("plan.json")).unwrap();
    assert_eq!(first, second, "new and render disagree");
    assert_eq!(second, third, "render is not byte-stable");

    let plan = json(&first);
    assert_eq!(plan["format"], "trestle-plan");
    assert_eq!(plan["context"]["env_name"], "demo");
    let uc = plan["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "unity-catalog")
        .unwrap();
    assert_eq!(uc["model"], "unityCatalogServer");
}

#[test]
fn list_modules_json_lists_model_refs_and_the_catalog_identity() {
    let out = trestle(&["env", "list-modules", "--json"]);
    let listing = json(&out.stdout);
    assert_eq!(listing["format"], "trestle-catalog");
    assert_eq!(listing["version"], 1);
    assert_eq!(listing["generator"]["name"], "trestle");
    assert!(
        listing["catalog"]["digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let modules = listing["modules"].as_array().unwrap();
    let rustfs = modules.iter().find(|m| m["id"] == "rustfs").unwrap();
    assert_eq!(rustfs["model"], "rustfs");
    assert_eq!(rustfs["provider_of"], "object_store");
    assert!(
        rustfs["helpers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["name"] == "rustfs-init")
    );
}

#[test]
fn an_environment_plans_with_its_module_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    let env = tmp.path().join("env");
    new_env(&env);
    let baseline = json(&fs::read(env.join("plan.json")).unwrap());
    add_probe_manifest(&env);

    let listing = json(
        &trestle(&[
            "env",
            "list-modules",
            "--json",
            "--dir",
            env.to_str().unwrap(),
        ])
        .stdout,
    );
    let probe = listing["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "probe")
        .expect("the manifest's module is listed");
    assert_eq!(probe["model"], "lakehouse.catalog");
    assert_eq!(
        listing["catalog"]["manifests"],
        serde_json::json!(["probe.module.yaml"])
    );

    trestle(&["env", "render", env.to_str().unwrap()]);
    let custom = json(&fs::read(env.join("plan.json")).unwrap());
    assert_eq!(
        custom["catalog"]["manifests"],
        serde_json::json!(["probe.module.yaml"])
    );
    assert_ne!(custom["catalog"]["digest"], baseline["catalog"]["digest"]);
    assert_eq!(custom["catalog"]["digest"], listing["catalog"]["digest"]);
}

#[test]
fn a_malformed_module_manifest_names_its_file() {
    let tmp = tempfile::tempdir().unwrap();
    let env = tmp.path().join("env");
    new_env(&env);
    add_probe_manifest(&env);
    fs::write(
        env.join("probe.module.yaml"),
        "id: probe\nmodel: not-a-ref\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_trestle"))
        .args(["env", "render", env.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("probe.module.yaml"), "{stderr}");
    assert!(stderr.contains("invalid model ref"), "{stderr}");
}

#[test]
fn version_names_the_revision_when_the_build_recorded_one() {
    let out = trestle(&["--version"]);
    let version = String::from_utf8(out.stdout).unwrap();
    let expected = match option_env!("TRESTLE_REVISION") {
        Some(revision) => format!("trestle {} ({revision})\n", env!("CARGO_PKG_VERSION")),
        None => format!("trestle {}\n", env!("CARGO_PKG_VERSION")),
    };
    assert_eq!(version, expected);
}

#[test]
fn diagram_prints_the_environment_as_mermaid() {
    let tmp = tempfile::tempdir().unwrap();
    let env = tmp.path().join("env");
    new_env(&env);
    let out = trestle(&["env", "diagram", env.to_str().unwrap()]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("flowchart LR\n"), "got:\n{text}");
    assert!(text.contains("<b>Unity Catalog</b>"), "got:\n{text}");

    let readme = fs::read_to_string(env.join("README.md")).unwrap();
    assert!(
        readme.contains(&format!("```mermaid\n{text}```")),
        "README.md embeds the same diagram"
    );

    let fenced = trestle(&["env", "diagram", "--fenced", env.to_str().unwrap()]);
    assert_eq!(
        String::from_utf8(fenced.stdout).unwrap(),
        format!("```mermaid\n{text}```\n")
    );
}
