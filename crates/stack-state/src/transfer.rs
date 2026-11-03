//! Import and export: `trestle-workspace.toml` and environment directories as an exchange
//! format for the store.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use olai_stack_topology::{EnvManifest, MAX_SLOT, WORKSPACE_FILE, WorkspaceEnv, WorkspaceManifest};

use crate::error::{Error, Result};
use crate::model::{Draft, EnvironmentId, InstanceId, Revision, Workspace, WorkspaceId};
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
        let mut workspace: Option<Workspace> = None;
        let mut members = Vec::new();
        for id in instances {
            let instance = self.instance(id)?;
            let env = self.environment(&instance.environment)?;
            let ws = self.workspace_by_id(&env.workspace)?;
            let first = workspace.get_or_insert_with(|| ws.clone());
            if first.id != ws.id {
                return Err(Error::Export(format!(
                    "the instances belong to two workspaces, `{}` and `{}`",
                    first.name, ws.name
                )));
            }
            let slot = instance.slot.ok_or_else(|| {
                Error::Export(format!("instance {id} holds no slot; lease one first"))
            })?;
            let revision = self.revision(&instance.environment, instance.revision)?;
            members.push((env.name, slot, revision));
        }
        let Some(workspace) = workspace else {
            return Err(Error::Export("no instances given".into()));
        };
        write_workspace(&workspace, &members, dir)
    }

    /// Export the head revision of every environment of the workspace as a workspace
    /// directory in `dir`, as [`export_instances`](Self::export_instances) does. A member sits
    /// on the slot of an instance of its head when one holds a slot, else on the lowest slot no
    /// other member takes. Every environment needs a revision.
    pub fn export_workspace(&self, workspace: &WorkspaceId, dir: &Path) -> Result<()> {
        let ws = self.workspace_by_id(workspace)?;
        let mut members = Vec::new();
        let mut unslotted = Vec::new();
        for env in self.environments(workspace)? {
            let head = env.head.ok_or_else(|| {
                Error::Export(format!(
                    "environment `{}` has no revision; commit it first",
                    env.name
                ))
            })?;
            let revision = self.revision(&env.id, head)?;
            let slot = self
                .instances_of(&env.id)?
                .into_iter()
                .filter(|i| i.revision == head)
                .find_map(|i| i.slot);
            match slot {
                Some(slot) => members.push((env.name, slot, revision)),
                None => unslotted.push((env.name, revision)),
            }
        }
        for (name, revision) in unslotted {
            let slot = (0..=MAX_SLOT)
                .find(|s| members.iter().all(|(_, taken, _)| taken != s))
                .ok_or(Error::SlotsExhausted)?;
            members.push((name, slot, revision));
        }
        members.sort_by(|a, b| a.0.cmp(&b.0));
        write_workspace(&ws, &members, dir)
    }

    /// Export revision `number` of the environment as an environment directory in `dir`: its
    /// `env.toml` and module manifests. No instance is needed. Like any `env.toml` it carries no
    /// placement, so it renders as an environment on its own.
    pub fn export_revision(&self, env: &EnvironmentId, number: u32, dir: &Path) -> Result<()> {
        write_env_dir(&self.revision(env, number)?, dir)
    }
}

/// Write the workspace manifest for `members` (name, slot, revision) and each member's
/// environment directory under `dir`.
fn write_workspace(ws: &Workspace, members: &[(String, u16, Revision)], dir: &Path) -> Result<()> {
    let manifest = WorkspaceManifest {
        version: WorkspaceManifest::current_version(),
        name: ws.name.clone(),
        mesh: ws.mesh,
        envs: members
            .iter()
            .map(|(name, slot, _)| WorkspaceEnv {
                name: name.clone(),
                path: name.clone(),
                slot: *slot,
            })
            .collect(),
    };
    // Two instances of one environment would be two members of one name.
    manifest.validate()?;
    for (name, _, revision) in members {
        write_env_dir(revision, &dir.join(name))?;
    }
    write(&dir.join(WORKSPACE_FILE), &manifest.to_toml()?)
}

/// Write the revision's `env.toml` and module manifests into `dir`.
fn write_env_dir(revision: &Revision, dir: &Path) -> Result<()> {
    for (rel, text) in revision.modules() {
        write(&dir.join(safe_relative(rel)?), text)?;
    }
    write(&dir.join(ENV_FILE), revision.manifest_toml())
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
