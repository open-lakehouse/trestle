//! The plan's complete rendered output, flattened to write-ready `(path, contents)` pairs
//! in one documented layout — [`MaterializedOutput`].
//!
//! [`render_all`] returns only the stack-aggregated
//! [`Artifacts`](crate::Artifacts) (the Envoy bootstrap, `.env`, and top-level compose); the
//! per-module fragments and their
//! mounted config files live separately on [`Plan::renders`](crate::Plan::renders). Every
//! consumer that writes a project therefore has to know the same on-disk layout — which
//! string goes to which path. [`Plan::materialize`](crate::Plan::materialize) encodes that
//! layout once.
//!
//! Building a [`MaterializedOutput`] is pure (no I/O), so it works in the browser too — a
//! consumer can iterate [`files`](MaterializedOutput::files) without touching disk. Writing
//! them is the one I/O step, behind [`write_to`](MaterializedOutput::write_to) and the
//! non-default `std-io` feature, so the default build stays WASM-clean.

use crate::plan::Plan;
use crate::render::artifacts::{ENVOY_CONFIG_PATH, render_all};

/// One file in a [`MaterializedOutput`]: a path relative to the project root and its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputFile {
    /// The path relative to the project root (e.g. `compose.yaml`,
    /// `modules/postgres/compose.yaml`).
    pub path: String,
    /// The file's contents.
    pub contents: String,
    /// Whether the file contains credentials and requires restricted permissions.
    pub sensitive: bool,
    /// Skip writing when the destination already exists (operator-owned files).
    pub preserve: bool,
}

/// Every file a [`Plan`] produces, flattened to write-ready `(path, contents)` pairs in a
/// fixed layout: the top-level `compose.yaml`, `.env`, and a human-readable `LAYOUT.md` summary
/// (see [`layout_report`](crate::layout_report)), the Envoy bootstrap at
/// `modules/envoy/envoy.yaml`, and each module's `modules/<id>/compose.yaml` fragment plus its
/// mounted config files (already rooted under `modules/<id>/`).
///
/// Build one with [`Plan::materialize`](crate::Plan::materialize). Pure and WASM-clean;
/// [`write_to`](Self::write_to) (behind the `std-io` feature) is the only step that does I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedOutput {
    /// The files to write, in a deterministic order: the top-level `compose.yaml`, `.env`, and
    /// `LAYOUT.md` first, then the Envoy bootstrap, then each module's fragment and config files
    /// in plan (dependency) order.
    pub files: Vec<OutputFile>,
}

/// Flatten a plan's full rendered output into the on-disk layout a consumer writes.
pub(crate) fn materialize(plan: &Plan) -> MaterializedOutput {
    let artifacts = render_all(plan);
    let mut files = vec![
        OutputFile {
            path: "compose.yaml".into(),
            contents: artifacts.compose,
            sensitive: false,
            preserve: false,
        },
        OutputFile {
            path: ".env".into(),
            contents: artifacts.env,
            sensitive: true,
            preserve: false,
        },
        OutputFile {
            path: ".gitignore".into(),
            contents: artifacts.gitignore,
            sensitive: false,
            preserve: false,
        },
        // A human-readable, at-a-glance summary of the gateway layout (routes → services).
        // Pure to build (see `report::layout_report`), so it keeps `materialize` pure.
        OutputFile {
            path: "LAYOUT.md".into(),
            contents: crate::report::layout_report(plan),
            sensitive: false,
            preserve: false,
        },
    ];

    // The Envoy bootstrap is written only when the plan has a gateway — the same condition
    // under which `render_all` declares the `envoy_config` mount. A gateway-less plan would
    // otherwise get an orphan `modules/envoy/envoy.yaml` that the top-level compose never
    // mounts. Detect the gateway by role (not module-id), matching `render_all`.
    let gateway_present = plan
        .services
        .values()
        .flatten()
        .any(|s| s.role == crate::Role::gateway());
    if gateway_present {
        files.push(OutputFile {
            path: ENVOY_CONFIG_PATH.into(),
            contents: artifacts.envoy,
            sensitive: false,
            preserve: false,
        });
    }

    // Each module owns a `modules/<id>/` directory: its compose fragment (skipped when empty)
    // plus any config files it emits (their `path` is already rooted under that directory,
    // or at the environment root when marked `at_root`).
    for (module, out) in &plan.renders {
        if !out.fragment.trim().is_empty() {
            files.push(OutputFile {
                path: format!("modules/{module}/compose.yaml"),
                contents: out.fragment.clone(),
                sensitive: false,
                preserve: false,
            });
        }
        for file in &out.files {
            files.push(OutputFile {
                path: file.path.clone(),
                contents: file.contents.clone(),
                sensitive: file.sensitive,
                preserve: file.preserve,
            });
        }
    }

    MaterializedOutput { files }
}

#[cfg(feature = "std-io")]
impl MaterializedOutput {
    /// Write every file under `dir`, creating parent directories as needed.
    ///
    /// The one I/O entry point in the crate, gated behind the `std-io` feature so the default
    /// build stays pure and WASM-clean. Each file's `path` is joined onto `dir`.
    pub fn write_to(&self, dir: &std::path::Path) -> std::io::Result<()> {
        for file in &self.files {
            let path = dir.join(&file.path);
            // Operator-owned files are seeded once and left alone thereafter so local edits
            // (e.g. Authelia users) survive subsequent renders.
            if file.preserve && path.exists() {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &file.contents)?;
            #[cfg(unix)]
            if file.sensitive {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "std-io"))]
mod tests {
    use crate::PlanCtx;
    use crate::catalog::baseline::baseline_catalog;
    use crate::plan::Selection;

    #[test]
    fn write_to_lands_every_file_under_the_dir() {
        let plan = baseline_catalog()
            .plan(
                &Selection::modules(["envoy", "postgres", "seaweedfs", "unity-catalog", "mlflow"]),
                &PlanCtx::default(),
            )
            .unwrap();
        let out = plan.materialize();

        let dir = std::env::temp_dir().join(format!("stack-topology-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        out.write_to(&dir).unwrap();

        for file in &out.files {
            let written = std::fs::read_to_string(dir.join(&file.path)).unwrap_or_else(|e| {
                panic!("expected {} on disk: {e}", file.path);
            });
            assert_eq!(
                written, file.contents,
                "contents mismatch for {}",
                file.path
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_to_preserves_existing_operator_owned_files() {
        // Force auth on so Authelia (and its root users.yml) is in the graph.
        let mut selection = Selection::modules(["envoy", "postgres"]);
        selection
            .knob_overrides
            .entry("envoy".into())
            .or_default()
            .insert("auth".into(), "true".into());
        let plan = baseline_catalog()
            .plan(
                &selection,
                &PlanCtx {
                    env_name: "auth-preserve".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let out = plan.materialize();
        assert!(
            out.files
                .iter()
                .any(|f| f.path == "users.yml" && f.preserve),
            "Authelia users.yml should be a preserved root file"
        );

        let dir =
            std::env::temp_dir().join(format!("stack-topology-preserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        out.write_to(&dir).unwrap();

        let users = dir.join("users.yml");
        assert!(users.is_file());
        std::fs::write(&users, "users:\n  edited: {}\n").unwrap();

        // Re-write: preserved files must keep the local edit.
        out.write_to(&dir).unwrap();
        let after = std::fs::read_to_string(&users).unwrap();
        assert_eq!(after, "users:\n  edited: {}\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
