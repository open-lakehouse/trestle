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
}

/// Every file a [`Plan`] produces, flattened to write-ready `(path, contents)` pairs in a
/// fixed layout: the top-level `compose.yaml` and `.env`, the Envoy bootstrap at
/// `modules/envoy/envoy.yaml`, and each module's `modules/<id>/compose.yaml` fragment plus its
/// mounted config files (already rooted under `modules/<id>/`).
///
/// Build one with [`Plan::materialize`](crate::Plan::materialize). Pure and WASM-clean;
/// [`write_to`](Self::write_to) (behind the `std-io` feature) is the only step that does I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedOutput {
    /// The files to write, in a deterministic order: the top-level `compose.yaml` and `.env`
    /// first, then the Envoy bootstrap, then each module's fragment and config files in plan
    /// (dependency) order.
    pub files: Vec<OutputFile>,
}

/// Flatten a plan's full rendered output into the on-disk layout a consumer writes.
pub(crate) fn materialize(plan: &Plan) -> MaterializedOutput {
    let artifacts = render_all(plan);
    let mut files = vec![
        OutputFile {
            path: "compose.yaml".into(),
            contents: artifacts.compose,
        },
        OutputFile {
            path: ".env".into(),
            contents: artifacts.env,
        },
        OutputFile {
            path: ENVOY_CONFIG_PATH.into(),
            contents: artifacts.envoy,
        },
    ];

    // Each module owns a `modules/<id>/` directory: its compose fragment (skipped when empty)
    // plus any config files it emits (their `path` is already rooted under that directory).
    for (module, out) in &plan.renders {
        if !out.fragment.trim().is_empty() {
            files.push(OutputFile {
                path: format!("modules/{module}/compose.yaml"),
                contents: out.fragment.clone(),
            });
        }
        for file in &out.files {
            files.push(OutputFile {
                path: file.path.clone(),
                contents: file.contents.clone(),
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
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &file.contents)?;
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
}
