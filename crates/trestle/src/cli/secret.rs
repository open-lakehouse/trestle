//! `trestle secret rotate`: a new value for a secret the secret server holds (an egress route's
//! credential, or a secret a module reads over SDS), handed to the running instances without
//! restarting the egress gateway or the module.
//!
//! The value is read from stdin (or asked for), never from the command line, and stored as
//! `trestle var set <ws>/<env> --secret <NAME>` would: in the keychain, on the environment, or on
//! this machine's own vars on a team's state (`--state`). Each running instance of the
//! environment (or only `@<n>`'s) then gets its `secret-server` recreated with the new value
//! (`docker compose up -d --no-deps --force-recreate secret-server`). The egress Envoy's SDS
//! stream reconnects and it swaps the value in, as does a module's SDS client; nothing else
//! restarts.

use clap::{Args, Subcommand};
use olai_stack_runtime::Layer;
use olai_stack_state::{Instance, VarScope};
use olai_stack_topology::plan::egress::SECRET_SERVER_SERVICE;

use super::state::{Ctx, project_of, running_projects};
use super::target::{Rev, Target, revision};
use crate::error::{Error, Result};

/// The `trestle secret` subcommand group.
#[derive(Subcommand)]
pub enum SecretCommand {
    /// Store a new value for a secret the secret server holds (an egress route's credential,
    /// or a module's SDS secret) and recreate the secret server of every running instance with
    /// it (the value is read from stdin, or asked for).
    Rotate(RotateArgs),
}

#[derive(Args, Clone)]
pub struct RotateArgs {
    /// `<ws>/<env>` (its running instances) or `<ws>/<env>@<n>` (revision n's).
    pub target: Target,

    /// The variable holding the secret: a route's `credential.secret`, or the `env` of a
    /// module's SDS secret.
    pub name: String,

    /// Store it on this machine only, as `var set --local` does.
    #[clap(long, conflicts_with = "shared")]
    pub local: bool,

    /// On a team's state, store it in the shared vars, as `var set --shared` does.
    #[clap(long)]
    pub shared: bool,
}

pub fn run(cmd: SecretCommand) -> Result<()> {
    let ctx = Ctx::open()?;
    match cmd {
        SecretCommand::Rotate(args) => run_rotate(&ctx, args),
    }
}

fn run_rotate(ctx: &Ctx, args: RotateArgs) -> Result<()> {
    let name = &args.name;
    if name.contains('=') {
        let key = name.split('=').next().unwrap_or_default();
        return Err(Error::other(
            "a secret's value is read from stdin, not given on the command line",
        )
        .with_hint(format!(
            "`printf %s \"$VALUE\" | trestle secret rotate {} {key}`",
            args.target
        )));
    }
    super::var::check_key(name)?;
    super::key::refuse_keyring_var(name, "trestle secret rotate")?;
    if args.target.env.is_none() {
        return Err(Error::other(
            "a secret the secret server holds belongs to an environment (<ws>/<env>)",
        ));
    }
    if args.target.rev == Some(Rev::Draft) {
        return Err(Error::other("the draft doesn't run; rotate on <ws>/<env>"));
    }
    // The value is stored on the environment whichever revision is named.
    let env_target = Target {
        rev: None,
        ..args.target.clone()
    };
    let (scope, prefix, local) =
        super::var::secret_scope(ctx, &env_target, name, args.local, args.shared)?;

    let running = running_instances(ctx, &args.target)?;
    check_route_secret(ctx, &args.target, &running, name)?;
    let runtime = ctx.instances();
    // A value stored where a higher layer overrides it would never reach the secret server.
    let stored_at = match scope {
        VarScope::Local(_) => Layer::LocalVars,
        _ => Layer::EnvironmentVars,
    };
    for instance in &running {
        let effective = ctx.block_on(async { Ok(runtime.effective_env(&instance.id).await?) })?;
        if let Some(var) = effective.masked().get(name.as_str())
            && var.provenance > stored_at
        {
            return Err(Error::other(format!(
                "`{name}` of {} is set by its {}, which override where this would store it",
                project_of(&ctx.store, instance)?,
                layer_name(&var.provenance)
            ))
            .with_hint("rotate it where it is set, or remove it there first"));
        }
    }

    let value = super::var::read_secret(name)?;
    super::var::store_secret(ctx, &env_target, (&scope, &prefix), name, &value)?;
    println!(
        "stored {name} on {env_target}{}",
        if local { " (this machine only)" } else { "" }
    );
    if running.is_empty() {
        println!(
            "no instance of {} is running; the next `trestle up` uses it",
            args.target
        );
        return Ok(());
    }
    recreate_secret_server(ctx, &running)
}

