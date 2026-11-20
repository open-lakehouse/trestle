//! The shared trestle state store.
//!
//! One SQLite database under the [trestle home](trestle_home) (`$TRESTLE_HOME`, else the
//! platform data directory), shared by the `trestle` CLI and the desktop app, and optionally
//! team databases attached to it (see [Sharing with a team](#sharing-with-a-team)). It holds:
//!
//! - [`Workspace`]s: sets of environments sharing a host, with their [`VarSet`]s.
//! - [`Environment`]s: an editable [`Draft`] (the placement-free `env.toml`, the
//!   `module.yaml` texts it lists, vars) and a head.
//! - [`Revision`]s: numbered, immutable snapshots of a draft with a digest, the time they
//!   were committed and an optional message. Nothing in the API changes one, and the
//!   database refuses an update.
//! - [`Instance`]s: a revision to run, whether it should run, overrides and the overlay
//!   [sources](SourceBinding) it takes.
//! - [`SlotLease`]s: host-wide, so two instances never share a slot whatever their
//!   workspace (slots per workspace clashed across workspaces).
//!
//! Secrets are stored as [`SecretRef`]s only; their values stay with their provider.
//!
//! Workspaces and environments are renamed by id ([`Store::rename_workspace`],
//! [`Store::rename_environment`]), keeping their revisions. A rename, like a change of the
//! workspace's mesh ([`Store::set_mesh`]), waits until no instance of it is left
//! ([`Error::HasInstances`]): Docker holds an instance's containers and volumes under names
//! derived from the workspace and environment names.
//!
//! Several processes use the store at once: writes are serialized (a writer waits for
//! another's), and a [`Watcher`] notices commits made elsewhere. A store with a newer schema
//! than this crate knows is refused ([`Error::UnsupportedVersion`]).
//!
//! # Sharing with a team
//!
//! With the `postgres` feature, a team keeps its workspaces, environments, drafts and
//! revisions in a Postgres database (on a Neon project, say), and each machine attaches it
//! ([`Store::attach`], [`Store::open_attached`]). What each machine runs stays in its own
//! `state.db`: instances, slot leases, and its own vars ([`VarScope::Local`]), such as
//! the minted keys sealing its instances' data and the user's own API keys. The attachment
//! records where the database is ([`RemoteLocation`]), never a credential: each user
//! connects with their own login.
//!
//! Colleagues editing one draft don't lose each other's edits: [`Store::update_draft_if`]
//! refuses with [`Error::Conflict`] when the draft changed since it was read, and a
//! [`Watcher`] on an attached store notices their commits. Opening an attached store never
//! migrates its database ([`Store::init_remote`] does).
//!
//! `trestle-workspace.toml` and environment directories stay as an exchange format:
//! [`Store::import_workspace`] drops placement (the store leases slots itself), and
//! [`Store::export_instances`] and [`Store::export_workspace`] write slots, so the exported
//! directory runs with `docker compose up` like any workspace.
//!
//! ```no_run
//! use olai_stack_state::{Draft, Store};
//! use olai_stack_topology::{EnvManifest, PlanCtx, Selection};
//!
//! # fn run() -> olai_stack_state::Result<()> {
//! let store = Store::open_default()?;
//! let ws = store.create_workspace("demo", false)?;
//! let manifest = EnvManifest::new(Selection::modules(["unity-catalog-oss"]), PlanCtx::default());
//! let env = store.create_environment(&ws.id, "dev", Draft::new(manifest))?;
//! let rev = store.commit_draft(&env.id)?;
//! let instance = store.create_instance(&env.id, rev.number())?;
//! let lease = store.lease_slot(&instance.id)?;
//! println!("dev r{} on slot {}", rev.number(), lease.slot);
//! # Ok(())
//! # }
//! ```

mod error;
mod model;
#[cfg(feature = "postgres")]
mod pg;
mod sql;
mod store;
mod transfer;
mod watch;

pub use error::{Error, Result};
pub use model::{
    Attachment, AttachmentId, Desired, Draft, Environment, EnvironmentId, Instance, InstanceId,
    RemoteLocation, Revision, SecretRef, SlotLease, SourceBinding, VarScope, VarSet, VarValue,
    Workspace, WorkspaceId,
};
#[cfg(feature = "postgres")]
pub use pg::REMOTE_SCHEMA_VERSION;
pub use store::{DB_FILE, SCHEMA_VERSION, Store, TRESTLE_HOME_VAR, trestle_home};
pub use transfer::{ENV_FILE, read_env_dir};
pub use watch::Watcher;
