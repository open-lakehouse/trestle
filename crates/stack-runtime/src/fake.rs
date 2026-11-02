//! [`FakeEngine`]: an in-memory [`ComposeEngine`] for tests (feature `testing`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::broadcast;

use crate::engine::{ComposeEngine, DownOptions, EngineStream, LogOptions, UpOptions};
use crate::error::{Error, Result};
use crate::events::{EngineEvent, EventAction, EventFilter};
use crate::logs::LogLine;
use crate::status::{Health, ProjectStatus, ServiceStatus, State};

/// One call the fake received, for assertions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeCall {
    /// [`up`](ComposeEngine::up) of the project `project` in `dir`, with the `env` it was given.
    Up {
        /// The project directory.
        dir: PathBuf,
        /// The project name read from its head `compose.yaml`.
        project: String,
        /// The variables passed.
        env: BTreeMap<String, String>,
    },
    /// [`down`](ComposeEngine::down) of `project`.
    Down {
        /// The project name.
        project: String,
        /// Whether volumes were removed.
        volumes: bool,
    },
    /// [`network_rm`](ComposeEngine::network_rm) of `name`.
    NetworkRm(String),
}

#[derive(Default)]
struct Inner {
    projects: BTreeMap<String, Vec<ServiceStatus>>,
    /// The directory each project was started from, as Compose reports its config files.
    dirs: BTreeMap<String, PathBuf>,
    /// The services (and their health once up) a project gets, set by the test.
    scripts: BTreeMap<String, Vec<(String, Health)>>,
    fail_up: BTreeSet<String>,
    networks: BTreeSet<String>,
    unavailable: bool,
    calls: Vec<FakeCall>,
    clock: i64,
}

/// An in-memory Docker: `up` creates a project with scripted services, `down` removes it, and
/// both raise the [events](ComposeEngine::events) Docker would. Clones share state, so a test
/// can hand one to the code under test and inspect another.
///
/// A project's name is read from the `name:` line of the head `compose.yaml` in the directory
/// `up` is given, as Compose does for the files trestle renders. Its services default to a
/// single healthy `app`; [`script`](Self::script) sets others.
#[derive(Clone)]
pub struct FakeEngine {
    inner: Arc<Mutex<Inner>>,
    events: broadcast::Sender<EngineEvent>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        FakeEngine {
            inner: Arc::default(),
            events: broadcast::channel(1024).0,
        }
    }
}

impl FakeEngine {
    /// An empty Docker host.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Give `project` these services, each reaching `health` once up.
    pub fn script(
        &self,
        project: &str,
        services: impl IntoIterator<Item = (&'static str, Health)>,
    ) {
        self.lock().scripts.insert(
            project.to_owned(),
            services
                .into_iter()
                .map(|(s, h)| (s.to_owned(), h))
                .collect(),
        );
    }

    /// Make the next `up` of `project` fail, as an unhealthy `--wait` does.
    pub fn fail_up(&self, project: &str) {
        self.lock().fail_up.insert(project.to_owned());
    }

    /// Make every call fail as if the daemon were down (or back up with `false`).
    pub fn set_unavailable(&self, unavailable: bool) {
        self.lock().unavailable = unavailable;
    }

    /// Add a project as if something else had started it (e.g. a runtime that since died).
    pub fn adopt(&self, project: &str) {
        let services = self.services_for(project);
        self.lock().projects.insert(project.to_owned(), services);
    }

    /// Declare the network `name`, as the first project joining it would.
    pub fn add_network(&self, name: &str) {
        self.lock().networks.insert(name.to_owned());
    }

    /// The networks that exist.
    pub fn networks(&self) -> BTreeSet<String> {
        self.lock().networks.clone()
    }

    /// The projects that exist.
    pub fn projects(&self) -> BTreeSet<String> {
        self.lock().projects.keys().cloned().collect()
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.lock().calls.clone()
    }

    /// Change the health of `service` in `project`, raising a health event.
    pub fn set_health(&self, project: &str, service: &str, health: Health) {
        let mut inner = self.lock();
        let Some(container) = inner.projects.get_mut(project).and_then(|services| {
            let s = services.iter_mut().find(|s| s.service == service)?;
            s.health = health;
            Some(s.container.clone())
        }) else {
            return;
        };
        let event = next_event(
            &mut inner,
            project,
            service,
            &container,
            EventAction::Health(health),
        );
        drop(inner);
        let _ = self.events.send(event);
    }

    fn services_for(&self, project: &str) -> Vec<ServiceStatus> {
        let script = self
            .lock()
            .scripts
            .get(project)
            .cloned()
            .unwrap_or_else(|| vec![("app".to_owned(), Health::Healthy)]);
        script
            .into_iter()
            .map(|(service, health)| ServiceStatus {
                container: format!("{project}-{service}-1"),
                service,
                state: State::Running,
                health,
                exit_code: 0,
                status: "Up".into(),
                published: Vec::new(),
            })
            .collect()
    }

    fn check_available(&self) -> Result<()> {
        if self.lock().unavailable {
            return Err(Error::Unavailable {
                reason: "the fake Docker daemon is down".into(),
            });
        }
        Ok(())
    }

    fn emit(&self, project: &str, services: &[ServiceStatus], actions: &[fn() -> EventAction]) {
        let mut inner = self.lock();
        let mut events = Vec::new();
        for s in services {
            for action in actions {
                events.push(next_event(
                    &mut inner,
                    project,
                    &s.service,
                    &s.container,
                    action(),
                ));
            }
        }
        drop(inner);
        for e in events {
            let _ = self.events.send(e);
        }
    }
}

fn next_event(
    inner: &mut Inner,
    project: &str,
    service: &str,
    container: &str,
    action: EventAction,
) -> EngineEvent {
    inner.clock += 1;
    EngineEvent {
        project: project.to_owned(),
        service: service.to_owned(),
        container: container.to_owned(),
        action,
        time_nanos: inner.clock,
    }
}

/// The `name:` of the head `compose.yaml` in `dir`.
fn project_in(dir: &Path) -> Result<String> {
    let path = dir.join("compose.yaml");
    let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
        command: format!("read {}", path.display()),
        source,
    })?;
    text.lines()
        .find_map(|l| l.strip_prefix("name:"))
        .map(|n| n.trim().trim_matches('"').to_owned())
        .ok_or_else(|| Error::Io {
            command: format!("read {}", path.display()),
            source: std::io::Error::other("no `name:` in the head compose.yaml"),
        })
}

