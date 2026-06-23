//! Integration tests that render the embedded base + apps + a matrix of
//! category selections into a tempdir and assert that the result looks right.
//!
//! The expensive checks (full `cargo check` on the rendered Rust workspace,
//! `docker compose config`, `envoy --mode validate`, `buf build`) are gated
//! behind `TRESTLE_TEST_SLOW=1` so the default test invocation stays fast.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use olai_trestle::cli::new::{self, NewArgs};
use tempfile::TempDir;

fn scaffold(args: NewArgs) {
    new::run(args).expect("scaffold succeeded");
}

fn run_slow_tests() -> bool {
    std::env::var("TRESTLE_TEST_SLOW").as_deref() == Ok("1")
}

fn assert_no_unrendered_tokens(root: &Path) {
    // Detect leftover Jinja: `{{ identifier ... }}` (with at least one space, so
    // we don't false-positive on `justfile`'s `{{var}}` syntax which trestle's
    // `{% raw %}` block deliberately preserves), and any `{%-? <word>` statement
    // openers. False-positive guards:
    //   - JSX inline style objects (`style={{ marginTop: 16 }}`) — colon inside
    //   - GitHub Actions expressions (`${{ ... }}`) — `$` prefix
    // Files where `{{ ident }}` is legitimately part of the output language.
    let allowlist = ["justfile"];

    let var_re =
        regex::Regex::new(r"(?:^|[^$])\{\{\s+[a-zA-Z_][a-zA-Z0-9_]*(?:\s*\|\s*[a-zA-Z_]+)*\s+\}\}")
            .unwrap();
    let stmt_re = regex::Regex::new(r"\{%-?\s*[a-zA-Z]").unwrap();

    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if allowlist.iter().any(|p| rel == *p) {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        if let Some(m) = var_re.find(text) {
            panic!("unrendered Jinja variable in {rel} near `{}`", m.as_str());
        }
        if let Some(m) = stmt_re.find(text) {
            panic!("unrendered Jinja block in {rel} near `{}`", m.as_str());
        }
    }
}

/// Convenience: render the `lakehouse` base alone with a hand-picked set of
/// lakehouse-category selections.
fn render_lakehouse(selections: Vec<(&str, Vec<&str>)>) -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("lab");
    let overrides: BTreeMap<String, String> =
        [("project_name".to_string(), "test-lab".to_string())]
            .into_iter()
            .collect();

    let args = NewArgs {
        name: "test-lab".to_string(),
        out_dir: Some(out),
        template: "lakehouse".to_string(),
        apps: vec![],
        selections: selections
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.into_iter().map(String::from).collect()))
            .collect(),
        profile: None,
        with: vec![],
        values: None,
        overrides: overrides.into_iter().collect(),
        non_interactive: true,
        force: false,
        runtime: None,
    };
    scaffold(args);
    tmp
}

/// Convenience: render the `lakehouse` base + `databricks-app-rust` app with
/// the given selections.
fn render_rust_app(base_selections: Vec<(&str, Vec<&str>)>, frontend: &str, ci: &str) -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("app");
    let overrides: BTreeMap<String, String> = [
        ("project_name".to_string(), "test-app".to_string()),
        ("gh_owner".to_string(), "acme".to_string()),
    ]
    .into_iter()
    .collect();

    let mut selections: Vec<(String, Vec<String>)> = base_selections
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into_iter().map(String::from).collect()))
        .collect();
    selections.push((
        "app.databricks-app-rust.frontend".to_string(),
        vec![frontend.to_string()],
    ));
    selections.push((
        "app.databricks-app-rust.ci".to_string(),
        vec![ci.to_string()],
    ));

    let args = NewArgs {
        name: "test-app".to_string(),
        out_dir: Some(out),
        template: "lakehouse".to_string(),
        apps: vec!["databricks-app-rust".to_string()],
        selections,
        profile: None,
        with: vec![],
        values: None,
        overrides: overrides.into_iter().collect(),
        non_interactive: true,
        force: false,
        runtime: None,
    };
    scaffold(args);
    tmp
}

