//! `trestle workspace`: several environments side by side on one Docker host.
//!
//! Deprecated in favour of the store-backed `trestle ws` (`ws import` takes a workspace
//! directory in) and `trestle up`/`down`/`ps`.
//!
//! A workspace directory holds a [`WORKSPACE_FILE`] and one environment directory per
//! member. Each member is an ordinary `trestle env` environment; the workspace gives it a slot
//! (its own host ports), a compose project name of its own and, with `--mesh`, a gateway on
//! the network the members share (see [`WorkspaceManifest`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use olai_stack_runtime::{
    ComposeEngine, DockerCli, DownOptions, Error as RuntimeError, UpOptions, remove_mesh_if_idle,
};
use olai_stack_topology::{
    EnvManifest, WORKSPACE_FILE, WorkspaceEnv, WorkspaceManifest, port_conflicts,
};
use serde::Serialize;

use super::env::{EnvNewArgs, load_catalog, member_at, new_env, render_env};
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
    eprintln!(
        "warning: `trestle workspace` is deprecated: `trestle ws import <dir>` moves a workspace \
         directory into the trestle state, then `trestle up <ws>/<env>` runs it"
    );
    let docker = DockerCli::new();
    match cmd {
        WorkspaceCommand::Init(args) => run_init(args),
        WorkspaceCommand::Add(args) => run_add(*args),
        WorkspaceCommand::Remove(args) => run_remove(args),
        WorkspaceCommand::List(args) => block_on(run_list(&docker, args)),
        WorkspaceCommand::Render(args) => run_render(args),
        WorkspaceCommand::Up(args) => block_on(run_up(&docker, args)),
        WorkspaceCommand::Down(args) => block_on(run_down(&docker, args)),
    }
}

/// Drive one of the Docker-facing commands to completion on a single-threaded runtime.
fn block_on(fut: impl std::future::Future<Output = Result<()>>) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::other(format!("starting the async runtime: {e}")))?
        .block_on(fut)
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

async fn run_list(engine: &dyn ComposeEngine, args: ListArgs) -> Result<()> {
    let dir = &args.dir.workspace;
    let workspace = read_workspace(dir)?;
    let running = compose_projects(engine).await;
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

/// The gateway URL of `env` (in `env_dir`), planned without secrets as the workspace places it
/// and against the module manifests the environment lists, as `render` does; `None` when it has
/// no gateway or doesn't plan.
fn gateway_url(
    workspace: &WorkspaceManifest,
    env: &WorkspaceEnv,
    env_dir: &Path,
) -> Option<String> {
    let mut manifest = EnvManifest::read_from(&env_dir.join("env.toml")).ok()?;
    let catalog = load_catalog(&manifest, env_dir).ok()?;
    workspace.apply(env, &mut manifest.context);
    manifest.plan(&catalog).ok()?.gateway_url()
}

/// Every Compose project Docker knows of, by name, with its status. Empty when Docker isn't
/// available (or can't be asked), so a listing still shows every member as stopped.
async fn compose_projects(engine: &dyn ComposeEngine) -> BTreeMap<String, String> {
    match engine.ls().await {
        Ok(projects) => projects.into_iter().map(|p| (p.name, p.status)).collect(),
        Err(e) => {
            tracing::debug!("listing compose projects: {e}");
            BTreeMap::new()
        }
    }
}

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

/// What `up` or `down` does to each picked member.
enum Lifecycle {
    Up,
    Down { volumes: bool },
}

impl Lifecycle {
    /// The compose command, as the progress line and an error show it.
    fn command(&self) -> &'static str {
        match self {
            Lifecycle::Up => "up -d --wait",
            Lifecycle::Down { volumes: false } => "down",
            Lifecycle::Down { volumes: true } => "down --volumes",
        }
    }
}

async fn run_up(engine: &dyn ComposeEngine, args: LifecycleArgs) -> Result<()> {
    compose_each(engine, &args, Lifecycle::Up).await
}

async fn run_down(engine: &dyn ComposeEngine, args: DownArgs) -> Result<()> {
    let lifecycle = Lifecycle::Down {
        volumes: args.volumes,
    };
    compose_each(engine, &args.lifecycle, lifecycle).await?;
    // Compose labels the mesh with the project that created it, so no member's `down` removes
    // it once another member has joined: drop it here when the last member stopped.
    let dir = &args.lifecycle.dir.workspace;
    let workspace = read_workspace(dir)?;
    if let Some(network) = workspace.mesh_network() {
        let members: BTreeSet<String> = workspace
            .envs
            .iter()
            .map(|e| workspace.project_name(e))
            .collect();
        if remove_mesh_if_idle(engine, &network, &members)
            .await
            .unwrap_or(false)
        {
            println!("Removed the mesh network {network}");
        }
    }
    Ok(())
}

