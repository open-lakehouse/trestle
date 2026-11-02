//! The [`ComposeEngine`] trait: what the runtime asks of Docker Compose.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::error::Result;
use crate::events::{EngineEvent, EventFilter};
use crate::logs::LogLine;
use crate::status::{ProjectStatus, ServiceStatus};

/// Where a command's progress output goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Progress {
    /// To this process's stdout/stderr, as if the user ran `docker compose` themselves.
    #[default]
    Inherit,
    /// Captured: stdout is dropped, stderr is kept for the error.
    Quiet,
}

/// How to [`up`](ComposeEngine::up) a project.
#[derive(Clone, Debug, Default)]
pub struct UpOptions {
    /// Wait until every service runs and passes its healthcheck (`--wait`).
    pub wait: bool,
    /// Give up waiting after this long (`--wait-timeout`).
    pub wait_timeout: Option<Duration>,
    /// Variables added to the command's environment, which Compose interpolates `${VAR}`
    /// references from. Values reach Compose this way only, never through a file.
    pub env: BTreeMap<String, String>,
    /// Where progress output goes.
    pub progress: Progress,
}

/// How to [`down`](ComposeEngine::down) a project.
#[derive(Clone, Debug, Default)]
pub struct DownOptions {
    /// Also remove the project's named volumes (`--volumes`).
    pub volumes: bool,
    /// Variables added to the command's environment (Compose interpolates the project's
    /// files on `down` too, so the ones `up` needed must be present).
    pub env: BTreeMap<String, String>,
    /// Where progress output goes.
    pub progress: Progress,
}

/// How to read a project's [`logs`](ComposeEngine::logs).
#[derive(Clone, Debug, Default)]
pub struct LogOptions {
    /// Keep streaming new lines (`--follow`).
    pub follow: bool,
    /// Only the last this many lines per container (`--tail`).
    pub tail: Option<u32>,
    /// Only these services (all of them when empty).
    pub services: Vec<String>,
}

/// A stream of items from a long-running command; dropping it stops the command.
pub type EngineStream<T> = BoxStream<'static, Result<T>>;

/// Docker Compose, as the runtime drives it. [`DockerCli`](crate::DockerCli) runs the real
/// CLI; [`FakeEngine`](crate::FakeEngine) (feature `testing`) is an in-memory stand-in.
///
/// A project is addressed by the directory holding its head `compose.yaml` for the commands
/// that read its files (`up`, `down`), and by its name for the ones Docker answers from its
/// labels (`ps`, `logs`).
#[async_trait]
pub trait ComposeEngine: Send + Sync {
    /// Create and start the project in `dir` (`docker compose up -d`).
    async fn up(&self, dir: &Path, opts: &UpOptions) -> Result<()>;

    /// Stop and remove the project in `dir` (`docker compose down`).
    async fn down(&self, dir: &Path, opts: &DownOptions) -> Result<()>;

    /// Every project Docker knows of, running or stopped (`docker compose ls --all`).
    async fn ls(&self) -> Result<Vec<ProjectStatus>>;

    /// The containers of `project`, running or stopped (`docker compose ps --all`).
    async fn ps(&self, project: &str) -> Result<Vec<ServiceStatus>>;

    /// The log lines of `project`'s containers (`docker compose logs --timestamps`).
    async fn logs(&self, project: &str, opts: &LogOptions) -> Result<EngineStream<LogLine>>;

    /// Container events of the projects `filter` picks, from now on (`docker events`).
    async fn events(&self, filter: &EventFilter) -> Result<EngineStream<EngineEvent>>;

    /// Remove the network `name`; `Ok(false)` when it doesn't exist or is still in use.
    async fn network_rm(&self, name: &str) -> Result<bool>;
}