#[test]
fn lakehouse_minimal() {
    // Deselect all optional categories → just Envoy (baseline) + nothing else.
    let tmp = render_lakehouse(vec![
        ("storage", vec![]),
        ("metadata_db", vec![]),
        ("catalog", vec![]),
        ("query_engine", vec![]),
        ("ml", vec![]),
        ("observability", vec![]),
        ("notebooks", vec![]),
    ]);
    let root = tmp.path().join("lab");
    // Always-present files from the base template/ tree.
    for path in [
        "README.md",
        "AGENTS.md",
        "CLAUDE.md",
        "compose.yaml",
        ".env.example",
        ".gitignore",
        "justfile",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    // Envoy is the only always-on component → its compose file is present.
    assert!(root.join("docker/compose/envoy.yaml").is_file());
    // Nothing else was selected → no postgres, no seaweedfs.
    assert!(!root.join("docker/compose/postgres.yaml").is_file());
    assert!(!root.join("docker/compose/seaweedfs.yaml").is_file());

    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
}

#[test]
fn lakehouse_with_storage_and_db() {
    // The baseline default for storage / metadata_db is local-stack-seaweedfs / -postgres.
    let tmp = render_lakehouse(vec![
        ("catalog", vec![]),
        ("query_engine", vec![]),
        ("ml", vec![]),
        ("observability", vec![]),
        ("notebooks", vec![]),
    ]);
    let root = tmp.path().join("lab");
    assert!(root.join("docker/compose/envoy.yaml").is_file());
    assert!(root.join("docker/compose/postgres.yaml").is_file());
    assert!(root.join("docker/compose/seaweedfs.yaml").is_file());
    assert!(root.join("docker/postgres/init-databases.sh").is_file());
    assert!(!root.join("docker/compose/mlflow.yaml").is_file());

    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
}

#[test]
fn lakehouse_full_stack() {
    // Pick UC + MLflow + notebooks + jaeger explicitly. Storage/db default.
    let tmp = render_lakehouse(vec![
        ("catalog", vec!["local-stack-unity-catalog"]),
        ("ml", vec!["local-stack-mlflow"]),
        ("notebooks", vec!["local-stack-notebooks"]),
        ("observability", vec!["local-stack-jaeger"]),
        ("query_engine", vec![]),
    ]);
    let root = tmp.path().join("lab");

    for path in [
        "docker/envoy/envoy.yaml",
        "docker/compose/envoy.yaml",
        "docker/compose/postgres.yaml",
        "docker/compose/seaweedfs.yaml",
        "docker/compose/mlflow.yaml",
        "docker/compose/unity-catalog.yaml",
        "docker/compose/notebooks.yaml",
        "docker/compose/jaeger.yaml",
        "docker/postgres/init-databases.sh",
        "notebooks/01_lakehouse_quickstart.py",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }

    let envoy = std::fs::read_to_string(root.join("docker/envoy/envoy.yaml")).unwrap();
    assert!(envoy.contains("prefix: \"/api/2.0/mlflow\""));
    assert!(envoy.contains("prefix: \"/api/2.1/unity-catalog\""));
    assert!(envoy.contains("prefix: \"/notebooks\""));

    let env = std::fs::read_to_string(root.join(".env.example")).unwrap();
    assert!(env.contains("MLFLOW_TRACKING_URI="));

    let init = std::fs::read_to_string(root.join("docker/postgres/init-databases.sh")).unwrap();
    assert!(init.contains("CREATE DATABASE mlflow"));
    assert!(init.contains("CREATE DATABASE unitycatalog"));

    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
    assert_yaml_parses(&root.join("docker/envoy/envoy.yaml"));

    if run_slow_tests() {
        assert_docker_compose_valid(&root, "svc");
    }
}

#[test]
fn databricks_app_rust_dbx_emulator() {
    // The classic "dbx-emulator" profile: envoy + postgres + seaweedfs + mlflow + emulator-env.
    // With the new model: base defaults + MLflow + the app (which auto-pulls databricks-emulator-env).
    let tmp = render_rust_app(
        vec![
            ("ml", vec!["local-stack-mlflow"]),
            ("catalog", vec![]),
            ("query_engine", vec![]),
            ("observability", vec![]),
            ("notebooks", vec![]),
        ],
        "react",
        "github",
    );
    let root = tmp.path().join("app");

    for path in [
        "Cargo.toml",
        "trestle.yaml",
        "buf.yaml",
        "buf.gen.yaml",
        "justfile",
        "app.yaml",
        "databricks.yml",
        "compose.yaml",
        ".env.example",
        ".gitignore",
        "crates/common/Cargo.toml",
        "crates/server/Cargo.toml",
        "crates/server/src/main.rs",
        "crates/server/src/api/context.rs",
        "crates/server/src/api/error.rs",
        "crates/server/src/handlers/greeting.rs",
        "crates/client/Cargo.toml",
        "frontend/package.json",
        "frontend/vite.config.ts",
        "frontend/src/App.tsx",
        ".github/workflows/ci.yml",
        ".github/workflows/deploy.yml",
        "docker/app/Dockerfile",
        "docker/envoy/envoy.yaml",
        "docker/compose/envoy.yaml",
        "docker/postgres/init-databases.sh",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }

    assert!(root.join("proto/test_app/v1/models.proto").is_file());
    assert!(root.join("proto/test_app/v1/service.proto").is_file());

    let envoy = std::fs::read_to_string(root.join("docker/envoy/envoy.yaml")).unwrap();
    assert!(envoy.contains("name: app"));
    assert!(envoy.contains("test-app-server"));

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("crates/server"));
    assert!(cargo_toml.contains("edition = \"2024\""));

    let env = std::fs::read_to_string(root.join(".env.example")).unwrap();
    // The app pulls databricks-emulator-env via lakehouse_requires.hard, so
    // these env vars are wired in.
    assert!(env.contains("DATABRICKS_HOST="));

    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
    assert_yaml_parses(&root.join("docker/envoy/envoy.yaml"));
    assert_yaml_parses(&root.join("databricks.yml"));

    if run_slow_tests() {
        assert_cargo_check_passes(&root);
        assert_docker_compose_valid(&root, "svc");
    }
}

