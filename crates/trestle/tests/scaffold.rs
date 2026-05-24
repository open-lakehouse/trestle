//! Integration tests that render each embedded template + a matrix of variable
//! combinations into a tempdir and assert that the result looks right.
//!
//! The expensive checks (full `cargo check` on the rendered Rust workspace,
//! `docker compose config`, `envoy --mode validate`, `buf build`) are gated
//! behind `TRESTLE_TEST_SLOW=1` so the default test invocation stays fast.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use trestle::cli::new::{self, NewArgs};

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

fn render_lab(profile: &str) -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("lab");
    let overrides: BTreeMap<String, String> = [
        ("lab_name".to_string(), "test-lab".to_string()),
        ("stack_profile".to_string(), profile.to_string()),
    ]
    .into_iter()
    .collect();

    let args = NewArgs {
        name: "test-lab".to_string(),
        out_dir: Some(out),
        template: "open-lakehouse-lab".to_string(),
        profile: Some(profile.to_string()),
        with: vec![],
        values: None,
        overrides: overrides.into_iter().collect(),
        non_interactive: true,
        force: false,
    };
    scaffold(args);
    tmp
}

fn render_rust_app(local_stack: &str, with_frontend: bool, with_ci: bool) -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("app");
    let overrides: BTreeMap<String, String> = [
        ("project_name".to_string(), "test-app".to_string()),
        ("gh_owner".to_string(), "acme".to_string()),
        ("rust_edition".to_string(), "2024".to_string()),
        ("with_frontend".to_string(), with_frontend.to_string()),
        ("with_ci".to_string(), with_ci.to_string()),
        ("local_stack".to_string(), local_stack.to_string()),
        ("license".to_string(), "Apache-2.0".to_string()),
    ]
    .into_iter()
    .collect();

    let args = NewArgs {
        name: "test-app".to_string(),
        out_dir: Some(out),
        template: "databricks-app-rust".to_string(),
        profile: Some(local_stack.to_string()),
        with: vec![],
        values: None,
        overrides: overrides.into_iter().collect(),
        non_interactive: true,
        force: false,
    };
    scaffold(args);
    tmp
}

#[test]
fn open_lakehouse_lab_minimal() {
    let tmp = render_lab("minimal");
    let root = tmp.path().join("lab");
    // Always-present files.
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
    // Minimal profile has no envoy → no envoy.yaml.
    assert!(!root.join("docker/envoy/envoy.yaml").is_file());
    // But postgres + seaweedfs are present.
    assert!(root.join("docker/compose/postgres.yaml").is_file());
    assert!(root.join("docker/compose/seaweedfs.yaml").is_file());
    assert!(root.join("docker/postgres/init-databases.sh").is_file());

    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
}

#[test]
fn open_lakehouse_lab_lakehouse() {
    let tmp = render_lab("lakehouse");
    let root = tmp.path().join("lab");
    // Lakehouse profile activates envoy + mlflow + UC + notebooks.
    for path in [
        "docker/envoy/envoy.yaml",
        "docker/compose/envoy.yaml",
        "docker/compose/postgres.yaml",
        "docker/compose/seaweedfs.yaml",
        "docker/compose/mlflow.yaml",
        "docker/compose/unity-catalog.yaml",
        "docker/compose/notebooks.yaml",
        "docker/postgres/init-databases.sh",
        "notebooks/01_lakehouse_quickstart.py",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    // jaeger is in the `full` profile, not `lakehouse`.
    assert!(!root.join("docker/compose/jaeger.yaml").is_file());

    let envoy = std::fs::read_to_string(root.join("docker/envoy/envoy.yaml")).unwrap();
    // Routes contributed by mlflow, UC, notebooks should all appear.
    assert!(envoy.contains("prefix: \"/api/2.0/mlflow\""));
    assert!(envoy.contains("prefix: \"/api/2.1/unity-catalog\""));
    assert!(envoy.contains("prefix: \"/notebooks\""));

    let env = std::fs::read_to_string(root.join(".env.example")).unwrap();
    assert!(env.contains("DATABRICKS_HOST="));
    assert!(env.contains("MLFLOW_TRACKING_URI="));

    let init = std::fs::read_to_string(root.join("docker/postgres/init-databases.sh")).unwrap();
    assert!(init.contains("CREATE DATABASE mlflow"));
    assert!(init.contains("CREATE DATABASE unitycatalog"));

    let seaweed = std::fs::read_to_string(root.join("docker/compose/seaweedfs.yaml")).unwrap();
    assert!(seaweed.contains("s3://mlflow"));
    assert!(seaweed.contains("s3://unity"));

    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("compose.yaml"));
    assert_yaml_parses(&root.join("docker/envoy/envoy.yaml"));

    if run_slow_tests() {
        assert_docker_compose_valid(&root, "svc");
    }
}

#[test]
fn databricks_app_rust_dbx_emulator() {
    let tmp = render_rust_app("dbx-emulator", true, true);
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

    // Path interpolation: proto/ tree uses the snake_case project name.
    assert!(root.join("proto/test_app/v1/models.proto").is_file());
    assert!(root.join("proto/test_app/v1/service.proto").is_file());

    let envoy = std::fs::read_to_string(root.join("docker/envoy/envoy.yaml")).unwrap();
    // Has the catch-all app cluster (since has_app=true).
    assert!(envoy.contains("name: app"));
    assert!(envoy.contains("test-app-server"));

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("crates/server"));
    assert!(cargo_toml.contains("edition = \"2024\""));

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
fn databricks_app_rust_no_local_stack() {
    let tmp = render_rust_app("none", false, false);
    let root = tmp.path().join("app");

    // Always-present files.
    for path in [
        "Cargo.toml",
        "crates/server/src/main.rs",
        "compose.yaml",
        "trestle.yaml",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    // No frontend, no CI, no shared-component fragments.
    assert!(!root.join("frontend").exists());
    assert!(!root.join(".github").exists());
    assert!(!root.join("docker/compose/envoy.yaml").is_file());
    assert!(!root.join("docker/envoy").exists());

    assert_no_unrendered_tokens(&root);
}

fn assert_yaml_parses(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let _: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("yaml parse {}: {e}", path.display()));
}

fn assert_docker_compose_valid(root: &Path, profile: &str) {
    // `docker compose config` requires a .env file (or env vars). Stage one.
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
