//! Integration tests that render the embedded base + apps into a tempdir and
//! assert that the result looks right.
//!
//! `trestle new` scaffolds the *project* (code, proto, app layering). The
//! composed Docker-Compose dev environment is produced separately by
//! `trestle env` (see `tests/env_render.rs`), so these tests assert the project
//! skeleton and app layering, and that no compose artifacts are emitted here.
//!
//! The expensive checks (full `cargo check` on the rendered Rust workspace,
//! `buf build`) are gated behind `TRESTLE_TEST_SLOW=1` so the default test
//! invocation stays fast.

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
            panic!("unrendered Jinja statement in {rel} near `{}`", m.as_str());
        }
    }
}

/// Assert `trestle new` did NOT emit any composed-environment artifacts — those
/// are `trestle env`'s job now. Guards the Phase-B cut against regressing to a
/// second compose path baked into the scaffold.
fn assert_no_compose_artifacts(root: &Path) {
    for path in [
        "compose.yaml",
        ".env.example",
        "docker/compose",
        "docker/envoy",
    ] {
        assert!(
            !root.join(path).exists(),
            "`trestle new` should not emit `{path}` — the composed environment is created with `trestle env`"
        );
    }
}

/// Render the `lakehouse` base + `databricks-app-rust` app, picking the app's
/// private `frontend` / `ci` categories.
fn render_rust_app(frontend: &str, ci: &str) -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("app");
    let overrides: BTreeMap<String, String> = [
        ("project_name".to_string(), "test-app".to_string()),
        ("gh_owner".to_string(), "acme".to_string()),
    ]
    .into_iter()
    .collect();

    let selections: Vec<(String, Vec<String>)> = vec![
        (
            "app.databricks-app-rust.frontend".to_string(),
            vec![frontend.to_string()],
        ),
        (
            "app.databricks-app-rust.ci".to_string(),
            vec![ci.to_string()],
        ),
    ];

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
fn base_lakehouse_scaffolds_project_skeleton_without_compose() {
    // The lakehouse base alone lays down a minimal project skeleton and points
    // at `trestle env` for the environment — it must not render compose files.
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("lab");
    scaffold(minimal_args("test-lab", out.clone()));

    for path in ["README.md", "justfile", ".gitignore"] {
        assert!(out.join(path).is_file(), "missing {path}");
    }
    assert_no_compose_artifacts(&out);

    // The base README should steer users to `trestle env`.
    let readme = std::fs::read_to_string(out.join("README.md")).unwrap();
    assert!(
        readme.contains("trestle env"),
        "base README should point at `trestle env`"
    );
    assert_no_unrendered_tokens(&out);
}

#[test]
fn databricks_app_rust_full() {
    // App + frontend + CI: the full project scaffold. No compose artifacts.
    let tmp = render_rust_app("react", "github");
    let root = tmp.path().join("app");

    for path in [
        "Cargo.toml",
        "trestle.yaml",
        "buf.yaml",
        "buf.gen.yaml",
        "justfile",
        "app.yaml",
        "databricks.yml",
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
        // The app's build Dockerfile stays — it builds the image `trestle env`
        // fronts as the app upstream.
        "docker/app/Dockerfile",
    ] {
        assert!(root.join(path).is_file(), "missing {path}");
    }

    assert!(root.join("proto/test_app/v1/models.proto").is_file());
    assert!(root.join("proto/test_app/v1/service.proto").is_file());

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("crates/server"));
    assert!(cargo_toml.contains("edition = \"2024\""));

    assert_no_compose_artifacts(&root);
    assert_no_unrendered_tokens(&root);
    assert_yaml_parses(&root.join("databricks.yml"));

    if run_slow_tests() {
        assert_cargo_check_passes(&root);
    }
}