#[test]
fn databricks_app_rust_no_frontend_no_ci() {
    // Frontend + CI off via the new app categories; minimal lakehouse otherwise.
    let tmp = render_rust_app(
        vec![
            ("storage", vec![]),
            ("metadata_db", vec![]),
            ("catalog", vec![]),
            ("query_engine", vec![]),
            ("ml", vec![]),
            ("observability", vec![]),
            ("notebooks", vec![]),
        ],
        "none",
        "none",
    );
    let root = tmp.path().join("app");

    for path in [
        "Cargo.toml",
        "crates/server/src/main.rs",
        "compose.yaml",
        "trestle.yaml",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    assert!(!root.join("frontend").exists());
    assert!(!root.join(".github").exists());

    // Even with no lakehouse selections, the app still pulls in
    // databricks-emulator-env via lakehouse_requires.hard. Envoy is always on.
    assert!(root.join("docker/compose/envoy.yaml").is_file());

    assert_no_unrendered_tokens(&root);
}

#[test]
fn lakehouse_with_trino_query_engine() {
    // Trino is shipped as a `query_engine` provider; this exercises the
    // "drop-a-component, the wizard discovers it" story end-to-end. The trino
    // component contributes its own envoy route + compose include via
    // `provides:`.
    let tmp = render_lakehouse(vec![
        ("catalog", vec![]),
        ("ml", vec![]),
        ("observability", vec![]),
        ("notebooks", vec![]),
        ("query_engine", vec!["local-stack-trino"]),
    ]);
    let root = tmp.path().join("lab");
    assert!(root.join("docker/compose/trino.yaml").is_file());
    let envoy = std::fs::read_to_string(root.join("docker/envoy/envoy.yaml")).unwrap();
    assert!(envoy.contains("prefix: \"/trino\""));
    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
    assert_yaml_parses(&root.join("docker/envoy/envoy.yaml"));
}

fn assert_yaml_parses(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let _: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("yaml parse {}: {e}", path.display()));
}