/// Run `docker compose <lifecycle>` in each picked member's directory, in order, stopping at
/// the first failure.
async fn compose_each(
    engine: &dyn ComposeEngine,
    args: &LifecycleArgs,
    lifecycle: Lifecycle,
) -> Result<()> {
    let dir = &args.dir.workspace;
    let workspace = read_workspace(dir)?;
    for env in pick(&workspace, &args.envs)? {
        let env_dir = dir.join(&env.path);
        println!("== {}: docker compose {}", env.name, lifecycle.command());
        let result = match lifecycle {
            Lifecycle::Up => {
                let opts = UpOptions {
                    wait: true,
                    ..UpOptions::default()
                };
                engine.up(&env_dir, &opts).await
            }
            Lifecycle::Down { volumes } => {
                let opts = DownOptions {
                    volumes,
                    ..DownOptions::default()
                };
                engine.down(&env_dir, &opts).await
            }
        };
        result.map_err(|e| match e {
            RuntimeError::Unavailable { reason } => {
                Error::other(format!("running docker compose: {reason}"))
                    .with_hint("install Docker with the Compose plugin")
            }
            RuntimeError::Failed { .. } => Error::other(format!(
                "`docker compose {}` failed for `{}`",
                lifecycle.command(),
                env.name
            )),
            e => Error::other(format!("running docker compose: {e}")),
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use olai_stack_topology::{PlanCtx, Selection};

    /// A module manifest that adds `probe`, which the environment below selects.
    const PROBE: &str = "id: probe\nmodel: probe\nprovider_of: probe_role\nrenders:\n  compose:\n    fragment: \"\"\n";

    #[test]
    fn gateway_url_plans_with_the_members_module_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let mut env_manifest = EnvManifest::new(Selection::modules(["probe"]), PlanCtx::default());
        std::fs::write(dir.path().join("probe.module.yaml"), PROBE).unwrap();
        env_manifest.catalog.manifests = vec!["probe.module.yaml".to_string()];
        env_manifest.write_to(&dir.path().join("env.toml")).unwrap();

        let mut workspace = WorkspaceManifest::new("ws", false).unwrap();
        let env = workspace.add_env("probe-env").unwrap().clone();
        assert!(gateway_url(&workspace, &env, dir.path()).is_some());

        // Without the manifest the baseline catalog doesn't know `probe`, so there is no plan.
        env_manifest.catalog.manifests.clear();
        env_manifest.write_to(&dir.path().join("env.toml")).unwrap();
        assert_eq!(gateway_url(&workspace, &env, dir.path()), None);
    }

    /// A mesh workspace `demo` with members `dev` and `prod`, each with a head compose file
    /// naming its project.
    fn fake_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = WorkspaceManifest::new("demo", true).unwrap();
        for name in ["dev", "prod"] {
            let env = workspace.add_env(name).unwrap().clone();
            let env_dir = dir.path().join(&env.path);
            std::fs::create_dir_all(&env_dir).unwrap();
            let project = workspace.project_name(&env);
            std::fs::write(env_dir.join("compose.yaml"), format!("name: {project}\n")).unwrap();
        }
        write_workspace(dir.path(), &workspace).unwrap();
        dir
    }

    fn lifecycle(dir: &Path, envs: &[&str]) -> LifecycleArgs {
        LifecycleArgs {
            dir: DirArgs {
                workspace: dir.to_owned(),
            },
            envs: envs.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn up_and_down_drive_every_member_and_drop_the_idle_mesh() {
        use olai_stack_runtime::{FakeCall, FakeEngine};
        let dir = fake_workspace();
        let engine = FakeEngine::new();
        engine.add_network("trestle-demo-mesh");

        run_up(&engine, lifecycle(dir.path(), &[])).await.unwrap();
        assert_eq!(
            engine.projects(),
            ["demo-dev".to_owned(), "demo-prod".to_owned()].into()
        );
        let running = compose_projects(&engine).await;
        assert_eq!(running["demo-prod"], "running(1)");

        // Stopping one member keeps the mesh the other still uses.
        let down = |envs: &[&str]| DownArgs {
            lifecycle: lifecycle(dir.path(), envs),
            volumes: true,
        };
        run_down(&engine, down(&["dev"])).await.unwrap();
        assert!(engine.networks().contains("trestle-demo-mesh"));
        run_down(&engine, down(&["prod"])).await.unwrap();
        assert!(engine.networks().is_empty());
        assert!(engine.calls().contains(&FakeCall::Down {
            project: "demo-prod".into(),
            volumes: true,
        }));
    }

    #[tokio::test]
    async fn a_failed_up_names_the_member_and_stops() {
        use olai_stack_runtime::FakeEngine;
        let dir = fake_workspace();
        let engine = FakeEngine::new();
        engine.fail_up("demo-dev");
        let err = run_up(&engine, lifecycle(dir.path(), &[]))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("`docker compose up -d --wait` failed for `dev`"),
            "{err}"
        );
        // `prod` comes after `dev`, so it never started.
        assert_eq!(engine.projects(), ["demo-dev".to_owned()].into());
    }

    #[tokio::test]
    async fn an_unavailable_docker_lists_as_stopped_and_fails_up_with_a_hint() {
        use olai_stack_runtime::FakeEngine;
        let dir = fake_workspace();
        let engine = FakeEngine::new();
        engine.set_unavailable(true);
        assert!(compose_projects(&engine).await.is_empty());
        let err = run_up(&engine, lifecycle(dir.path(), &["prod"]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("running docker compose"), "{err}");
    }
}
