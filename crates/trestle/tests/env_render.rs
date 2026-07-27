//! End-to-end check of the `trestle env` render pipeline: plan the baseline
//! catalog, materialize + persist to a tempdir exactly as `trestle env new` does,
//! then re-render from the persisted `env.toml` and assert byte-stable output.
//!
//! This mirrors `olai-stack-topology`'s own `golden_lakehouse`/`render_stack`, but
//! exercises the layout the trestle CLI writes (compose.yaml, .env, LAYOUT.md,
//! env.toml, modules/<id>/...). It drives the engine directly rather than the
//! cliclack wizard so it needs no TTY.

use std::path::Path;

use olai_stack_topology::{EnvManifest, PlanCtx, Selection, baseline_catalog};

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
    for f in ["compose.yaml", ".env", "LAYOUT.md", "env.toml"] {
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
    let compose = std::fs::read_to_string(dir.path().join("compose.yaml")).unwrap();
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
    let first = std::fs::read_to_string(dir.path().join("compose.yaml")).unwrap();

    // Re-render from the persisted manifest (what `trestle env render` does):
    // read env.toml, re-plan, materialize into the same dir.
    let manifest = EnvManifest::read_from(&dir.path().join("env.toml")).unwrap();
    let plan = manifest.plan(&baseline_catalog()).unwrap();
    plan.materialize().write_to(dir.path()).unwrap();
    let second = std::fs::read_to_string(dir.path().join("compose.yaml")).unwrap();

    assert_eq!(first, second, "re-render should be byte-identical");
}

#[test]
fn knob_override_pulls_in_authelia() {
    // ENVOY_AUTH=true is a gateway knob that pulls the auth provider into the
    // graph — a good check that CLI-supplied knob overrides reach the planner.
    let dir = tempfile::tempdir().unwrap();
    let mut selection = Selection::modules(["envoy", "postgres"]);
    selection
        .knob_overrides
        .entry("envoy".into())
        .or_default()
        .insert("ENVOY_AUTH".into(), "true".into());
    write_env(&selection, &PlanCtx::default(), dir.path());

    assert!(
        dir.path()
            .join("modules")
            .join("authelia")
            .join("compose.yaml")
            .is_file(),
        "ENVOY_AUTH=true should pull authelia into the rendered stack"
    );
}
