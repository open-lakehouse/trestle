//! `trestle state`: a team's shared state, attached on this machine.
//!
//! A team keeps its workspaces and environments (drafts, revisions, vars) in a database on
//! its Neon project, so colleagues see each other's edits. `state init` creates it, `state
//! attach` records it on a machine, and `--state <name>` (or `TRESTLE_STATE`) points any
//! store-backed command at it. Instances, slots and this machine's own vars
//! (`var set --local`) stay in the local store. Nothing stores a credential: every
//! connection is made with the user's own Neon CLI login.

use clap::{Args, Subcommand};
use olai_stack_runtime::neon::{self, NeonCli, StateError};
use olai_stack_state::{Attachment, RemoteLocation, Store};
use serde::Serialize;

use super::output::emit;
use super::state::{block_on, local_store, neon_err};
use crate::error::{Error, Result};

/// The `trestle state` subcommand group.
#[derive(Subcommand)]
pub enum StateCommand {
    /// Set up a team's shared state on a Neon project (create the `trestle_state` database,
    /// create or migrate its schema) and attach it here. Safe to run again, e.g. after an
    /// upgrade to migrate the schema.
    Init(NeonArgs),
    /// Attach a team's shared state that a colleague set up, so `--state <name>` works on it.
    Attach(NeonArgs),
    /// Forget an attached state on this machine (the team's database is untouched).
    Detach(DetachArgs),
    /// List the attached states.
    List(ListArgs),
}

#[derive(Args, Clone)]
pub struct NeonArgs {
    /// The Neon project id.
    #[clap(long, value_name = "PROJECT")]
    pub neon: String,

    /// The branch holding the state (its name or id); the project's default branch when
    /// omitted.
    #[clap(long)]
    pub branch: Option<String>,

    /// What to call it on this machine (`--state <name>`); the project id by default.
    #[clap(long)]
    pub name: Option<String>,
}

#[derive(Args, Clone)]
pub struct DetachArgs {
    /// The attached state's name.
    pub name: String,
}

#[derive(Args, Clone)]
pub struct ListArgs {
    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

pub fn run(cmd: StateCommand) -> Result<()> {
    match cmd {
        StateCommand::Init(args) => run_init(args),
        StateCommand::Attach(args) => run_attach(args),
        StateCommand::Detach(args) => run_detach(args),
        StateCommand::List(args) => run_list(args),
    }
}

/// `e` as a CLI error, with a hint where there's one to give.
pub(super) fn state_err(e: StateError) -> Error {
    match e {
        StateError::Neon(e) => neon_err(e),
        StateError::Store(e) => store_err(e),
    }
}

/// `e` as a CLI error, with a hint for an attached store that isn't ready.
pub(super) fn store_err(e: olai_stack_state::Error) -> Error {
    let hint = match &e {
        olai_stack_state::Error::Uninitialized | olai_stack_state::Error::Outdated { .. } => {
            Some("`trestle state init --neon <project>` sets it up or migrates it")
        }
        olai_stack_state::Error::UnsupportedVersion { .. } => Some("upgrade trestle"),
        olai_stack_state::Error::Conflict { .. } => {
            Some("someone else edited it meanwhile; run the command again")
        }
        _ => None,
    };
    match hint {
        Some(h) => Error::from(e).with_hint(h),
        None => e.into(),
    }
}

/// The name to attach `args`' project as.
fn attachment_name(args: &NeonArgs) -> String {
    args.name.clone().unwrap_or_else(|| args.neon.clone())
}

/// The attachment of `location`, if this machine has one.
fn attached_at(local: &Store, location: &RemoteLocation) -> Result<Option<Attachment>> {
    Ok(local
        .attachments()?
        .into_iter()
        .find(|a| a.location == *location))
}

fn run_init(args: NeonArgs) -> Result<()> {
    let local = local_store()?;
    let cli = NeonCli::new().map_err(neon_err)?;
    let location = block_on(async {
        neon::init_state(&cli, &args.neon, args.branch.as_deref())
            .await
            .map_err(state_err)
    })?;
    println!("set up the shared state at {location}");
    match attached_at(&local, &location)? {
        Some(existing) => println!("already attached here as `{}`", existing.name),
        None => attach(&local, &cli, &attachment_name(&args), location)?,
    }
    Ok(())
}

fn run_attach(args: NeonArgs) -> Result<()> {
    let local = local_store()?;
    let cli = NeonCli::new().map_err(neon_err)?;
    let location = neon::state_location(&args.neon, args.branch.as_deref());
    if let Some(existing) = attached_at(&local, &location)? {
        return Err(Error::other(format!(
            "{location} is already attached as `{}`",
            existing.name
        )));
    }
    attach(&local, &cli, &attachment_name(&args), location)
}

fn attach(local: &Store, cli: &NeonCli, name: &str, location: RemoteLocation) -> Result<()> {
    let attachment = block_on(async {
        neon::attach(cli, local, name, location)
            .await
            .map_err(state_err)
    })?;
    println!(
        "attached {} as `{}`: pass `--state {}` (or set TRESTLE_STATE) to work on it",
        attachment.location, attachment.name, attachment.name
    );
    Ok(())
}

fn run_detach(args: DetachArgs) -> Result<()> {
    let local = local_store()?;
    let attachment = find(&local, &args.name)?;
    local.detach(&attachment.id).map_err(|e| {
        let blocked = matches!(e, olai_stack_state::Error::HasInstances { .. });
        let e = Error::from(e);
        if blocked {
            e.with_hint(format!(
                "`trestle --state {} down <ws> --rm` first",
                attachment.name
            ))
        } else {
            e
        }
    })?;
    println!("detached `{}`", args.name);
    Ok(())
}

/// The attachment called `name`.
pub(super) fn find(local: &Store, name: &str) -> Result<Attachment> {
    local
        .attachments()?
        .into_iter()
        .find(|a| a.name == name)
        .ok_or_else(|| {
            Error::other(format!("no shared state `{name}` is attached here")).with_hint(
                "`trestle state list` shows what is; `trestle state attach --neon <project>` \
                 attaches one",
            )
        })
}

#[derive(Serialize)]
struct Row {
    name: String,
    location: RemoteLocation,
}

fn run_list(args: ListArgs) -> Result<()> {
    let rows: Vec<Row> = local_store()?
        .attachments()?
        .into_iter()
        .map(|a| Row {
            name: a.name,
            location: a.location,
        })
        .collect();
    emit(args.json, "states", &rows, |rows| {
        if rows.is_empty() {
            println!("no shared state attached; this machine's own is the only one");
        }
        for r in rows {
            println!("{:<20} {}", r.name, r.location);
        }
    })
}
