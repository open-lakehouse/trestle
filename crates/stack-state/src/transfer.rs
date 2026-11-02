//! Import and export: `trestle-workspace.toml` and environment directories as an exchange
//! format for the store.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use olai_stack_topology::{EnvManifest, WORKSPACE_FILE, WorkspaceEnv, WorkspaceManifest};

use crate::error::{Error, Result};
use crate::model::{Draft, InstanceId, Workspace};
use crate::store::Store;

/// The environment manifest's file name inside an environment directory.
pub const ENV_FILE: &str = "env.toml";

impl Store {
    /// Import the workspace directory `dir` (a `trestle-workspace.toml` and one environment
    /// directory per member) as a new workspace. Each member becomes an environment whose
    /// draft is its `env.toml` and the module manifests it lists, committed as revision 1.
    ///
    /// Slots are not imported: the store leases them per instance, host-wide. Nor is
    /// `secrets.env`: the store holds secret references, not values.
    pub fn import_workspace(&self, dir: &Path) -> Result<Workspace> {
        let path = dir.join(WORKSPACE_FILE);
        let manifest = WorkspaceManifest::read_from(&path)?;
        let workspace = self.create_workspace(&manifest.name, manifest.mesh)?;
        let imported = (|| {
            for member in &manifest.envs {
                let draft = read_env_dir(&dir.join(&member.path))?;
                let env = self.create_environment(&workspace.id, &member.name, draft)?;
                self.commit_draft(&env.id)?;
            }
            Ok(())
        })();
        if let Err(e) = imported {
            // All or nothing: drop what the failed import created.
            let _ = self.delete_workspace(&workspace.id);
            return Err(e);
        }
        Ok(workspace)
    }

    /// Export `instances` (all of one workspace, each of a different environment) as a
    /// workspace directory in `dir`: a `trestle-workspace.toml` whose members sit on the slots
    /// the instances lease, and per member its revision's `env.toml` and module manifests. In
    /// the exported directory `trestle env render` and `docker compose up` work as in any
    /// workspace.
    pub fn export_instances(&self, instances: &[InstanceId], dir: &Path) -> Result<()> {
        let mut workspace: Option<WorkspaceManifest> = None;
        let mut members = Vec::new();
        for id in instances {
            let instance = self.instance(id)?;
            let env = self.environment(&instance.environment)?;
            let ws = self.workspace_by_id(&env.workspace)?;
            let manifest = workspace.get_or_insert_with(|| WorkspaceManifest {
                version: WorkspaceManifest::current_version(),
                name: ws.name.clone(),
                mesh: ws.mesh,
                envs: Vec::new(),
            });
            if manifest.name != ws.name {
                return Err(Error::Export(format!(
                    "the instances belong to two workspaces, `{}` and `{}`",
                    manifest.name, ws.name
                )));
            }
            let slot = instance.slot.ok_or_else(|| {
                Error::Export(format!("instance {id} holds no slot; lease one first"))
            })?;
            manifest.envs.push(WorkspaceEnv {
                name: env.name.clone(),
                path: env.name.clone(),
                slot,
            });
            members.push((
                env.name,
                self.revision(&instance.environment, instance.revision)?,
            ));
        }
        let Some(workspace) = workspace else {
            return Err(Error::Export("no instances given".into()));
        };
        // Two instances of one environment would be two members of one name.
        workspace.validate()?;

        for (name, revision) in &members {
            let env_dir = dir.join(name);
            for (rel, text) in revision.modules() {
                write(&env_dir.join(safe_relative(rel)?), text)?;
            }
            write(&env_dir.join(ENV_FILE), revision.manifest_toml())?;
        }
        write(&dir.join(WORKSPACE_FILE), &workspace.to_toml()?)
    }
}

/// An environment directory's `env.toml` and the module manifests it lists, as a draft.
pub fn read_env_dir(dir: &Path) -> Result<Draft> {
    let manifest = EnvManifest::read_from(&dir.join(ENV_FILE))?;
    let mut modules = BTreeMap::new();
    for rel in &manifest.catalog.manifests {
        let path = dir.join(rel);
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        modules.insert(rel.clone(), text);
    }
    Ok(Draft {
        manifest,
        modules,
        vars: Default::default(),
    })
}

/// `rel` if it stays inside the directory it is relative to.
fn safe_relative(rel: &str) -> Result<&Path> {
    let path = Path::new(rel);
    if path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        Ok(path)
    } else {
        Err(Error::Export(format!(
            "module manifest path `{rel}` leaves the environment directory"
        )))
    }
}

fn write(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(path, contents).map_err(|e| Error::io(path, e))
}
