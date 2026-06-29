//! End-to-end render of a Lakehouse dev environment, for eyeballing the materialized
//! artifacts a selection produces. This is the manual counterpart to the golden tests:
//! it plans the baseline catalog and writes every artifact (the four stack-aggregated
//! ones plus each module's compose fragment and mounted files) to a scratch folder at
//! the repository root, printing a summary of what it wrote to stdout.
//!
//! Run it:
//!
//! ```text
//! cargo run -p olai-stack-topology --example render_stack
//! # pick your own modules:
//! cargo run -p olai-stack-topology --example render_stack -- envoy postgres seaweedfs jaeger
//! # prefer Azurite over SeaweedFS for the object_store role:
//! cargo run -p olai-stack-topology --example render_stack -- --azurite
//! ```
//!
//! Module ids are the short module names (`envoy`, `postgres`, `mlflow`, …).
//!
//! Output lands in `<repo-root>/scratch/render_stack/`, which is git-ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use olai_stack_topology::{
    EnvoyOpts, ModuleId, PlanCtx, Selection, baseline_catalog, plan, render_all,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let prefer_azurite = args.iter().any(|a| a == "--azurite");

    // Module ids from the CLI (the short module names, taken verbatim), or the default
    // lakehouse selection when none are given.
    let picks: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let selection = if picks.is_empty() {
        // The default lakehouse. Under `--azurite` the object store is left to MLflow/UC's
        // demands (resolved to Azurite via the preference below) instead of selecting
        // SeaweedFS directly — selecting both object_store providers without a pin is a
        // `ConflictingRoleProviders` error.
        let mut mods = vec!["envoy", "postgres"];
        if !prefer_azurite {
            mods.push("seaweedfs");
        }
        mods.extend(["unity-catalog", "mlflow"]);
        Selection::modules(mods)
    } else {
        Selection::modules(picks)
    };

    let mut provider_preference = BTreeMap::new();
    if prefer_azurite {
        // Keyed by the `object_store` role string (see `Role::OBJECT_STORE`).
        provider_preference.insert(
            "object_store".to_string(),
            vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
        );
    }

    let ctx = PlanCtx {
        env_name: "lh-ref".into(),
        provider_preference,
        ..Default::default()
    };

    let plan = match plan(&selection, &baseline_catalog(), &ctx) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("planning failed: {e}");
            std::process::exit(1);
        }
    };

    let artifacts = render_all(&plan, &EnvoyOpts::default());

    // Write everything into a fresh scratch folder at the repo root.
    let out_dir = scratch_dir();
    if out_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&out_dir) {
            eprintln!("failed to clear {}: {e}", out_dir.display());
            std::process::exit(1);
        }
    }

    write_artifact(&out_dir, "docker/envoy/envoy.yaml", &artifacts.envoy);
    write_artifact(&out_dir, "compose.yaml", &artifacts.compose);
    write_artifact(&out_dir, ".env", &artifacts.env);
    write_artifact(
        &out_dir,
        "docker/postgres/init-databases.sh",
        &artifacts.postgres_init,
    );

    for (module, out) in &plan.renders {
        if !out.fragment.trim().is_empty() {
            write_artifact(
                &out_dir,
                &format!("fragments/{module}.compose.yaml"),
                &out.fragment,
            );
        }
        for file in &out.files {
            write_artifact(&out_dir, &file.path, &file.contents);
        }
    }

    println!("Wrote rendered artifacts to {}", out_dir.display());
}

/// `<repo-root>/scratch/render_stack`. The repo root is two levels up from this
/// crate's manifest dir (`crates/stack-topology` → workspace root).
fn scratch_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest_dir);
    repo_root.join("scratch").join("render_stack")
}

/// Write `body` to `out_dir/rel_path`, creating parent directories, and log it.
fn write_artifact(out_dir: &Path, rel_path: &str, body: &str) {
    let path = out_dir.join(rel_path);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("failed to create {}: {e}", parent.display());
            std::process::exit(1);
        }
    }
    if let Err(e) = std::fs::write(&path, body) {
        eprintln!("failed to write {}: {e}", path.display());
        std::process::exit(1);
    }
    println!("  {rel_path} ({} bytes)", body.len());
}
