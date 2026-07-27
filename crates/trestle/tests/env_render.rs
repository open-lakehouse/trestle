//! End-to-end check of the `trestle env` render pipeline: plan the baseline
//! catalog, materialize + persist to a tempdir exactly as `trestle env new` does,
//! then re-render from the persisted `env.toml` and assert byte-stable output.
//!
//! Scenario fixtures under `test/scenarios/` are rendered and compared against
//! golden artifacts in `test/expected/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use olai_stack_topology::{EnvManifest, ModuleId, PlanCtx, Selection, baseline_catalog};
use walkdir::WalkDir;

/// Plan + materialize + persist an env.toml to `dir` — the same effect as
/// `trestle env new --non-interactive` (the CLI's `write_env`).
fn write_env(selection: &Selection, ctx: &PlanCtx, dir: &Path) {
    let catalog = baseline_catalog();
    let plan = catalog.plan(selection, ctx).expect("plan should succeed");
    plan.materialize().write_to(dir).expect("write_to");
    EnvManifest::new(selection.clone(), ctx.clone())
        .write_to(&dir.join("env.toml"))
        .expect("manifest write");
}

/// Render from a persisted manifest (what `trestle env render` does).
fn render_manifest(manifest: &EnvManifest, dir: &Path) {
    let plan = manifest
        .plan(&baseline_catalog())
        .expect("plan should succeed");
    plan.materialize().write_to(dir).expect("write_to");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn scenarios_dir() -> PathBuf {
    repo_root().join("test/scenarios")
}

fn expected_dir() -> PathBuf {
    repo_root().join("test/expected")
}

/// Discover scenario names from `test/scenarios/*/env.toml`.
fn scenario_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(scenarios_dir())
        .expect("test/scenarios should exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("env.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Collect relative file paths under `root`, skipping directories.
fn collect_files(root: &Path) -> BTreeSet<String> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            e.path()
                .strip_prefix(root)
                .expect("path under root")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// Sensitive runtime files are generated and tested in temporary output, never committed.
fn is_sensitive_path(path: &str) -> bool {
    path == ".env" || path.contains("/.env/") || path.contains("/secrets/")
}

/// Byte-compare every non-sensitive file under `got` against `want`.
fn assert_tree_matches(got: &Path, want: &Path, label: &str) {
    let got_files: BTreeSet<_> = collect_files(got)
        .into_iter()
        .filter(|path| !is_sensitive_path(path))
        .collect();
    let want_files: BTreeSet<_> = collect_files(want)
        .into_iter()
        .filter(|path| !is_sensitive_path(path))
        .collect();
    assert_eq!(
        got_files, want_files,
        "{label}: rendered file set differs from golden"
    );
    for rel in &got_files {
        let got_bytes = fs::read(got.join(rel)).unwrap_or_else(|_| panic!("read {rel}"));
        let want_bytes = fs::read(want.join(rel)).unwrap_or_else(|_| panic!("read golden {rel}"));
        assert_eq!(
            got_bytes, want_bytes,
            "{label}: `{rel}` differs from golden"
        );
    }
}

/// Load a scenario manifest from `test/scenarios/<name>/env.toml`.
fn load_scenario(name: &str) -> EnvManifest {
    let path = scenarios_dir().join(name).join("env.toml");
    EnvManifest::read_from(&path).unwrap_or_else(|e| panic!("load scenario {name}: {e}"))
}

/// Render a scenario manifest into a tempdir.
fn render_scenario(name: &str, dir: &Path) {
    let manifest = load_scenario(name);
    fs::create_dir_all(dir).expect("create temp dir");
    fs::copy(
        scenarios_dir().join(name).join("env.toml"),
        dir.join("env.toml"),
    )
    .expect("copy env.toml");
    render_manifest(&manifest, dir);
}

#[test]
fn env_new_writes_the_expected_layout() {
    let dir = tempfile::tempdir().unwrap();
    let selection = Selection::modules(["envoy", "postgres", "mlflow", "seaweedfs"]);
    let ctx = PlanCtx {
        env_name: "lakehouse".into(),
        ..PlanCtx::default()
    };
    write_env(&selection, &ctx, dir.path());

    // Top-level artifacts + the persisted manifest.
    for f in [
        "compose.yaml",
        ".env",
        ".gitignore",
        "LAYOUT.md",
        "env.toml",
    ] {
        assert!(dir.path().join(f).is_file(), "expected {f} to be written");
    }
    // Per-module fragments land under modules/<id>/.
    for m in ["envoy", "postgres", "mlflow", "seaweedfs"] {
        assert!(
            dir.path()
                .join("modules")
                .join(m)
                .join("compose.yaml")
                .is_file(),
            "expected modules/{m}/compose.yaml"
        );
    }

    // The top-level compose parses as YAML.
    let compose = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();
    let _: serde_yaml::Value =
        serde_yaml::from_str(&compose).expect("compose.yaml should be valid YAML");
}

#[test]
fn env_render_is_byte_stable() {
    let dir = tempfile::tempdir().unwrap();
    let selection = Selection::modules(["envoy", "postgres", "mlflow", "seaweedfs"]);
    let ctx = PlanCtx {
        env_name: "lakehouse".into(),
        ..PlanCtx::default()
    };
    write_env(&selection, &ctx, dir.path());
    let first = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();

    // Re-render from the persisted manifest (what `trestle env render` does):
    // read env.toml, re-plan, materialize into the same dir.
    let manifest = EnvManifest::read_from(&dir.path().join("env.toml")).unwrap();
    render_manifest(&manifest, dir.path());
    let second = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();

    assert_eq!(first, second, "re-render should be byte-identical");
}

#[test]
fn knob_override_pulls_in_authelia() {
    // auth=true is a gateway knob that pulls the auth provider into the
    // graph — a good check that CLI-supplied knob overrides reach the planner.
    let dir = tempfile::tempdir().unwrap();
    let mut selection = Selection::modules(["envoy", "postgres"]);
    selection
        .knob_overrides
        .entry("envoy".into())
        .or_default()
        .insert("auth".into(), "true".into());
    write_env(&selection, &PlanCtx::default(), dir.path());

    assert!(
        dir.path()
            .join("modules")
            .join("authelia")
            .join("compose.yaml")
            .is_file(),
        "auth=true should pull authelia into the rendered stack"
    );
}

// ---------------------------------------------------------------------------
// Curated scenario fixtures
// ---------------------------------------------------------------------------

#[test]
fn scenario_fixtures_exist() {
    let names = scenario_names();
    assert!(
        names.len() >= 4,
        "expected at least 4 scenarios, got {:?}",
        names
    );
    for name in ["minimal", "lakehouse", "authenticated", "full-azure"] {
        assert!(names.contains(&name.to_string()), "missing scenario {name}");
    }
}

#[test]
fn scenario_renders_match_golden_fixtures() {
    for name in scenario_names() {
        let dir = tempfile::tempdir().unwrap();
        render_scenario(&name, dir.path());
        let golden = expected_dir().join(&name);
        assert!(
            golden.is_dir(),
            "missing golden fixture for scenario {name} — run `just env-golden-refresh`"
        );
        assert_tree_matches(dir.path(), &golden, &name);

        // Every rendered compose.yaml must be valid YAML.
        let compose = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();
        let _: serde_yaml::Value =
            serde_yaml::from_str(&compose).expect("compose.yaml should be valid YAML");
    }
}

#[test]
fn sensitive_runtime_files_are_generated_but_not_committed() {
    for name in ["lakehouse", "authenticated", "full-azure"] {
        let dir = tempfile::tempdir().unwrap();
        render_scenario(name, dir.path());

        let sensitive: Vec<_> = collect_files(dir.path())
            .into_iter()
            .filter(|path| is_sensitive_path(path))
            .collect();
        assert!(
            !sensitive.is_empty(),
            "{name} should generate sensitive files"
        );

        for path in &sensitive {
            assert!(
                !expected_dir().join(name).join(path).exists(),
                "{path} must not be committed as a golden"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = fs::metadata(dir.path().join(path))
                    .expect("sensitive metadata")
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "{path} should be owner-readable only");
            }
        }
    }
}

#[test]
fn compose_artifacts_reference_env_files_and_native_secrets_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("authenticated", dir.path());

    let root = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();
    assert!(root.contains("secrets:\n"));
    assert!(root.contains("postgres_password:"));
    assert!(root.contains("authelia_session_secret:"));

    let postgres = fs::read_to_string(dir.path().join("modules/postgres/compose.yaml")).unwrap();
    assert!(postgres.contains("format: raw"));
    assert!(postgres.contains("./modules/postgres/.env/db.env"));
    assert!(postgres.contains("- postgres_password"));

    let authelia = fs::read_to_string(dir.path().join("modules/authelia/compose.yaml")).unwrap();
    assert!(authelia.contains("./modules/authelia/.env/authelia.env"));
    assert!(authelia.contains("- authelia_jwt_secret"));

    for path in collect_files(dir.path())
        .into_iter()
        .filter(|path| !is_sensitive_path(path))
    {
        let contents = fs::read_to_string(dir.path().join(&path)).unwrap();
        for forbidden in [
            "postgresql://postgres:postgres@",
            "postgres://postgres:postgres@",
            "AccountKey=",
            "insecure_session_secret_change_me",
            "insecure_storage_encryption_key_change_me",
            "insecure_jwt_secret_change_me",
        ] {
            assert!(
                !contents.contains(forbidden),
                "{path} contains credential-bearing text `{forbidden}`"
            );
        }
    }
}