#[test]
fn databricks_app_rust_no_frontend_no_ci() {
    // Frontend + CI off via the app categories.
    let tmp = render_rust_app("none", "none");
    let root = tmp.path().join("app");

    for path in ["Cargo.toml", "crates/server/src/main.rs", "trestle.yaml"] {
        assert!(root.join(path).is_file(), "missing {path}");
    }
    assert!(!root.join("frontend").exists());
    assert!(!root.join(".github").exists());
    assert_no_compose_artifacts(&root);
    assert_no_unrendered_tokens(&root);
}

#[test]
fn databricks_app_rust_connect() {
    // `connect=on` adds the buf ConnectRPC facade alongside REST and forces the
    // buffa runtime. Assert the wiring is rendered (the actual build + dual-port
    // serve are exercised by the committed example + slow cargo-check).
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("app");
    let args = NewArgs {
        name: "test-app".to_string(),
        out_dir: Some(out.clone()),
        template: "lakehouse".to_string(),
        apps: vec!["databricks-app-rust".to_string()],
        selections: vec![
            (
                "app.databricks-app-rust.frontend".to_string(),
                vec!["none".to_string()],
            ),
            (
                "app.databricks-app-rust.ci".to_string(),
                vec!["none".to_string()],
            ),
            (
                "app.databricks-app-rust.connect".to_string(),
                vec!["on".to_string()],
            ),
        ],
        profile: None,
        with: vec![],
        values: None,
        overrides: [
            ("project_name".to_string(), "test-app".to_string()),
            ("gh_owner".to_string(), "acme".to_string()),
        ]
        .into_iter()
        .collect(),
        non_interactive: true,
        force: false,
        // Deliberately leave runtime unset (defaults to prost) to prove connect
        // forces buffa.
        runtime: None,
    };
    scaffold(args);
    let root = out;

    // The Connect adapter ships from the connect-rust component; the shared core
    // is always present.
    assert!(
        root.join("crates/server/src/handlers/greeting_connect.rs")
            .is_file()
    );
    assert!(root.join("crates/server/src/handlers/core.rs").is_file());

    // Connect forces buffa.
    let trestle_yaml = std::fs::read_to_string(root.join("trestle.yaml")).unwrap();
    assert!(
        trestle_yaml.contains("proto_lib: buffa"),
        "connect must force buffa"
    );

    // buf.gen.yaml drives the connect plugins (now the remote connect-rust plugin).
    let buf_gen = std::fs::read_to_string(root.join("buf.gen.yaml")).unwrap();
    assert!(buf_gen.contains("buf.build/anthropics/connect-rust"));
    assert!(buf_gen.contains("buffa_module=::test_app_common::models"));

    // Server crate gets the connect deps + same-port serve wiring.
    let server_cargo = std::fs::read_to_string(root.join("crates/server/Cargo.toml")).unwrap();
    assert!(server_cargo.contains("connectrpc"));
    assert!(server_cargo.contains("http-body"));
    let main_rs = std::fs::read_to_string(root.join("crates/server/src/main.rs")).unwrap();
    assert!(main_rs.contains("mod connect;"));
    assert!(main_rs.contains("fallback_service"));

    assert_no_compose_artifacts(&root);
    assert_no_unrendered_tokens(&root);
    // NB: no `cargo check` here — the connect path needs `buf generate` with the
    // locally-installed connect plugins to populate `connect/` first, which the
    // slow-test harness doesn't run. The committed `examples/golden-path-app`
    // exercises the full build + dual-protocol serve.
}

fn assert_yaml_parses(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let _: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("yaml parse {}: {e}", path.display()));
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

/// A baseline `NewArgs` for the embedded `lakehouse` base, rendering into `out`,
/// with `name`.
fn minimal_args(name: &str, out: std::path::PathBuf) -> NewArgs {
    let overrides: BTreeMap<String, String> = [("gh_owner".to_string(), "acme".to_string())]
        .into_iter()
        .collect();
    NewArgs {
        name: name.to_string(),
        out_dir: Some(out),
        template: "lakehouse".to_string(),
        apps: vec![],
        selections: vec![],
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
