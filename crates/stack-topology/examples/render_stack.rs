//! End-to-end render of a Lakehouse dev environment, for eyeballing the materialized
//! artifacts a selection produces. This is the manual counterpart to the golden tests:
//! it plans the baseline catalog and prints every artifact (the four stack-aggregated
//! ones plus each module's compose fragment) to stdout.
//!
//! Run it:
//!
//! ```text
//! cargo run -p olai-stack-topology --example render_stack
//! # pick your own modules:
//! cargo run -p olai-stack-topology --example render_stack -- envoy postgres seaweedfs trino jaeger
//! # prefer Azurite over SeaweedFS for the object_store role:
//! cargo run -p olai-stack-topology --example render_stack -- --azurite
//! ```
//!
//! Module ids may be given with or without the `local-stack-` prefix.

use std::collections::BTreeMap;

use olai_stack_topology::{
    EnvoyOpts, ModuleId, PlanCtx, Selection, baseline_catalog, plan, render_all,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let prefer_azurite = args.iter().any(|a| a == "--azurite");

    // Module ids from the CLI (normalized to the `local-stack-` prefix), or the default
    // lakehouse selection when none are given.
    let picks: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| normalize_module_id(a))
        .collect();
    let selection = if picks.is_empty() {
        // The default lakehouse. Under `--azurite` the object store is left to MLflow/UC's
        // demands (resolved to Azurite via the preference below) instead of selecting
        // SeaweedFS directly — selecting both object_store providers without a pin is a
        // `ConflictingRoleProviders` error.
        let mut mods = vec!["local-stack-envoy", "local-stack-postgres"];
        if !prefer_azurite {
            mods.push("local-stack-seaweedfs");
        }
        mods.extend(["local-stack-unity-catalog", "local-stack-mlflow"]);
        Selection::modules(mods)
    } else {
        Selection::modules(picks)
    };

    let mut provider_preference = BTreeMap::new();
    if prefer_azurite {
        // Keyed by the `object_store` role string (see `Role::OBJECT_STORE`).
        provider_preference.insert(
            "object_store".to_string(),
            vec![
                ModuleId::from("local-stack-azurite"),
                ModuleId::from("local-stack-seaweedfs"),
            ],
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

    section("docker/envoy/envoy.yaml", &artifacts.envoy);
    section("compose.yaml", &artifacts.compose);
    section(".env", &artifacts.env);
    section(
        "docker/postgres/init-databases.sh",
        &artifacts.postgres_init,
    );

    for (module, out) in &plan.renders {
        if out.fragment.trim().is_empty() {
            continue;
        }
        section(&format!("fragment: {module}"), &out.fragment);
        for file in &out.files {
            section(
                &format!("  mounted file: {} ({module})", file.path),
                &file.contents,
            );
        }
    }
}

/// Accept a bare module name (`postgres`) or a full id (`local-stack-postgres`); the
/// `databricks-emulator-env` module is the one id without the prefix.
fn normalize_module_id(arg: &str) -> String {
    if arg.starts_with("local-stack-") || arg == "databricks-emulator-env" {
        arg.to_string()
    } else {
        format!("local-stack-{arg}")
    }
}

/// Print a labeled artifact block.
fn section(title: &str, body: &str) {
    println!("\n===== {title} =====");
    print!("{body}");
    if !body.ends_with('\n') {
        println!();
    }
}