#[test]
fn generated_gitignore_covers_runtime_credentials_and_data() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("minimal", dir.path());
    let ignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    for rule in [".data/", ".env", "modules/*/.env/", "modules/*/secrets/"] {
        assert!(ignore.lines().any(|line| line == rule), "missing `{rule}`");
    }
}

#[test]
fn authenticated_scenario_pulls_in_authelia() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("authenticated", dir.path());
    assert!(
        dir.path().join("modules/authelia/compose.yaml").is_file(),
        "authenticated scenario should render authelia"
    );
    assert!(
        dir.path().join("users.yml").is_file(),
        "Authelia users.yml should live at the environment root"
    );
    let compose = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();
    assert!(
        compose.contains("file: ./users.yml"),
        "root compose should mount ./users.yml"
    );
    let envoy_compose = fs::read_to_string(dir.path().join("modules/envoy/compose.yaml")).unwrap();
    assert!(
        envoy_compose.contains("authelia:"),
        "envoy should depend on authelia when auth=true"
    );
}

#[test]
fn users_yml_survives_rerender() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("authenticated", dir.path());
    let users = dir.path().join("users.yml");
    fs::write(&users, "# edited by operator\nusers: {}\n").unwrap();

    let manifest = EnvManifest::read_from(&dir.path().join("env.toml")).unwrap();
    render_manifest(&manifest, dir.path());

    let after = fs::read_to_string(&users).unwrap();
    assert_eq!(
        after, "# edited by operator\nusers: {}\n",
        "operator edits to users.yml must survive re-render"
    );
}

