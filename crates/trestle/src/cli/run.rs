//! `trestle up` / `down` / `ps` / `logs` / `health`: run the store's revisions as instances.
//!
//! `up <ws>/<env>[@<rev>]` starts an instance of the revision (the head by default), creating
//! it the first time: its own compose project (`<ws>_<env>_r<n>`), slot and host label, so
//! two revisions of one environment run side by side. The rendered tree lives under
//! `$TRESTLE_HOME/instances/` and holds no credential; they reach Compose as process
//! environment.

use std::collections::BTreeMap;

use clap::Args;
use futures::StreamExt;
use olai_stack_runtime::{Health, LogOptions, remove_mesh_if_idle};
use olai_stack_state::{Desired, Instance};
use serde::Serialize;

use super::output::emit;
use super::state::{Ctx, project_of, running_projects};
use super::target::{Rev, Target, revision};
use crate::error::{Error, Result};

#[derive(Args, Clone)]
pub struct UpArgs {
    /// `<ws>/<env>[@<rev>]` (the head revision by default).
    pub target: Target,
}

#[derive(Args, Clone)]
pub struct DownArgs {
    /// `<ws>` (all its instances), `<ws>/<env>` (the environment's) or `<ws>/<env>@<rev>`.
    pub target: Target,

    /// Also remove the named volumes (`docker compose down --volumes`).
    #[clap(long)]
    pub volumes: bool,

    /// Also delete the instances, releasing their slots.
    #[clap(long)]
    pub rm: bool,
}

