//! The shared trestle state store.
//!
//! One SQLite database under the [trestle home](trestle_home) (`$TRESTLE_HOME`, else the
//! platform data directory), shared by the `trestle` CLI and the desktop app. It holds:
//!
//! - [`Workspace`]s: sets of environments sharing a host, with their [`VarSet`]s.
//! - [`Environment`]s: an editable [`Draft`] (the placement-free `env.toml`, the
//!   `module.yaml` texts it lists, vars) and a head.
//! - [`Revision`]s: numbered, immutable snapshots of a draft with a digest. Nothing in the
//!   API changes one, and the database refuses an update.
//! - [`Instance`]s: a revision to run, whether it should run, overrides and the overlay
//!   [sources](SourceBinding) it takes.
//! - [`SlotLease`]s: host-wide, so two instances never share a slot whatever their
//!   workspace (slots per workspace clashed across workspaces).
//!
//! Secrets are stored as [`SecretRef`]s only; their values stay with their provider.
//!
//! Several processes use the store at once: writes are serialized (a writer waits for
//! another's), and a [`Watcher`] notices commits made elsewhere. A store with a newer schema
//! than this crate knows is refused ([`Error::UnsupportedVersion`]).
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
//! let manifest = EnvManifest::new(Selection::modules(["unity-catalog"]), PlanCtx::default());
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
mod store;
mod transfer;
mod watch;

pub use error::{Error, Result};
pub use model::{
    Desired, Draft, Environment, EnvironmentId, Instance, InstanceId, Revision, SecretRef,
    SlotLease, SourceBinding, VarScope, VarSet, VarValue, Workspace, WorkspaceId,
};
pub use store::{DB_FILE, SCHEMA_VERSION, Store, TRESTLE_HOME_VAR, trestle_home};
pub use transfer::{ENV_FILE, read_env_dir};
pub use watch::Watcher;