#[async_trait]
impl ComposeEngine for FakeEngine {
    async fn up(&self, dir: &Path, opts: &UpOptions) -> Result<()> {
        self.check_available()?;
        let project = project_in(dir)?;
        self.lock().calls.push(FakeCall::Up {
            dir: dir.to_owned(),
            project: project.clone(),
            env: opts.env.clone(),
        });
        let services = self.services_for(&project);
        let failed = self.lock().fail_up.remove(&project);
        self.lock()
            .projects
            .insert(project.clone(), services.clone());
        self.lock().dirs.insert(project.clone(), dir.to_owned());
        self.emit(
            &project,
            &services,
            &[|| EventAction::Create, || EventAction::Start],
        );
        if failed {
            return Err(Error::Failed {
                command: "compose up -d --wait".into(),
                status: exit_status(1),
                stderr: format!("container {project}-app-1 is unhealthy"),
            });
        }
        Ok(())
    }

    async fn down(&self, dir: &Path, opts: &DownOptions) -> Result<()> {
        self.check_available()?;
        let project = project_in(dir)?;
        self.lock().calls.push(FakeCall::Down {
            project: project.clone(),
            volumes: opts.volumes,
        });
        let removed = self.lock().projects.remove(&project);
        self.lock().dirs.remove(&project);
        if let Some(services) = removed {
            self.emit(
                &project,
                &services,
                &[
                    || EventAction::Stop,
                    || EventAction::Die { exit_code: Some(0) },
                    || EventAction::Destroy,
                ],
            );
        }
        Ok(())
    }

    async fn ls(&self) -> Result<Vec<ProjectStatus>> {
        self.check_available()?;
        let inner = self.lock();
        Ok(inner
            .projects
            .iter()
            .map(|(name, services)| {
                let mut states = BTreeMap::new();
                for s in services {
                    let state = match &s.state {
                        State::Running => "running",
                        State::Exited => "exited",
                        _ => "created",
                    };
                    *states.entry(state.to_owned()).or_insert(0) += 1;
                }
                let status = states
                    .iter()
                    .map(|(s, n)| format!("{s}({n})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                ProjectStatus {
                    name: name.clone(),
                    status,
                    states,
                    config_files: inner
                        .dirs
                        .get(name)
                        .map(|d| d.join("compose.yaml").display().to_string())
                        .unwrap_or_default(),
                }
            })
            .collect())
    }

    async fn ps(&self, project: &str) -> Result<Vec<ServiceStatus>> {
        self.check_available()?;
        Ok(self
            .lock()
            .projects
            .get(project)
            .cloned()
            .unwrap_or_default())
    }

    async fn logs(&self, project: &str, _opts: &LogOptions) -> Result<EngineStream<LogLine>> {
        self.check_available()?;
        let lines: Vec<Result<LogLine>> = self
            .lock()
            .projects
            .get(project)
            .into_iter()
            .flatten()
            .map(|s| {
                Ok(LogLine {
                    container: format!("{}-1", s.service),
                    timestamp: None,
                    text: format!("{} started", s.service),
                })
            })
            .collect();
        Ok(futures::stream::iter(lines).boxed())
    }

    async fn events(&self, filter: &EventFilter) -> Result<EngineStream<EngineEvent>> {
        self.check_available()?;
        let filter = filter.clone();
        let rx = self.events.subscribe();
        let stream = futures::stream::unfold(rx, |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(event) => return Some((event, rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .filter(move |e| futures::future::ready(filter.matches(e)))
        .map(Ok);
        Ok(stream.boxed())
    }

    async fn network_rm(&self, name: &str) -> Result<bool> {
        self.check_available()?;
        let mut inner = self.lock();
        inner.calls.push(FakeCall::NetworkRm(name.to_owned()));
        Ok(inner.networks.remove(name))
    }
}

#[cfg(unix)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code as u32)
}