#[derive(Args, Clone)]
pub struct PsArgs {
    /// Only this workspace, environment or revision.
    pub target: Option<Target>,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct LogsArgs {
    /// `<ws>/<env>[@<rev>]`.
    pub target: Target,

    /// Keep printing new lines.
    #[clap(long, short)]
    pub follow: bool,

    /// Only the last this many lines per container.
    #[clap(long)]
    pub tail: Option<u32>,

    /// Only these services.
    #[clap(long = "service", short = 's')]
    pub services: Vec<String>,
}

#[derive(Args, Clone)]
pub struct HealthArgs {
    /// `<ws>/<env>[@<rev>]`.
    pub target: Target,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

/// The instances `target` names: a workspace's, an environment's, or one revision's.
fn instances_of(ctx: &Ctx, target: &Target) -> Result<Vec<Instance>> {
    let ws = target.workspace(&ctx.store)?;
    let envs = match &target.env {
        None => ctx.store.environments(&ws.id)?,
        Some(_) => vec![target.environment(&ctx.store)?.1],
    };
    let mut out = Vec::new();
    for env in envs {
        let mut instances = ctx.store.instances_of(&env.id)?;
        match target.rev {
            None => {}
            Some(Rev::Draft) => return Err(Error::other("the draft doesn't run; commit it")),
            rev => {
                let number = revision(&ctx.store, &env, rev)?.number();
                instances.retain(|i| i.revision == number);
            }
        }
        out.extend(instances);
    }
    Ok(out)
}

/// The one instance `target` names (the revision's, the head's without `@`): the one wanted
/// running, else the oldest.
fn instance_for(ctx: &Ctx, target: &Target) -> Result<Instance> {
    let (_, env) = target.environment(&ctx.store)?;
    let number = revision(&ctx.store, &env, target.rev.or(Some(Rev::Head)))?.number();
    let mut instances: Vec<Instance> = ctx
        .store
        .instances_of(&env.id)?
        .into_iter()
        .filter(|i| i.revision == number)
        .collect();
    instances.sort_by_key(|i| i.desired != Desired::Running);
    instances.into_iter().next().ok_or_else(|| {
        Error::other(format!("no instance of {target} (r{number})"))
            .with_hint(format!("`trestle up {target}`"))
    })
}

pub fn run_up(args: UpArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let (ws, env) = args.target.environment(&ctx.store)?;
    let rev = revision(&ctx.store, &env, args.target.rev)?;
    if args.target.rev.is_none() && super::env_store::draft_differs(&env, &rev)? {
        eprintln!(
            "note: the draft of {}/{} has uncommitted changes; running r{} (`trestle env commit` \
             to run them)",
            ws.name,
            env.name,
            rev.number()
        );
    }
    let existing = ctx
        .store
        .instances_of(&env.id)?
        .into_iter()
        .filter(|i| i.revision == rev.number())
        .min_by_key(|i| i.desired != Desired::Running);
    let id = match existing {
        Some(i) => i.id,
        None => ctx.store.create_instance(&env.id, rev.number())?.id,
    };
    let runtime = ctx.instances();
    let materialized = ctx.block_on(async { Ok(runtime.start(&id).await?) })?;
    let placement = runtime.placement(&id)?;
    println!(
        "{}/{} r{} is up as {} (slot {})",
        ws.name,
        env.name,
        rev.number(),
        placement.project_name,
        placement.slot
    );
    if let Some(url) = materialized.plan.gateway_url() {
        println!("  gateway: {url}");
    }
    Ok(())
}

pub fn run_down(args: DownArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let instances = instances_of(&ctx, &args.target)?;
    if instances.is_empty() {
        println!("no instances of {}", args.target);
        return Ok(());
    }
    let runtime = ctx.instances();
    ctx.block_on(async {
        let projects = running_projects(&ctx).await?;
        for instance in &instances {
            let project = project_of(&ctx.store, instance)?;
            let known = projects.iter().any(|p| p.name == project);
            if args.rm {
                runtime.remove(&instance.id, args.volumes).await?;
                println!("removed {project}");
            } else if known || instance.desired == Desired::Running {
                runtime.stop(&instance.id, args.volumes).await?;
                println!("stopped {project}");
            }
        }
        // The mesh network goes with the workspace's last running instance.
        let ws = args.target.workspace(&ctx.store)?;
        if ws.mesh {
            let mut members = std::collections::BTreeSet::new();
            for env in ctx.store.environments(&ws.id)? {
                for instance in ctx.store.instances_of(&env.id)? {
                    members.insert(project_of(&ctx.store, &instance)?);
                }
            }
            let network = format!("trestle-{}-mesh", ws.name);
            if remove_mesh_if_idle(ctx.engine(), &network, &members).await? {
                println!("removed network {network}");
            }
        }
        Ok(())
    })
}

/// An instance in `ps`.
#[derive(Serialize)]
struct PsRow {
    workspace: String,
    environment: String,
    revision: u32,
    instance: String,
    project: String,
    slot: Option<u16>,
    desired: Desired,
    /// Containers per state, as Docker reports them (empty when it doesn't know the project).
    containers: BTreeMap<String, u32>,
}

pub fn run_ps(args: PsArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let instances = match &args.target {
        Some(t) => instances_of(&ctx, t)?,
        None => ctx.store.instances()?,
    };
    let projects = ctx.block_on(running_projects(&ctx))?;
    let mut rows = Vec::new();
    for i in instances {
        let env = ctx.store.environment(&i.environment)?;
        let ws = ctx.store.workspace_by_id(&env.workspace)?;
        let project = project_of(&ctx.store, &i)?;
        let containers = projects
            .iter()
            .find(|p| p.name == project)
            .map(|p| p.states.clone())
            .unwrap_or_default();
        rows.push(PsRow {
            workspace: ws.name,
            environment: env.name,
            revision: i.revision,
            instance: i.id.0,
            project,
            slot: i.slot,
            desired: i.desired,
            containers,
        });
    }
    rows.sort_by(|a, b| {
        (&a.workspace, &a.environment, a.revision, &a.instance).cmp(&(
            &b.workspace,
            &b.environment,
            b.revision,
            &b.instance,
        ))
    });
    emit(args.json, "instances", &rows, |rows| {
        if rows.is_empty() {
            println!("no instances; `trestle up <ws>/<env>`");
            return;
        }
        println!(
            "{:<28} {:>4} {:<8} CONTAINERS",
            "INSTANCE", "SLOT", "WANTED"
        );
        for r in rows {
            let containers = if r.containers.is_empty() {
                "-".to_owned()
            } else {
                r.containers
                    .iter()
                    .map(|(s, n)| format!("{s}({n})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let desired = match r.desired {
                Desired::Running => "running",
                Desired::Stopped => "stopped",
            };
            println!(
                "{:<28} {:>4} {desired:<8} {containers}",
                format!("{}/{}@{}", r.workspace, r.environment, r.revision),
                r.slot.map_or("-".to_owned(), |s| s.to_string()),
            );
        }
    })
}

pub fn run_logs(args: LogsArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let instance = instance_for(&ctx, &args.target)?;
    let project = project_of(&ctx.store, &instance)?;
    let opts = LogOptions {
        follow: args.follow,
        tail: args.tail,
        services: args.services,
    };
    ctx.block_on(async {
        let mut lines =
            olai_stack_runtime::ComposeEngine::logs(ctx.engine(), &project, &opts).await?;
        while let Some(line) = lines.next().await {
            let line = line?;
            match (line.container.is_empty(), &line.timestamp) {
                (true, _) => println!("{}", line.text),
                (false, Some(ts)) => println!("{} | {ts} {}", line.container, line.text),
                (false, None) => println!("{} | {}", line.container, line.text),
            }
        }
        Ok(())
    })
}

/// A service in `health`.
#[derive(Serialize)]
struct HealthDoc {
    instance: String,
    project: String,
    services: BTreeMap<String, Health>,
}

pub fn run_health(args: HealthArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let instance = instance_for(&ctx, &args.target)?;
    let project = project_of(&ctx.store, &instance)?;
    let services = ctx.block_on(async { Ok(ctx.instances().health(&instance.id).await?) })?;
    let doc = HealthDoc {
        instance: instance.id.0,
        project,
        services,
    };
    emit(args.json, "health", &doc, |d| {
        if d.services.is_empty() {
            println!("{} is not running", d.project);
        }
        for (service, health) in &d.services {
            let h = match health {
                Health::None => "no healthcheck",
                Health::Starting => "starting",
                Health::Healthy => "healthy",
                Health::Unhealthy => "unhealthy",
            };
            println!("{service:<24} {h}");
        }
    })
}
