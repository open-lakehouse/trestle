//! `trestle ws`: workspaces in the trestle state store.
//!
//! A workspace is a set of environments sharing the Docker host, with the vars they all see
//! and whether their gateways share a mesh network. It lives in the store the desktop app
//! shares, under `$TRESTLE_HOME`; `ws import` / `ws export` move it to and from a workspace
//! directory (`trestle-workspace.toml` plus one directory per environment).

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use olai_stack_state::{Desired, Store, Workspace};
use serde::Serialize;

use super::output::emit;
use super::state::{Ctx, any_running, workspace_keychain_refs};
use super::target::find_workspace;
use super::util::ensure_writable_dir;
use super::var::{VarRow, var_rows};
use crate::error::{Error, Result};

/// The `trestle ws` subcommand group.
#[derive(Subcommand)]
pub enum WsCommand {
    /// Create an empty workspace.
    Create(CreateArgs),
    /// List the workspaces.
    List(JsonArgs),
    /// Show a workspace: its environments, revisions, instances and vars.
    Show(ShowArgs),
    /// Delete a workspace with its environments, revisions and instances.
    Rm(RmArgs),
    /// Import a workspace directory (`trestle-workspace.toml`) as a new workspace; each
    /// environment's `env.toml` becomes its revision 1.
    Import(ImportArgs),
    /// Export a workspace's head revisions as a workspace directory.
    Export(ExportArgs),
}

#[derive(Args, Clone)]
pub struct CreateArgs {
    /// The workspace name: lowercase letters, digits and dashes.
    pub name: String,

    /// Join every environment's gateway to a network the environments share.
    #[clap(long)]
    pub mesh: bool,
}

