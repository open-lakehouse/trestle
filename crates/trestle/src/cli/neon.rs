//! `trestle neon` and the Neon side of `trestle up`: branches of a workspace's Neon project,
//! and binding instances to them.
//!
//! A workspace names its Neon project once (`trestle ws set-neon`). `trestle up <ws>/<env>
//! --neon-branch <branch>` then binds the instance to a branch of it (`new` creates one, owned
//! by trestle and expiring), and the branch supplies the Postgres, object storage and function
//! URLs the revision's Neon-hosted placements need. Everything goes through the Neon CLI.

use std::time::Duration;

use clap::{Args, Subcommand};
use olai_stack_runtime::neon::{
    NEON_SOURCE, NeonBinding, NeonControl, StorageCredentialRef, bind, create_owned_branch,
    delete_owned_branch, expires_in, reset_owned_branch, unbind,
};
use olai_stack_state::{Environment, Instance, Revision, SecretRef, SourceBinding, Workspace};
use olai_stack_topology::NeonRequirements;
use serde::Serialize;

use super::output::emit;
use super::state::{Ctx, KEYCHAIN, neon_err};
use super::target::find_workspace;
use crate::error::{Error, Result};

/// The `neon` source config key naming the workspace's Neon project.
const PROJECT_KEY: &str = "project_id";
/// The `neon` source config key naming the branch new branches are made from.
const PARENT_KEY: &str = "parent";

/// How long a branch `--neon-branch new` creates lives, unless `--ttl` says otherwise.
pub(super) const DEFAULT_TTL: &str = "7d";

/// The `trestle neon` subcommand group.
#[derive(Subcommand)]
pub enum NeonCommand {
    /// Branches of the workspace's Neon project.
    #[command(subcommand)]
    Branch(BranchCommand),
}

/// `trestle neon branch`.
#[derive(Subcommand)]
pub enum BranchCommand {
    /// List the project's branches, marking those trestle created.
    List(ListArgs),
    /// Create a branch owned by trestle, expiring after `--ttl`.
    Create(CreateArgs),
    /// Delete a branch trestle created.
    Delete(BranchArgs),
    /// Reset a branch trestle created to its parent's current state.
    Reset(BranchArgs),
}

#[derive(Args, Clone)]
pub struct ListArgs {
    /// The workspace.
    pub workspace: String,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct CreateArgs {
    /// The workspace.
    pub workspace: String,

    /// The branch name.
    pub name: String,

    /// The branch to create it from (the workspace's parent, else the project's default).
    #[clap(long)]
    pub parent: Option<String>,

    /// How long it lives, e.g. `12h`, `7d`.
    #[clap(long, default_value = DEFAULT_TTL, value_parser = parse_ttl)]
    pub ttl: Duration,
}

#[derive(Args, Clone)]
pub struct BranchArgs {
    /// The workspace.
    pub workspace: String,

    /// The branch name or id.
    pub name: String,
}

/// Parse `<n>m`, `<n>h` or `<n>d`.
pub(super) fn parse_ttl(s: &str) -> std::result::Result<Duration, String> {
    let bad = || format!("expected a duration like `12h` or `7d`, got `{s}`");
    let (n, unit) = s.split_at(s.len().checked_sub(1).ok_or_else(bad)?);
    let n: u64 = n.parse().map_err(|_| bad())?;
    let secs = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(bad()),
    };
    Ok(Duration::from_secs(n * secs))
}

