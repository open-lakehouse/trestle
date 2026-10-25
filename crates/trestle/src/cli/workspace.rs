//! `trestle workspace` — several environments side by side on one Docker host.
//!
//! A workspace directory holds a [`WORKSPACE_FILE`] and one environment directory per
//! member. Each member is an ordinary `trestle env` environment; the workspace gives it a slot
//! (its own host ports), a compose project name of its own and, with `--mesh`, a gateway on
//! the network the members share (see [`WorkspaceManifest`]).
//!
//! - [`WorkspaceCommand::Init`] — create the workspace file.
//! - [`WorkspaceCommand::Add`] — create a member, with every `trestle env new` flag.
//! - [`WorkspaceCommand::Remove`] — drop a member from the workspace (its directory stays).
//! - [`WorkspaceCommand::List`] — each member's slot, project, gateway and whether it runs.
//! - [`WorkspaceCommand::Render`] — re-render every member, then check that no two bind the
//!   same host port.
//! - [`WorkspaceCommand::Up`] / [`WorkspaceCommand::Down`] — start or stop members with
//!   Docker Compose.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};
use olai_stack_topology::{
    EnvManifest, WORKSPACE_FILE, WorkspaceEnv, WorkspaceManifest, baseline_catalog, port_conflicts,
};
use serde::Serialize;

use super::env::{EnvNewArgs, member_at, new_env, render_env};
use crate::error::{Error, Result};

/// The `trestle workspace` subcommand group.
#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// Create a workspace: a directory for several environments sharing one host.
    Init(InitArgs),
    /// Create an environment in the workspace, on its own ports.
    Add(Box<AddArgs>),
    /// Drop an environment from the workspace (its directory is kept).
    Remove(RemoveArgs),
    /// List the workspace's environments: slot, project, gateway and status.
    #[command(visible_alias = "status")]
    List(ListArgs),
    /// Re-render every environment, then check none bind the same host port.
    Render(DirArgs),
    /// Start environments (`docker compose up -d --wait`); all of them by default.
    Up(LifecycleArgs),
    /// Stop environments (`docker compose down`); all of them by default.
    Down(DownArgs),
}

#[derive(Args, Clone)]
pub struct DirArgs {
    /// The workspace directory (defaults to the current directory).
    #[clap(long, short = 'w', default_value = ".")]
    pub workspace: PathBuf,
}

#[derive(Args, Clone)]
pub struct InitArgs {
    /// The directory to create the workspace in (defaults to the current directory).
    #[clap(default_value = ".")]
    pub dir: PathBuf,

    /// The workspace name, prefixing every environment's compose project (defaults to the
    /// directory's name).
    #[clap(long)]
    pub name: Option<String>,

    /// Join every environment's gateway to a network the environments share, so they can
    /// reach each other through their gateways (and nothing else).
    #[clap(long)]
    pub mesh: bool,
}

#[derive(Args, Clone)]
pub struct AddArgs {
    #[clap(flatten)]
    pub dir: DirArgs,

    #[clap(flatten)]
    pub env: EnvNewArgs,
}

#[derive(Args, Clone)]
pub struct RemoveArgs {
    #[clap(flatten)]
    pub dir: DirArgs,

    /// The environment to drop.
    pub name: String,
}

#[derive(Args, Clone)]
pub struct ListArgs {
    #[clap(flatten)]
    pub dir: DirArgs,

    /// Print the listing as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct LifecycleArgs {
    #[clap(flatten)]
    pub dir: DirArgs,

    /// The environments to act on (defaults to all of them).
    pub envs: Vec<String>,
}

#[derive(Args, Clone)]
pub struct DownArgs {
    #[clap(flatten)]
    pub lifecycle: LifecycleArgs,

    /// Also remove the environments' named volumes (`docker compose down --volumes`).
    #[clap(long)]
    pub volumes: bool,
}

pub fn run(cmd: WorkspaceCommand) -> Result<()> {
    match cmd {
        WorkspaceCommand::Init(args) => run_init(args),
        WorkspaceCommand::Add(args) => run_add(*args),
        WorkspaceCommand::Remove(args) => run_remove(args),
        WorkspaceCommand::List(args) => run_list(args),
        WorkspaceCommand::Render(args) => run_render(args),
        WorkspaceCommand::Up(args) => run_up(args),
        WorkspaceCommand::Down(args) => run_down(args),
    }
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(WORKSPACE_FILE)
}

fn read_workspace(dir: &Path) -> Result<WorkspaceManifest> {
    let path = manifest_path(dir);
    WorkspaceManifest::read_from(&path).map_err(|e| {
        Error::other(format!("{}: {e}", path.display())).with_hint(format!(
            "expected a `{WORKSPACE_FILE}` in {} — create one with `trestle workspace init`",
            dir.display()
        ))
    })
}

