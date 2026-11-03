//! Run trestle environments on Docker Compose.
//!
//! [`olai_stack_topology`] plans and renders an environment without touching Docker; this crate
//! is the part that runs it. The CLI and the desktop app share it.
//!
//! - [`ComposeEngine`]: what the runtime asks of Compose (up, down, ls, ps, logs, events,
//!   network removal). [`DockerCli`] runs the `docker` CLI; [`FakeEngine`] (feature `testing`)
//!   is an in-memory stand-in for tests.
//! - Typed [`ProjectStatus`], [`ServiceStatus`] / [`Health`] and [`EngineEvent`], parsed from the
//!   CLI's JSON output (the test fixtures are captured from real engines, see
//!   `docs/runtime.md`).
//! - [`probe_host_ports`]: whether a plan's host ports are free before starting it.
//! - [`managed_projects`] and [`remove_mesh_if_idle`]: matching what Docker runs against what
//!   the caller manages.
//! - [`Instances`]: runs the revisions of an [`olai_stack_state::Store`] as compose projects,
//!   side by side, with the layered [`EffectiveEnv`] injected and credentials resolved by
//!   [`SecretProvider`]s ([`EnvProvider`], and `KeychainProvider` behind feature `keychain`) and
//!   [`Source`] overlays.
//! - `neon` (feature `neon`): Neon branches as a source, through the Neon CLI
//!   (`NeonControl`, `NeonCli`, `NeonBranchSource`, and `FakeNeon` with `testing`).
//!
//! Values that must not reach a file (credentials) go to Compose as process environment only:
//! every lifecycle call takes the variables to add ([`UpOptions::env`]).
//!
//! ```no_run
//! use olai_stack_runtime::{ComposeEngine, DockerCli, UpOptions};
//!
//! # async fn run() -> olai_stack_runtime::Result<()> {
//! let docker = DockerCli::new();
//! let opts = UpOptions { wait: true, ..UpOptions::default() };
//! docker.up(std::path::Path::new("demo/dev"), &opts).await?;
//! for project in docker.ls().await? {
//!     println!("{}: {}", project.name, project.status);
//! }
//! # Ok(())
//! # }
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

mod docker;
mod effective;
mod engine;
mod error;
mod events;
#[cfg(feature = "testing")]
mod fake;
mod instances;
mod logs;
#[cfg(feature = "neon")]
#[cfg_attr(docsrs, doc(cfg(feature = "neon")))]
pub mod neon;
mod ports;
mod reconcile;
mod secrets;
mod source;
mod status;

pub use docker::DockerCli;
pub use effective::{
    EffectiveEnv, EnvLayers, EnvValue, Layer, LayerValue, MASK, MaskedVar, Resolved,
};
pub use engine::{ComposeEngine, DownOptions, EngineStream, LogOptions, Progress, UpOptions};
pub use error::{Error, Result};
pub use events::{
    EngineEvent, EventAction, EventFilter, PROJECT_LABEL, SERVICE_LABEL, parse_event,
};
#[cfg(feature = "testing")]
#[cfg_attr(docsrs, doc(cfg(feature = "testing")))]
pub use fake::{FakeCall, FakeEngine};
pub use instances::{
    INSTANCES_DIR, InstanceError, InstanceEvent, Instances, Materialized, PLACEMENT_KEYS,
    ReconcileOptions, Reconciled, instance_project, parse_instance_project,
};
pub use logs::{LogLine, parse_log_line};
pub use ports::{BusyPort, is_free, probe_host_ports};
pub use reconcile::{ProjectMatch, managed_projects, remove_mesh_if_idle};
#[cfg(feature = "keychain")]
#[cfg_attr(docsrs, doc(cfg(feature = "keychain")))]
pub use secrets::KeychainProvider;
pub use secrets::{EnvProvider, SecretError, SecretProvider, SecretProviders, SecretValue};
pub use source::{Overlay, Source, SourceCtx, SourceError, UserVars};
pub use status::{Health, ProjectStatus, Published, ServiceStatus, State, parse_ls, parse_ps};
