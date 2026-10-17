//! The target-neutral, write-ready output of rendering a plan — [`MaterializedOutput`], a flat
//! list of `(path, contents)` pairs — plus the client environment files every target writes.
//!
//! A deploy target (today: [`compose`](crate::render::compose)) owns the on-disk layout and
//! fills a [`MaterializedOutput`] in it, so every consumer that writes a project shares one
//! layout instead of re-deriving which string goes to which path.
//!
//! Building a [`MaterializedOutput`] is pure (no I/O), so it works in the browser too — a
//! consumer can iterate [`files`](MaterializedOutput::files) without touching disk. Writing
//! them is the one I/O step, behind [`write_to`](MaterializedOutput::write_to) and the
//! non-default `std-io` feature, so the default build stays WASM-clean.

use std::fmt::Write as _;

use crate::render::InjectedEnv;
use crate::render::deploy::DeployStep;

/// Where the host-vantage client environment is written (see
/// [`Plan::client_env`](crate::Plan::client_env)).
pub const CLIENT_HOST_ENV_PATH: &str = "client/host.env";
/// Where the in-network (container-vantage) client environment is written.
pub const CLIENT_CONTAINER_ENV_PATH: &str = "client/container.env";

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
    /// For a `preserve`d file, the path older renders wrote it to: an existing file there is
    /// moved to `path` instead of seeding the default (see [`RenderFile::moved_from`]).
    ///
    /// [`RenderFile::moved_from`]: crate::render::RenderFile::moved_from
    pub moved_from: Option<String>,
}

/// Every file a rendered [`Plan`](crate::Plan) produces, flattened to write-ready
/// `(path, contents)` pairs in the deploy target's layout. For Docker Compose
/// ([`compose::materialize`](crate::render::compose::materialize)) that is the top-level
/// `compose.yaml`, `.env`, the per-vantage client environments (`client/host.env`,
/// `client/container.env`), a human-readable `LAYOUT.md` summary (see
/// [`layout_report`](crate::layout_report)), the Envoy bootstrap at `modules/envoy/envoy.yaml`,
/// and each module's `modules/<id>/compose.yaml` fragment plus its mounted config files
/// (already rooted under `modules/<id>/`). When the selection asks for it, the Python client
/// workspace follows under `client/` (see
/// [`python_client_files`](crate::render::python_client::python_client_files)).
///
/// Build one with [`Plan::materialize`](crate::Plan::materialize). Pure and WASM-clean;
/// [`write_to`](Self::write_to) (behind the `std-io` feature) is the only step that does I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedOutput {
    /// The files to write, in a deterministic order: the top-level `compose.yaml`, `.env`, and
    /// `LAYOUT.md` first, then the Envoy bootstrap, then each module's fragment and config files
    /// in plan (dependency) order.
    pub files: Vec<OutputFile>,
    /// The environment's deployment steps, in order (see
    /// [`render_steps`](crate::render::deploy::render_steps)). Their README and scripts are
    /// among [`files`](Self::files) too.
    pub deploy: Vec<DeployStep>,
}

/// Render a client environment file (see [`Plan::client_env`](crate::Plan::client_env)):
/// `KEY=value` lines in key order under a header naming the vantage the addresses are for.
pub(crate) fn render_client_env(env: &InjectedEnv, vantage: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Client environment for callers {vantage}. Generated; re-render to update."
    );
    for line in [
        "# Load it into an app, notebook, or engine to reach this stack. It holds no",
        "# credentials: authenticate through the gateway, and read/write object storage",
        "# with the temporary credentials Unity Catalog vends.",
    ] {
        out.push_str(line);
        out.push('\n');
    }
    for (k, v) in env.iter() {
        let _ = writeln!(out, "{k}={v}");
    }
    out
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
            // A relocated operator-owned file: carry the operator's copy over from where an
            // older render left it, rather than seeding the default over it.
            if file.preserve
                && let Some(old) = file.moved_from.as_deref().map(|old| dir.join(old))
                && old.is_file()
            {
                std::fs::rename(&old, &path)?;
                continue;
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
                &Selection::modules(["envoy", "postgres", "rustfs", "unity-catalog", "mlflow"]),
                &PlanCtx::default(),
            )
            .unwrap();
        let out = plan.materialize().unwrap();

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

    /// A plan with sign-in on, so Authelia (and its operator-owned users.yml) is rendered.
    fn authenticated_output(env_name: &str) -> super::MaterializedOutput {
        let mut selection = Selection::modules(["envoy", "postgres"]);
        selection
            .knob_overrides
            .entry("envoy".into())
            .or_default()
            .insert("auth".into(), "true".into());
        baseline_catalog()
            .plan(
                &selection,
                &PlanCtx {
                    env_name: env_name.into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .materialize()
            .unwrap()
    }

    #[test]
    fn write_to_preserves_existing_operator_owned_files() {
        let out = authenticated_output("auth-preserve");
        assert!(
            out.files
                .iter()
                .any(|f| f.path == "modules/authelia/users.yml" && f.preserve),
            "Authelia users.yml should be a preserved module file"
        );

        let dir =
            std::env::temp_dir().join(format!("stack-topology-preserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        out.write_to(&dir).unwrap();

        let users = dir.join("modules/authelia/users.yml");
        assert!(users.is_file());
        std::fs::write(&users, "users:\n  edited: {}\n").unwrap();

        // Re-write: preserved files must keep the local edit.
        out.write_to(&dir).unwrap();
        let after = std::fs::read_to_string(&users).unwrap();
        assert_eq!(after, "users:\n  edited: {}\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_to_moves_a_relocated_operator_file_into_place() {
        let out = authenticated_output("auth-moved");
        let dir = std::env::temp_dir().join(format!("stack-topology-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // An environment rendered before users.yml moved into modules/authelia/.
        std::fs::write(dir.join("users.yml"), "users:\n  edited: {}\n").unwrap();

        out.write_to(&dir).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("modules/authelia/users.yml")).unwrap(),
            "users:\n  edited: {}\n",
            "the operator's accounts move to the new path instead of the demo default"
        );
        assert!(
            !dir.join("users.yml").exists(),
            "the old copy is moved, not left behind"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