fn write_workspace(dir: &Path, workspace: &WorkspaceManifest) -> Result<()> {
    let path = manifest_path(dir);
    workspace
        .write_to(&path)
        .map_err(|e| Error::other(format!("{}: {e}", path.display())))
}

/// The members `names` picks (all of them when empty), erroring on an unknown name.
fn pick<'a>(workspace: &'a WorkspaceManifest, names: &[String]) -> Result<Vec<&'a WorkspaceEnv>> {
    if names.is_empty() {
        return Ok(workspace.envs.iter().collect());
    }
    names
        .iter()
        .map(|name| {
            workspace.env(name).ok_or_else(|| {
                Error::other(format!("the workspace has no environment `{name}`"))
                    .with_hint("run `trestle workspace list` to see its environments")
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// init / add / remove
// ---------------------------------------------------------------------------

fn run_init(args: InitArgs) -> Result<()> {
    let path = manifest_path(&args.dir);
    if path.exists() {
        return Err(Error::other(format!("{} already exists", path.display())));
    }
    let name = match args.name {
        Some(name) => name,
        None => std::path::absolute(&args.dir)
            .map_err(|e| Error::io_at(&args.dir, e))?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error::other("the workspace directory has no name; pass --name"))?,
    };
    let workspace = WorkspaceManifest::new(name, args.mesh)
        .map_err(|e| Error::other(e.to_string()).with_hint("pass a valid name with --name"))?;
    write_workspace(&args.dir, &workspace)?;
    println!(
        "Created workspace `{}` in {}\n\nNext: trestle workspace add <env>",
        workspace.name,
        args.dir.display()
    );
    Ok(())
}

fn run_add(args: AddArgs) -> Result<()> {
    let dir = &args.dir.workspace;
    let mut workspace = read_workspace(dir)?;
    if args.env.out_dir.is_some() {
        return Err(Error::other(
            "--out-dir can't be used in a workspace: an environment lives in `<workspace>/<name>`",
        ));
    }
    let env = workspace
        .add_env(args.env.name.clone())
        .map_err(|e| Error::other(e.to_string()))?
        .clone();
    let env_dir = dir.join(&env.path);
    let mut env_args = args.env;
    env_args.out_dir = Some(env_dir.clone());
    let written = new_env(env_args, Some((&workspace, &env)));
    // A member whose env.toml was saved is recorded even if rendering stopped short (e.g. on
    // hosted secrets still to set), so `trestle workspace render` can finish it.
    if env_dir.join("env.toml").is_file() {
        write_workspace(dir, &workspace)?;
    }
    if written? {
        println!(
            "\nAdded `{}` to workspace `{}` on slot {} (ports +{})",
            env.name,
            workspace.name,
            env.slot,
            env.port_offset()
        );
    }
    Ok(())
}

fn run_remove(args: RemoveArgs) -> Result<()> {
    let dir = &args.dir.workspace;
    let mut workspace = read_workspace(dir)?;
    let env = workspace
        .remove_env(&args.name)
        .map_err(|e| Error::other(e.to_string()))?;
    write_workspace(dir, &workspace)?;
    println!(
        "Removed `{}` from workspace `{}`; its directory {} is kept (stop it first with \
         `docker compose down` there if it runs)",
        env.name,
        workspace.name,
        dir.join(&env.path).display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// One row of `trestle workspace list`.
#[derive(Serialize)]
struct Row {
    name: String,
    slot: u16,
    project: String,
    path: String,
    /// The gateway's host-side URL, or `None` without a gateway (or an unplannable env).
    gateway: Option<String>,
    /// Docker Compose's status for the project (e.g. `running(7)`), or `None` when it isn't
    /// up or Docker can't be asked.
    status: Option<String>,
}

fn run_list(args: ListArgs) -> Result<()> {
    let dir = &args.dir.workspace;
    let workspace = read_workspace(dir)?;
    let running = compose_projects();
    let rows: Vec<Row> = workspace
        .envs
        .iter()
        .map(|env| {
            let project = workspace.project_name(env);
            Row {
                name: env.name.clone(),
                slot: env.slot,
                gateway: gateway_url(&workspace, env, &dir.join(&env.path)),
                status: running.get(&project).cloned(),
                project,
                path: env.path.clone(),
            }
        })
        .collect();
    if args.json {
        let json = serde_json::to_string_pretty(&rows)
            .map_err(|e| Error::other(format!("serializing the listing: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    println!(
        "Workspace `{}`{}",
        workspace.name,
        workspace
            .mesh_network()
            .map(|n| format!(" (mesh: {n})"))
            .unwrap_or_default()
    );
    if rows.is_empty() {
        println!("\nNo environments yet: trestle workspace add <env>");
        return Ok(());
    }
    println!(
        "\n{:<16} {:>4}  {:<24} {:<36} STATUS",
        "ENV", "SLOT", "PROJECT", "GATEWAY"
    );
    for row in rows {
        println!(
            "{:<16} {:>4}  {:<24} {:<36} {}",
            row.name,
            row.slot,
            row.project,
            row.gateway.as_deref().unwrap_or("-"),
            row.status.as_deref().unwrap_or("stopped")
        );
    }
    Ok(())
}

/// The gateway URL of `env` (in `env_dir`), planned without secrets as the workspace places it;
/// `None` when it has no gateway or doesn't plan.
fn gateway_url(
    workspace: &WorkspaceManifest,
    env: &WorkspaceEnv,
    env_dir: &Path,
) -> Option<String> {
    let mut manifest = EnvManifest::read_from(&env_dir.join("env.toml")).ok()?;
    workspace.apply(env, &mut manifest.context);
    manifest.plan(&baseline_catalog()).ok()?.gateway_url()
}

/// Every Compose project Docker knows of, by name, with its status. Empty when Docker isn't
/// available.
fn compose_projects() -> BTreeMap<String, String> {
    #[derive(serde::Deserialize)]
    struct Project {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Status")]
        status: String,
    }
    let Ok(out) = Command::new("docker")
        .args(["compose", "ls", "--all", "--format", "json"])
        .output()
    else {
        return BTreeMap::new();
    };
    serde_json::from_slice::<Vec<Project>>(&out.stdout)
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.name, p.status))
        .collect()
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

fn run_render(args: DirArgs) -> Result<()> {
    let dir = &args.workspace;
    let workspace = read_workspace(dir)?;
    let mut plans = Vec::new();
    for env in &workspace.envs {
        let env_dir = dir.join(&env.path);
        println!("== {} (slot {})", env.name, env.slot);
        if member_at(&workspace, dir, &env_dir).is_none() {
            return Err(Error::other(format!(
                "environment `{}` has no directory at {}",
                env.name,
                env_dir.display()
            )));
        }
        plans.push((
            env.name.as_str(),
            render_env(&env_dir, None, false, Some((&workspace, env)))?,
        ));
        println!();
    }
    let conflicts = port_conflicts(plans.iter().map(|(name, plan)| (*name, plan)));
    if !conflicts.is_empty() {
        let lines: Vec<String> = conflicts.iter().map(|c| format!("  {c}")).collect();
        return Err(Error::other(format!(
            "the environments can't run side by side:\n{}",
            lines.join("\n")
        ))
        .with_hint("give the clashing ports distinct values in the envs' env.toml"));
    }
    println!(
        "Rendered {} environment(s); no host port is bound twice",
        plans.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// up / down
// ---------------------------------------------------------------------------

fn run_up(args: LifecycleArgs) -> Result<()> {
    compose_each(&args, &["up", "-d", "--wait"])
}

fn run_down(args: DownArgs) -> Result<()> {
    let mut compose = vec!["down"];
    if args.volumes {
        compose.push("--volumes");
    }
    compose_each(&args.lifecycle, &compose)?;
    // Compose labels the mesh with the project that created it, so no member's `down` removes
    // it once another member has joined: drop it here when the last member stopped.
    let dir = &args.lifecycle.dir.workspace;
    let workspace = read_workspace(dir)?;
    if let Some(network) = workspace.mesh_network() {
        let running = compose_projects();
        if workspace
            .envs
            .iter()
            .all(|e| !running.contains_key(&workspace.project_name(e)))
        {
            let removed = Command::new("docker")
                .args(["network", "rm", &network])
                .output()
                .is_ok_and(|o| o.status.success());
            if removed {
                println!("Removed the mesh network {network}");
            }
        }
    }
    Ok(())
}

/// Run `docker compose <compose…>` in each picked member's directory, in order, stopping at
/// the first failure.
fn compose_each(args: &LifecycleArgs, compose: &[&str]) -> Result<()> {
    let dir = &args.dir.workspace;
    let workspace = read_workspace(dir)?;
    for env in pick(&workspace, &args.envs)? {
        let env_dir = dir.join(&env.path);
        println!("== {}: docker compose {}", env.name, compose.join(" "));
        let status = Command::new("docker")
            .arg("compose")
            .args(compose)
            .current_dir(&env_dir)
            .status()
            .map_err(|e| {
                Error::other(format!("running docker compose: {e}"))
                    .with_hint("install Docker with the Compose plugin")
            })?;
        if !status.success() {
            return Err(Error::other(format!(
                "`docker compose {}` failed for `{}`",
                compose.join(" "),
                env.name
            )));
        }
    }
    Ok(())
}