/// The workspace's Neon project and parent branch, from `ws set-neon`.
pub(super) fn neon_project(ws: &Workspace) -> Result<(String, Option<String>)> {
    let config = ws.sources.get(NEON_SOURCE);
    let get = |key: &str| {
        config
            .and_then(|c| c.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let project = get(PROJECT_KEY).ok_or_else(|| {
        Error::other(format!("workspace `{}` has no Neon project", ws.name))
            .with_hint(format!("`trestle ws set-neon {} --project <id>`", ws.name))
    })?;
    Ok((project, get(PARENT_KEY)))
}

/// The config `ws set-neon` stores.
pub(super) fn project_config(project: &str, parent: Option<&str>) -> serde_json::Value {
    let mut config = serde_json::json!({ PROJECT_KEY: project });
    if let Some(parent) = parent {
        config[PARENT_KEY] = parent.into();
    }
    config
}

pub fn run(cmd: NeonCommand) -> Result<()> {
    let ctx = Ctx::open()?;
    let NeonCommand::Branch(cmd) = cmd;
    ctx.block_on(async {
        let neon = ctx.neon()?;
        match cmd {
            BranchCommand::List(args) => list(&ctx, &neon, args).await,
            BranchCommand::Create(args) => create(&ctx, &neon, args).await,
            BranchCommand::Delete(args) => {
                let ws = find_workspace(&ctx.store, &args.workspace)?;
                let (project, _) = neon_project(&ws)?;
                let branch = delete_owned_branch(&neon, &project, &args.name)
                    .await
                    .map_err(neon_err)?;
                println!("deleted Neon branch `{}` ({})", branch.name, branch.id);
                Ok(())
            }
            BranchCommand::Reset(args) => {
                let ws = find_workspace(&ctx.store, &args.workspace)?;
                let (project, _) = neon_project(&ws)?;
                let branch = reset_owned_branch(&neon, &project, &args.name)
                    .await
                    .map_err(neon_err)?;
                println!("reset Neon branch `{}` to its parent", branch.name);
                Ok(())
            }
        }
    })
}

/// A branch in `neon branch list`.
#[derive(Serialize)]
struct BranchRow {
    name: String,
    id: String,
    owned: bool,
    default: bool,
    expires_at: Option<String>,
}

async fn list(ctx: &Ctx, neon: &dyn NeonControl, args: ListArgs) -> Result<()> {
    let ws = find_workspace(&ctx.store, &args.workspace)?;
    let (project, _) = neon_project(&ws)?;
    let rows: Vec<BranchRow> = neon
        .branches(&project)
        .await
        .map_err(neon_err)?
        .into_iter()
        .map(|b| BranchRow {
            owned: b.is_owned(),
            name: b.name,
            id: b.id,
            default: b.default,
            expires_at: b.expires_at,
        })
        .collect();
    emit(args.json, "neon_branches", &rows, |rows| {
        for r in rows {
            let mut notes = Vec::new();
            if r.default {
                notes.push("default".to_owned());
            }
            if r.owned {
                notes.push("trestle".to_owned());
            }
            if let Some(at) = &r.expires_at {
                notes.push(format!("expires {at}"));
            }
            println!("{:<28} {:<28} {}", r.name, r.id, notes.join(", "));
        }
    })
}

async fn create(ctx: &Ctx, neon: &dyn NeonControl, args: CreateArgs) -> Result<()> {
    let ws = find_workspace(&ctx.store, &args.workspace)?;
    let (project, parent) = neon_project(&ws)?;
    let parent = args.parent.or(parent);
    let branch = create_owned_branch(
        neon,
        &project,
        &args.name,
        parent.as_deref(),
        Some(expires_in(args.ttl)),
        None,
    )
    .await
    .map_err(neon_err)?;
    println!(
        "created Neon branch `{}` ({}), expiring {}",
        branch.name,
        branch.id,
        branch.expires_at.as_deref().unwrap_or("never")
    );
    Ok(())
}

/// What the revision needs of a Neon branch, from a plan of it without secrets.
pub(super) fn requirements(revision: &Revision) -> Result<NeonRequirements> {
    let catalog = super::env_store::catalog_for(revision.modules())?;
    let plan = revision
        .manifest()
        .plan(&catalog)
        .map_err(super::env::map_plan_err)?;
    Ok(plan.neon_requirements())
}

/// The instance's Neon binding, if it has one.
pub(super) fn binding_of(instance: &Instance) -> Option<NeonBinding> {
    instance
        .sources
        .iter()
        .find(|s| s.name == NEON_SOURCE)
        .and_then(|s| serde_json::from_value(s.config.clone()).ok())
}

/// Bind the instance to `branch` (`new`: a branch created for it, living `ttl`), unless it is
/// bound to it already. A binding to another branch is undone first.
#[allow(clippy::too_many_arguments)]
pub(super) async fn bind_instance(
    ctx: &Ctx,
    ws: &Workspace,
    env: &Environment,
    revision: &Revision,
    instance: &Instance,
    branch: &str,
    ttl: Duration,
    req: &NeonRequirements,
) -> Result<NeonBinding> {
    let neon = ctx.neon()?;
    let (project, parent) = neon_project(ws)?;
    let branch = if branch == "new" {
        let name = format!("{}-{}-r{}", ws.name, env.name, revision.number());
        create_owned_branch(
            &neon,
            &project,
            &name,
            parent.as_deref(),
            Some(expires_in(ttl)),
            Some(&instance.id.0),
        )
        .await
        .map_err(neon_err)?
        .id
    } else {
        branch.to_owned()
    };
    if let Some(current) = binding_of(instance) {
        if current.branch == branch || current.branch_name == branch {
            return Ok(current);
        }
        unbind_instance(ctx, &neon, instance).await?;
    }
    let credential_name = format!("trestle-{}", instance.id);
    let (mut binding, credential) = bind(&neon, &project, &branch, req, &credential_name)
        .await
        .map_err(neon_err)?;
    if let Some(credential) = credential {
        let key = format!(
            "neon/{}/{}/r{}/{}",
            ws.name,
            env.name,
            revision.number(),
            credential.token_id
        );
        ctx.secrets().set(&key, &credential.secret_access_key)?;
        binding.storage_credential = Some(StorageCredentialRef {
            token_id: credential.token_id,
            secret: SecretRef {
                provider: KEYCHAIN.into(),
                key,
            },
        });
    }
    let config = serde_json::to_value(&binding).map_err(|e| Error::other(e.to_string()))?;
    let mut sources: Vec<SourceBinding> = instance
        .sources
        .iter()
        .filter(|s| s.name != NEON_SOURCE)
        .cloned()
        .collect();
    sources.push(SourceBinding::new(NEON_SOURCE).with_config(config));
    ctx.store.set_sources(&instance.id, &sources)?;
    Ok(binding)
}

/// Undo the instance's Neon binding: revoke its storage credential, forget the secret and drop
/// the source.
pub(super) async fn unbind_instance(
    ctx: &Ctx,
    neon: &dyn NeonControl,
    instance: &Instance,
) -> Result<()> {
    let Some(binding) = binding_of(instance) else {
        return Ok(());
    };
    release(ctx, neon, &binding).await?;
    let sources: Vec<SourceBinding> = instance
        .sources
        .iter()
        .filter(|s| s.name != NEON_SOURCE)
        .cloned()
        .collect();
    ctx.store.set_sources(&instance.id, &sources)?;
    Ok(())
}

/// Revoke the binding's storage credential and forget its secret.
pub(super) async fn release(
    ctx: &Ctx,
    neon: &dyn NeonControl,
    binding: &NeonBinding,
) -> Result<()> {
    unbind(neon, binding).await.map_err(neon_err)?;
    if let Some(credential) = &binding.storage_credential {
        ctx.secrets().delete(&credential.secret.key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttls_parse() {
        assert_eq!(parse_ttl("12h").unwrap(), Duration::from_secs(12 * 3600));
        assert_eq!(parse_ttl("7d").unwrap(), Duration::from_secs(7 * 86_400));
        assert_eq!(parse_ttl("30m").unwrap(), Duration::from_secs(1800));
        for bad in ["", "7", "d", "7w", "-1d"] {
            assert!(parse_ttl(bad).is_err(), "{bad}");
        }
    }
}