#[derive(Args, Clone)]
pub struct JsonArgs {
    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ShowArgs {
    /// The workspace.
    pub name: String,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct RmArgs {
    /// The workspace.
    pub name: String,

    /// Stop and remove its running instances first (with their volumes).
    #[clap(long)]
    pub force: bool,
}

#[derive(Args, Clone)]
pub struct ImportArgs {
    /// The workspace directory (holding `trestle-workspace.toml`).
    #[clap(default_value = ".")]
    pub dir: PathBuf,
}

#[derive(Args, Clone)]
pub struct ExportArgs {
    /// The workspace.
    pub name: String,

    /// The directory to write.
    pub dir: PathBuf,

    /// Write into a non-empty directory (overwrites the exported files).
    #[clap(long)]
    pub force: bool,
}

pub fn run(cmd: WsCommand) -> Result<()> {
    let ctx = Ctx::open()?;
    match cmd {
        WsCommand::Create(args) => run_create(&ctx, args),
        WsCommand::List(args) => run_list(&ctx, args),
        WsCommand::Show(args) => run_show(&ctx, args),
        WsCommand::Rm(args) => ctx.block_on(run_rm(&ctx, args)),
        WsCommand::Import(args) => run_import(&ctx, args),
        WsCommand::Export(args) => run_export(&ctx, args),
    }
}

fn run_create(ctx: &Ctx, args: CreateArgs) -> Result<()> {
    let ws = ctx.store.create_workspace(&args.name, args.mesh)?;
    println!("created workspace `{}`", ws.name);
    println!("  next: `trestle env create {}/<env>`", ws.name);
    Ok(())
}

/// A workspace in `ws list`.
#[derive(Serialize)]
struct WsRow {
    name: String,
    mesh: bool,
    environments: usize,
    instances: usize,
    running: usize,
}

fn run_list(ctx: &Ctx, args: JsonArgs) -> Result<()> {
    let instances = ctx.store.instances()?;
    let mut rows = Vec::new();
    for ws in ctx.store.workspaces()? {
        let envs = ctx.store.environments(&ws.id)?;
        let mine: Vec<_> = instances
            .iter()
            .filter(|i| envs.iter().any(|e| e.id == i.environment))
            .collect();
        rows.push(WsRow {
            environments: envs.len(),
            instances: mine.len(),
            running: mine
                .iter()
                .filter(|i| i.desired == Desired::Running)
                .count(),
            name: ws.name,
            mesh: ws.mesh,
        });
    }
    emit(args.json, "workspaces", &rows, |rows| {
        if rows.is_empty() {
            println!("no workspaces; `trestle ws create <name>` or `trestle ws import <dir>`");
            return;
        }
        println!(
            "{:<20} {:<5} {:>4} {:>9} {:>7}",
            "WORKSPACE", "MESH", "ENVS", "INSTANCES", "RUNNING"
        );
        for r in rows {
            println!(
                "{:<20} {:<5} {:>4} {:>9} {:>7}",
                r.name,
                if r.mesh { "yes" } else { "no" },
                r.environments,
                r.instances,
                r.running
            );
        }
    })
}

/// A workspace in `ws show`.
#[derive(Serialize)]
pub(super) struct WsDetail {
    name: String,
    mesh: bool,
    vars: Vec<VarRow>,
    sources: BTreeMap<String, serde_json::Value>,
    environments: Vec<EnvSummary>,
}

/// An environment in `ws show`.
#[derive(Serialize)]
struct EnvSummary {
    name: String,
    head: Option<u32>,
    revisions: usize,
    /// Whether the draft differs from the head revision (always, without one).
    draft_changed: bool,
    instances: Vec<InstanceSummary>,
}

/// An instance in `ws show` and `ps`.
#[derive(Serialize)]
pub(super) struct InstanceSummary {
    pub id: String,
    pub revision: u32,
    pub desired: Desired,
    pub slot: Option<u16>,
}

fn run_show(ctx: &Ctx, args: ShowArgs) -> Result<()> {
    let ws = find_workspace(&ctx.store, &args.name)?;
    let detail = workspace_detail(&ctx.store, &ws)?;
    emit(args.json, "workspace", &detail, |d| {
        println!(
            "workspace {}{}",
            d.name,
            if d.mesh { " (mesh)" } else { "" }
        );
        if !d.vars.is_empty() {
            println!("vars:");
            for v in &d.vars {
                println!("  {v}");
            }
        }
        for (name, config) in &d.sources {
            println!("source {name}: {config}");
        }
        if d.environments.is_empty() {
            println!("no environments; `trestle env create {}/<env>`", d.name);
        }
        for e in &d.environments {
            let head = e.head.map_or("no revision".to_owned(), |h| format!("r{h}"));
            let draft = if e.draft_changed {
                ", draft changed"
            } else {
                ""
            };
            println!("{}/{}: {head}{draft}", d.name, e.name);
            for i in &e.instances {
                let slot = i.slot.map_or("-".to_owned(), |s| s.to_string());
                let desired = match i.desired {
                    Desired::Running => "running",
                    Desired::Stopped => "stopped",
                };
                println!("  r{} {desired} slot {slot} ({})", i.revision, i.id);
            }
        }
    })
}

pub(super) fn workspace_detail(store: &Store, ws: &Workspace) -> Result<WsDetail> {
    let mut environments = Vec::new();
    for env in store.environments(&ws.id)? {
        let revisions = store.revisions(&env.id)?;
        let draft_changed = match revisions.last() {
            Some(head) => super::env_store::draft_differs(&env, head)?,
            None => true,
        };
        let instances = store
            .instances_of(&env.id)?
            .into_iter()
            .map(|i| InstanceSummary {
                id: i.id.0,
                revision: i.revision,
                desired: i.desired,
                slot: i.slot,
            })
            .collect();
        environments.push(EnvSummary {
            name: env.name,
            head: env.head,
            revisions: revisions.len(),
            draft_changed,
            instances,
        });
    }
    Ok(WsDetail {
        name: ws.name.clone(),
        mesh: ws.mesh,
        vars: var_rows(&ws.vars, "workspace"),
        sources: ws.sources.clone(),
        environments,
    })
}

async fn run_rm(ctx: &Ctx, args: RmArgs) -> Result<()> {
    let ws = find_workspace(&ctx.store, &args.name)?;
    let mut instances = Vec::new();
    for env in ctx.store.environments(&ws.id)? {
        instances.extend(ctx.store.instances_of(&env.id)?);
    }
    remove_instances(
        ctx,
        &instances,
        args.force,
        &format!("workspace `{}`", ws.name),
    )
    .await?;
    let refs = workspace_keychain_refs(&ctx.store, &ws)?;
    ctx.store.delete_workspace(&ws.id)?;
    for r in refs {
        ctx.secrets().delete(&r.key)?;
    }
    if ws.mesh {
        // Best effort: the network goes once nothing uses it.
        let _ = olai_stack_runtime::ComposeEngine::network_rm(
            ctx.engine(),
            &format!("trestle-{}-mesh", ws.name),
        )
        .await;
    }
    println!("deleted workspace `{}`", ws.name);
    Ok(())
}

/// Refuse to delete `what` while one of `instances` runs, unless `force`, which takes them all
/// down (volumes too) first.
pub(super) async fn remove_instances(
    ctx: &Ctx,
    instances: &[olai_stack_state::Instance],
    force: bool,
    what: &str,
) -> Result<()> {
    if let Some(running) = any_running(ctx, instances).await? {
        if !force {
            return Err(Error::other(format!(
                "{what} has a running instance (revision {}, {})",
                running.revision, running.id
            ))
            .with_hint(
                "`trestle down` it first, or pass `--force` to remove it with its volumes",
            ));
        }
        let runtime = ctx.instances();
        for instance in instances {
            runtime.remove(&instance.id, true).await?;
        }
    }
    Ok(())
}

fn run_import(ctx: &Ctx, args: ImportArgs) -> Result<()> {
    let ws = ctx.store.import_workspace(&args.dir).map_err(|e| {
        Error::from(e).with_hint("a workspace directory holds `trestle-workspace.toml`")
    })?;
    let envs = ctx.store.environments(&ws.id)?;
    println!(
        "imported workspace `{}` from {} ({} environment{}, each at r1)",
        ws.name,
        args.dir.display(),
        envs.len(),
        if envs.len() == 1 { "" } else { "s" }
    );
    println!("  secrets.env is not imported: set secrets with `trestle var set --secret`");
    println!("  next: `trestle up {}/<env>`", ws.name);
    Ok(())
}

fn run_export(ctx: &Ctx, args: ExportArgs) -> Result<()> {
    let ws = find_workspace(&ctx.store, &args.name)?;
    ensure_writable_dir(&args.dir, args.force)?;
    ctx.store.export_workspace(&ws.id, &args.dir)?;
    println!("exported workspace `{}` to {}", ws.name, args.dir.display());
    Ok(())
}