#[test]
fn legacy_auth_alias_still_pulls_in_authelia() {
    // Old SCREAMING_SNAKE override keys remain accepted as aliases.
    let dir = tempfile::tempdir().unwrap();
    let mut selection = Selection::modules(["envoy", "postgres"]);
    selection
        .knob_overrides
        .entry("envoy".into())
        .or_default()
        .insert("ENVOY_AUTH".into(), "true".into());
    write_env(&selection, &PlanCtx::default(), dir.path());
    assert!(
        dir.path().join("modules/authelia/compose.yaml").is_file(),
        "legacy ENVOY_AUTH alias should still pull authelia"
    );
}

#[test]
fn full_azure_scenario_uses_azurite_not_seaweedfs() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("full-azure", dir.path());
    assert!(
        dir.path().join("modules/azurite/compose.yaml").is_file(),
        "full-azure should render azurite"
    );
    assert!(
        !dir.path().join("modules/seaweedfs").exists(),
        "full-azure should not render seaweedfs when azurite is preferred"
    );
    let compose = fs::read_to_string(dir.path().join("compose.yaml")).unwrap();
    assert!(
        compose.contains("azurite"),
        "top-level compose should include azurite"
    );
    assert!(
        !compose.contains("seaweedfs"),
        "top-level compose should not include seaweedfs"
    );
}

#[test]
fn image_defaults_are_baked_into_compose() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("minimal", dir.path());
    let postgres = fs::read_to_string(dir.path().join("modules/postgres/compose.yaml")).unwrap();
    assert!(
        postgres.contains("image: postgres:16"),
        "postgres image default should be baked at render time"
    );
    let envoy = fs::read_to_string(dir.path().join("modules/envoy/compose.yaml")).unwrap();
    assert!(
        envoy.contains("image: envoyproxy/envoy:v1.34-latest"),
        "envoy image default should be baked at render time"
    );
}

#[test]
fn image_override_survives_render_rerender() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = load_scenario("minimal");
    manifest.set_knob("postgres", "image", "postgres:17");
    fs::create_dir_all(dir.path()).unwrap();
    manifest
        .write_to(&dir.path().join("env.toml"))
        .expect("write manifest");
    render_manifest(&manifest, dir.path());

    let first = fs::read_to_string(dir.path().join("modules/postgres/compose.yaml")).unwrap();
    assert!(
        first.contains("image: postgres:17"),
        "image override should appear in rendered compose"
    );

    // Re-render from persisted manifest.
    let reloaded = EnvManifest::read_from(&dir.path().join("env.toml")).unwrap();
    render_manifest(&reloaded, dir.path());
    let second = fs::read_to_string(dir.path().join("modules/postgres/compose.yaml")).unwrap();
    assert_eq!(first, second, "image override should survive re-render");
}

#[test]
fn lakehouse_scenario_provisions_expected_databases() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("lakehouse", dir.path());
    let init = fs::read_to_string(dir.path().join("modules/postgres/init-databases.sh")).unwrap();
    for db in ["mlflow", "unitycatalog"] {
        assert!(
            init.contains(&format!("CREATE DATABASE {db}")),
            "lakehouse should provision database {db}"
        );
    }
}

#[test]
fn full_azure_scenario_includes_lineage_and_tracing_routes() {
    let dir = tempfile::tempdir().unwrap();
    render_scenario("full-azure", dir.path());
    let layout = fs::read_to_string(dir.path().join("LAYOUT.md")).unwrap();
    assert!(
        layout.contains("/jaeger"),
        "full-azure should front jaeger UI"
    );
    assert!(
        layout.contains("/lineage"),
        "full-azure should front headwaters UI"
    );
    assert!(
        layout.contains("/api/v1/lineage"),
        "full-azure should front headwaters API"
    );

    // Planner-selected modules are reflected in the render set.
    let plan = load_scenario("full-azure")
        .plan(&baseline_catalog())
        .expect("plan");
    let rendered: BTreeSet<ModuleId> = plan.renders.iter().map(|(id, _)| id.clone()).collect();
    for module in ["jaeger", "headwaters", "azurite"] {
        assert!(
            rendered.contains(&ModuleId::from(module)),
            "full-azure plan should render {module}"
        );
    }
}
