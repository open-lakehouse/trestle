//! [`DockerCli`]: the [`ComposeEngine`] that runs the `docker` CLI.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::engine::{ComposeEngine, DownOptions, EngineStream, LogOptions, Progress, UpOptions};
use crate::error::{Error, Result};
use crate::events::{EngineEvent, EventFilter, PROJECT_LABEL, parse_event};
use crate::logs::{LogLine, parse_log_line};
use crate::status::{ProjectStatus, ServiceStatus, parse_ls, parse_ps};

/// Runs the `docker` CLI (with the Compose plugin). It honours `DOCKER_HOST` and the current
/// Docker context like the CLI does; [`with_context`](Self::with_context) pins one.
#[derive(Clone, Debug)]
pub struct DockerCli {
    program: PathBuf,
    context: Option<String>,
}

impl Default for DockerCli {
    fn default() -> Self {
        DockerCli {
            program: PathBuf::from("docker"),
            context: None,
        }
    }
}

/// What the CLI prints when it can't reach a daemon (Docker CLI and Compose wordings).
const UNREACHABLE: &[&str] = &[
    "Cannot connect to the Docker daemon",
    "failed to connect to the docker API",
    "error during connect",
    "Is the docker daemon running",
];

impl DockerCli {
    /// The `docker` on `PATH`, under the current context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `program` instead of `docker` (e.g. an absolute path).
    pub fn with_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.program = program.into();
        self
    }

    /// Talk to the daemon of Docker context `context` (`docker --context <context>`).
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.program);
        if let Some(context) = &self.context {
            cmd.args(["--context", context]);
        }
        cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
        cmd
    }

    /// Run `args` to completion, capturing stdout; a failure becomes an [`Error`].
    async fn output(&self, args: &[&str]) -> Result<String> {
        let line = args.join(" ");
        let out = self
            .command(args)
            .output()
            .await
            .map_err(|e| spawn_error(&line, e))?;
        if !out.status.success() {
            return Err(failure(line, out.status, &out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run a compose lifecycle command in `dir`, with `env` added and progress as asked.
    async fn lifecycle(
        &self,
        dir: &Path,
        args: &[&str],
        env: &std::collections::BTreeMap<String, String>,
        progress: Progress,
    ) -> Result<()> {
        let line = args.join(" ");
        let mut cmd = self.command(args);
        cmd.current_dir(dir).envs(env);
        match progress {
            Progress::Inherit => {
                cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
                let status = cmd.status().await.map_err(|e| spawn_error(&line, e))?;
                if !status.success() {
                    return Err(failure(line, status, &[]));
                }
            }
            Progress::Quiet => {
                let out = cmd.output().await.map_err(|e| spawn_error(&line, e))?;
                if !out.status.success() {
                    return Err(failure(line, out.status, &out.stderr));
                }
            }
        }
        Ok(())
    }

    /// Spawn `args` with stdout piped, as a stream of its lines.
    fn lines(&self, args: &[&str]) -> Result<EngineStream<String>> {
        let line = args.join(" ");
        let mut child = self
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| spawn_error(&line, e))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let lines = BufReader::new(stdout).lines();
        // The child rides along in the stream state, so dropping the stream kills it.
        let stream = futures::stream::unfold(
            (lines, child, line),
            |(mut lines, child, line): (_, Child, String)| async move {
                match lines.next_line().await {
                    Ok(Some(l)) => Some((Ok(l), (lines, child, line))),
                    Ok(None) => None,
                    Err(source) => Some((
                        Err(Error::Io {
                            command: line.clone(),
                            source,
                        }),
                        (lines, child, line),
                    )),
                }
            },
        );
        Ok(stream.boxed())
    }
}

fn spawn_error(command: &str, source: std::io::Error) -> Error {
    if source.kind() == std::io::ErrorKind::NotFound {
        Error::Unavailable {
            reason: format!("the `docker` command was not found ({source})"),
        }
    } else {
        Error::Io {
            command: command.to_owned(),
            source,
        }
    }
}

fn failure(command: String, status: std::process::ExitStatus, stderr: &[u8]) -> Error {
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    if UNREACHABLE.iter().any(|needle| stderr.contains(needle)) {
        return Error::Unavailable { reason: stderr };
    }
    Error::Failed {
        command,
        status,
        stderr,
    }
}

#[async_trait]
impl ComposeEngine for DockerCli {
    async fn up(&self, dir: &Path, opts: &UpOptions) -> Result<()> {
        let timeout = opts.wait_timeout.map(|t| t.as_secs().max(1).to_string());
        let mut args = vec!["compose", "up", "-d"];
        if opts.wait {
            args.push("--wait");
            if let Some(timeout) = &timeout {
                args.extend(["--wait-timeout", timeout]);
            }
        }
        self.lifecycle(dir, &args, &opts.env, opts.progress).await
    }

    async fn down(&self, dir: &Path, opts: &DownOptions) -> Result<()> {
        let mut args = vec!["compose", "down"];
        if opts.volumes {
            args.push("--volumes");
        }
        self.lifecycle(dir, &args, &opts.env, opts.progress).await
    }

    async fn ls(&self) -> Result<Vec<ProjectStatus>> {
        let args = ["compose", "ls", "--all", "--format", "json"];
        let out = self.output(&args).await?;
        parse_ls(&out).map_err(|source| Error::Parse {
            command: args.join(" "),
            source,
        })
    }

    async fn ps(&self, project: &str) -> Result<Vec<ServiceStatus>> {
        let args = ["compose", "-p", project, "ps", "--all", "--format", "json"];
        let out = self.output(&args).await?;
        parse_ps(&out).map_err(|source| Error::Parse {
            command: args.join(" "),
            source,
        })
    }

    async fn logs(&self, project: &str, opts: &LogOptions) -> Result<EngineStream<LogLine>> {
        let tail = opts.tail.map(|t| t.to_string());
        let mut args = vec![
            "compose",
            "-p",
            project,
            "logs",
            "--timestamps",
            "--no-color",
        ];
        if opts.follow {
            args.push("--follow");
        }
        if let Some(tail) = &tail {
            args.extend(["--tail", tail]);
        }
        args.extend(opts.services.iter().map(String::as_str));
        Ok(self
            .lines(&args)?
            .map(|line| line.map(|l| parse_log_line(&l)))
            .boxed())
    }

    async fn events(&self, filter: &EventFilter) -> Result<EngineStream<EngineEvent>> {
        let label = match filter {
            EventFilter::Project(name) => format!("label={PROJECT_LABEL}={name}"),
            // Docker filters on a label's exact value only; a prefix is matched below.
            EventFilter::Prefix(_) | EventFilter::All => format!("label={PROJECT_LABEL}"),
        };
        let args = [
            "events",
            "--format",
            "{{json .}}",
            "--filter",
            "type=container",
            "--filter",
            &label,
        ];
        let command = args.join(" ");
        let filter = filter.clone();
        Ok(self
            .lines(&args)?
            .filter_map(move |line| {
                let event = line.and_then(|l| {
                    parse_event(&l).map_err(|source| Error::Parse {
                        command: command.clone(),
                        source,
                    })
                });
                let keep = match event {
                    Ok(Some(e)) if filter.matches(&e) => Some(Ok(e)),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                };
                futures::future::ready(keep)
            })
            .boxed())
    }

    async fn network_rm(&self, name: &str) -> Result<bool> {
        match self.output(&["network", "rm", name]).await {
            Ok(_) => Ok(true),
            Err(Error::Failed { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }
}
