//! End-to-end render of a Lakehouse dev environment, for eyeballing the materialized
//! artifacts a selection produces. This is the manual counterpart to the golden tests:
//! it plans the baseline catalog and writes every artifact (the top-level `compose.yaml`
//! and `.env`, the Envoy bootstrap, and each module's `modules/<id>/` directory holding its
//! compose fragment and mounted config files) to a scratch folder at the repository root,
//! printing a summary of what it wrote to stdout.
//!
//! Run it:
//!
//! ```text
//! cargo run -p olai-stack-topology --example render_stack
//! # pick your own modules:
//! cargo run -p olai-stack-topology --example render_stack -- envoy postgres seaweedfs jaeger
//! # prefer Azurite over SeaweedFS for the object_store role:
//! cargo run -p olai-stack-topology --example render_stack -- --azurite
//! # render a second, non-conflicting env (own project name, own output dir, shifted ports):
//! cargo run -p olai-stack-topology --example render_stack -- --name lh-alt --port-base 9180
//! ```
//!
//! Module ids are the short module names (`envoy`, `postgres`, `mlflow`, …).
//!
//! `--name <env>` sets the compose project name and the output subfolder; `--port-base <N>`
//! shifts the gateway's host ports (shared → N, dedicated base → N+20, admin → N+21) — the only
//! host-facing surface, since backends are network-only. Together they let two stacks run on one
//! host without colliding.
//!
//! Output lands in `<repo-root>/scratch/render_stack/` (or `render_stack_<name>/` for a
//! non-default `--name`), which is git-ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use olai_stack_topology::{ExtraResource, ModuleId, PlanCtx, Selection, baseline_catalog};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let prefer_azurite = args.iter().any(|a| a == "--azurite");

    // `--name <env>` sets the compose project name and the output subfolder, so two stacks
    // land in distinct directories. `--port-base <N>` shifts the gateway's host surface (the
    // sole host-facing ports) so a second env doesn't collide with the first: shared listener
    // → N, dedicated-listener base → N+20, Envoy admin → N+21. Without it the library defaults
    // apply. Backends never publish host ports, so nothing else needs shifting.
    let flag_value = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let env_name = flag_value("--name").unwrap_or_else(|| "lh-ref".to_string());
    // Only an explicit `--port-base` shifts the gateway surface; without it the example renders
    // the library defaults (shared 9080, dedicated 9100, admin 9901).
    let port_base: Option<u16> =
        flag_value("--port-base").map(|v| v.parse().expect("--port-base must be a port number"));

    // Module ids from the CLI (the short module names, taken verbatim), or the default
    // lakehouse selection when none are given. A flag and the token right after a
    // value-taking flag (`--name`, `--port-base`) are not module picks.
    let value_flags = ["--name", "--port-base"];
    let mut skip_next = false;
    let picks: Vec<&str> = args
        .iter()
        .filter(|a| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if value_flags.contains(&a.as_str()) {
                skip_next = true; // the following token is this flag's value, not a module
                return false;
            }
            !a.starts_with("--")
        })
        .map(String::as_str)
        .collect();

    let mut selection = if picks.is_empty() {
        // The default lakehouse. Under `--azurite` the object store is left to MLflow/UC's
        // demands (resolved to Azurite via the preference below) instead of selecting
        // SeaweedFS directly — selecting both object_store providers without a pin is a
        // `ConflictingRoleProviders` error.
        let mut mods = vec!["envoy", "postgres"];
        if !prefer_azurite {
            mods.push("seaweedfs");
        }
        mods.extend(["unity-catalog", "mlflow", "headwaters"]);
        Selection::modules(mods)
    } else {
        Selection::modules(picks)
    };

    // Demonstrate environment-level extra resources: an extra database and bucket that no
    // module consumes, created by the chosen providers' init jobs for an external host process
    // to query (the bucket through the gateway, the database directly at Postgres).
    selection.extra_resources = vec![
        ExtraResource {
            resource: "relational_db".into(),
            name: "analytics".into(),
            provider: None,
        },
        ExtraResource {
            resource: "object_store".into(),
            name: "exports".into(),
            provider: None,
        },
    ];

    let mut provider_preference = BTreeMap::new();
    if prefer_azurite {
        // Keyed by the `object_store` role string (see `Role::OBJECT_STORE`).
        provider_preference.insert(
            "object_store".to_string(),
            vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
        );
    }

    let defaults = PlanCtx::default();
    let ctx = PlanCtx {
        env_name: env_name.clone(),
        provider_preference,
        gateway_host_port: port_base.unwrap_or(defaults.gateway_host_port),
        // `saturating_add` so a near-`u16::MAX` `--port-base` clamps instead of panicking; such
        // a base is nonsensical for a dev gateway, but it shouldn't crash the example.
        dedicated_listener_port_base: port_base
            .map_or(defaults.dedicated_listener_port_base, |n| {
                n.saturating_add(20)
            }),
        gateway_admin_port: port_base.map_or(defaults.gateway_admin_port, |n| n.saturating_add(21)),
        ..defaults
    };

    let plan = match baseline_catalog().plan(&selection, &ctx) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("planning failed: {e}");
            std::process::exit(1);
        }
    };

    // Flatten the whole plan — stack artifacts plus every module fragment and config file —
    // into the project's on-disk layout, then write it. `materialize()` is pure; `write_to`
    // (behind the `std-io` feature this example requires) is the single I/O step.
    let output = plan.materialize();

    // Write everything into a fresh scratch folder at the repo root. A non-default `--name`
    // gets its own subfolder so two stacks coexist on disk as well as on the host ports.
    let out_dir = scratch_dir(&env_name);
    if out_dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&out_dir)
    {
        eprintln!("failed to clear {}: {e}", out_dir.display());
        std::process::exit(1);
    }
    if let Err(e) = output.write_to(&out_dir) {
        eprintln!("failed to write artifacts: {e}");
        std::process::exit(1);
    }
    for file in &output.files {
        println!("  {} ({} bytes)", file.path, file.contents.len());
    }

    println!("Wrote rendered artifacts to {}", out_dir.display());
}

/// `<repo-root>/scratch/render_stack` for the default env, or `render_stack_<name>` for a
/// non-default `--name`, so two rendered stacks don't overwrite each other. The repo root is
/// two levels up from this crate's manifest dir (`crates/stack-topology` → workspace root).
fn scratch_dir(env_name: &str) -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest_dir);
    let folder = if env_name == "lh-ref" {
        "render_stack".to_string()
    } else {
        format!("render_stack_{env_name}")
    };
    repo_root.join("scratch").join(folder)
}