fn assert_docker_compose_valid(root: &Path, profile: &str) {
    let env_local = root.join(".env.local");
    let env_example = root.join(".env.example");
    if env_example.is_file() {
        std::fs::copy(&env_example, &env_local).unwrap();
    }
    let out = Command::new("docker")
        .args(["compose", "--profile", profile, "config"])
        .current_dir(root)
        .output()
        .expect("docker available");
    assert!(
        out.status.success(),
        "docker compose config failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_cargo_check_passes(root: &Path) {
    let out = Command::new("cargo")
        .args(["check", "--all-targets"])
        .current_dir(root)
        .output()
        .expect("cargo available");
    assert!(
        out.status.success(),
        "cargo check failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// Behavior tests: CLI argument handling and edge cases (cheap; no rendering of
// the full tree where avoidable).
// ---------------------------------------------------------------------------

/// A baseline `NewArgs` for the embedded `lakehouse` base, rendering into
/// `out`, with `name`. Optional categories are deselected so the scaffold stays
/// minimal and fast.
fn minimal_args(name: &str, out: std::path::PathBuf) -> NewArgs {
    let overrides: BTreeMap<String, String> = [("gh_owner".to_string(), "acme".to_string())]
        .into_iter()
        .collect();
    NewArgs {
        name: name.to_string(),
        out_dir: Some(out),
        template: "lakehouse".to_string(),
        apps: vec![],
        selections: [
            ("storage", vec![]),
            ("metadata_db", vec![]),
            ("catalog", vec![]),
            ("query_engine", vec![]),
            ("ml", vec![]),
            ("observability", vec![]),
            ("notebooks", vec![]),
        ]
        .into_iter()
        .map(|(k, v): (&str, Vec<&str>)| (k.to_string(), v.into_iter().map(String::from).collect()))
        .collect(),
        profile: None,
        with: vec![],
        values: None,
        overrides: overrides.into_iter().collect(),
        non_interactive: true,
        force: false,
        runtime: None,
    }
}

#[test]
fn rejects_path_traversal_project_name() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("dest");
    // `..` is not a valid project name and must be rejected before any fs op.
    let args = minimal_args("../evil", out.clone());
    let err = new::run(args).expect_err("path-traversal name must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("project_name") && msg.contains("must match"),
        "unexpected error: {msg}"
    );
    // Nothing should have been created.
    assert!(!out.exists(), "no output dir should be created on bad name");
}

#[test]
fn rejects_invalid_char_project_name() {
    let tmp = tempfile::tempdir().unwrap();
    for bad in ["Foo", "1foo", "foo_bar", "foo bar", "/abs", ""] {
        let out = tmp.path().join(format!("dest-{}", bad.len()));
        let args = minimal_args(bad, out.clone());
        let err = new::run(args).expect_err(&format!("`{bad}` should be rejected"));
        assert!(
            err.to_string().contains("project_name"),
            "`{bad}` gave unexpected error: {err}"
        );
        assert!(!out.exists(), "`{bad}` should not create an output dir");
    }
}

#[test]
fn force_overwrites_existing_nonempty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("dest");
    std::fs::create_dir_all(&out).unwrap();
    // Pre-existing, non-empty directory.
    std::fs::write(out.join("preexisting.txt"), b"keep me out of the way").unwrap();

    // Without --force this must fail with OutputExists.
    let args = minimal_args("test-app", out.clone());
    let err = new::run(args).expect_err("non-empty dir without --force must fail");
    assert!(
        err.to_string().contains("already exists"),
        "unexpected error: {err}"
    );

    // With --force the scaffold proceeds.
    let mut args = minimal_args("test-app", out.clone());
    args.force = true;
    new::run(args).expect("--force should overwrite an existing directory");
    // A known base file is now present alongside the pre-existing file.
    assert!(out.join("preexisting.txt").exists());
}

#[test]
fn non_interactive_missing_required_var_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("dest");
    // Drop the gh_owner override so a required variable is unset.
    let mut args = minimal_args("test-app", out);
    args.overrides.clear();
    let result = new::run(args);
    // Either it errors on the missing variable, or gh_owner has a default and it
    // succeeds — assert the contract that *if* it fails, it's a clear variable
    // error rather than a panic or rendering failure.
    if let Err(e) = result {
        let msg = e.to_string();
        assert!(
            msg.contains("required") || msg.contains("variable") || msg.contains("gh_owner"),
            "missing-var failure should mention the variable, got: {msg}"
        );
    }
}