/// Recreate `secret-server` in each of `running`, with the launch environment it resolves to
/// now.
pub(super) fn recreate_secret_server(ctx: &Ctx, running: &[Instance]) -> Result<()> {
    let runtime = ctx.instances();
    for instance in running {
        ctx.block_on(async {
            Ok(runtime
                .recreate_service(&instance.id, SECRET_SERVER_SERVICE)
                .await?)
        })?;
        println!(
            "recreated {SECRET_SERVER_SERVICE} in {}",
            project_of(&ctx.store, instance)?
        );
    }
    Ok(())
}

/// The instances `target` names whose compose project runs.
pub(super) fn running_instances(ctx: &Ctx, target: &Target) -> Result<Vec<Instance>> {
    let instances = super::run::instances_of(ctx, target)?;
    let projects = ctx.block_on(running_projects(ctx))?;
    let mut out = Vec::new();
    for instance in instances {
        let project = project_of(&ctx.store, &instance)?;
        if projects.iter().any(|p| p.name == project && p.is_running()) {
            out.push(instance);
        }
    }
    Ok(out)
}

/// `name` must be a secret the secret server holds: of a running instance's revision, or (when
/// none runs) of the revision `target` names (the head without `@`).
fn check_route_secret(ctx: &Ctx, target: &Target, running: &[Instance], name: &str) -> Result<()> {
    let (_, env) = target.environment(&ctx.store)?;
    let mut numbers: Vec<u32> = running.iter().map(|i| i.revision).collect();
    if numbers.is_empty() {
        numbers.push(revision(&ctx.store, &env, target.rev)?.number());
    }
    numbers.dedup();
    for number in &numbers {
        let rev = revision(&ctx.store, &env, Some(Rev::Number(*number)))?;
        if served_secrets(&rev)?.iter().any(|s| s == name) {
            return Ok(());
        }
    }
    let revs: Vec<String> = numbers.iter().map(|n| format!("r{n}")).collect();
    Err(Error::other(format!(
        "`{name}` is not a secret the secret server holds in {target} ({})",
        revs.join(", ")
    ))
    .with_hint(format!(
        "rotation recreates the secret server, which only holds egress route credentials and \
         modules' SDS secrets; set other secrets with `trestle var set {target} --secret {name}`"
    )))
}

/// The variables the revision's secret server holds: the egress routes' credentials and the
/// modules' SDS secrets.
fn served_secrets(rev: &olai_stack_state::Revision) -> Result<Vec<String>> {
    let catalog = super::env_store::catalog_for(rev.modules())?;
    let plan = rev
        .manifest()
        .plan(&catalog)
        .map_err(super::env::map_plan_err)?;
    Ok(plan.egress.map(|e| e.secrets).unwrap_or_default())
}

/// How `env show --effective` names a layer (`revision_vars`, `source:neon`, …).
fn layer_name(layer: &Layer) -> String {
    let value = serde_json::to_value(layer).unwrap_or_default();
    match (value["layer"].as_str(), value["name"].as_str()) {
        (Some(layer), Some(name)) => format!("{layer}:{name}"),
        (Some(layer), None) => layer.to_owned(),
        _ => format!("{layer:?}"),
    }
}
