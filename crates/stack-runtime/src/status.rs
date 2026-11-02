//! Typed project and service status, parsed from `docker compose ls/ps --format json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One Compose project Docker knows of (a row of `docker compose ls --all`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectStatus {
    /// The project name.
    pub name: String,
    /// Compose's status string as it prints it, e.g. `exited(1), running(2)`.
    pub status: String,
    /// The containers per state, parsed from [`status`](Self::status), e.g. `running → 2`.
    pub states: BTreeMap<String, u32>,
    /// The compose files the project was started from, comma-separated as Compose lists them.
    pub config_files: String,
}

impl ProjectStatus {
    /// How many of the project's containers are running.
    pub fn running(&self) -> u32 {
        self.states.get("running").copied().unwrap_or(0)
    }

    /// Whether any of the project's containers is running.
    pub fn is_running(&self) -> bool {
        self.running() > 0
    }
}

/// Parse `docker compose ls --all --format json`: a JSON array of projects.
pub fn parse_ls(json: &str) -> Result<Vec<ProjectStatus>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Status", default)]
        status: String,
        #[serde(rename = "ConfigFiles", default)]
        config_files: String,
    }
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<Row> = serde_json::from_str(json)?;
    Ok(rows
        .into_iter()
        .map(|r| ProjectStatus {
            states: parse_states(&r.status),
            name: r.name,
            status: r.status,
            config_files: r.config_files,
        })
        .collect())
}

/// `exited(1), running(2)` → `{exited: 1, running: 2}`. Parts that don't parse are skipped.
fn parse_states(status: &str) -> BTreeMap<String, u32> {
    status
        .split(',')
        .filter_map(|part| {
            let (state, count) = part.trim().split_once('(')?;
            let count = count.strip_suffix(')')?.parse().ok()?;
            Some((state.to_owned(), count))
        })
        .collect()
}

/// A container's lifecycle state, as Docker reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Created, never started.
    Created,
    /// Running.
    Running,
    /// Paused.
    Paused,
    /// Restarting after an exit, under a restart policy.
    Restarting,
    /// Being removed.
    Removing,
    /// Exited.
    Exited,
    /// Dead (could not be stopped or removed cleanly).
    Dead,
    /// A state this crate doesn't know.
    Other(String),
}

impl State {
    fn parse(s: &str) -> Self {
        match s {
            "created" => State::Created,
            "running" => State::Running,
            "paused" => State::Paused,
            "restarting" => State::Restarting,
            "removing" => State::Removing,
            "exited" => State::Exited,
            "dead" => State::Dead,
            other => State::Other(other.to_owned()),
        }
    }
}

/// A container's health, as its healthcheck reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// The service declares no healthcheck.
    None,
    /// The healthcheck hasn't passed yet (within its start period).
    Starting,
    /// The healthcheck passes.
    Healthy,
    /// The healthcheck fails.
    Unhealthy,
}

impl Health {
    /// Parse Docker's health word; an empty string (no healthcheck) and unknown words are
    /// [`Health::None`].
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "starting" => Health::Starting,
            "healthy" => Health::Healthy,
            "unhealthy" => Health::Unhealthy,
            _ => Health::None,
        }
    }
}

/// A host port a container publishes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Published {
    /// The host address it is bound on (e.g. `0.0.0.0`, `127.0.0.1`).
    pub address: String,
    /// The host port.
    pub host_port: u16,
    /// The container port.
    pub container_port: u16,
    /// `tcp` or `udp`.
    pub protocol: String,
}

/// One container of a project (a row of `docker compose ps --all`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceStatus {
    /// The compose service.
    pub service: String,
    /// The container's name.
    pub container: String,
    /// Its lifecycle state.
    pub state: State,
    /// Its health.
    pub health: Health,
    /// Its exit code (meaningful once [`State::Exited`]).
    pub exit_code: i32,
    /// Docker's human status, e.g. `Up 5 seconds (healthy)`.
    pub status: String,
    /// The host ports it publishes.
    pub published: Vec<Published>,
}

impl ServiceStatus {
    /// Whether the container runs and, if it has a healthcheck, passes it.
    pub fn is_ready(&self) -> bool {
        self.state == State::Running && matches!(self.health, Health::None | Health::Healthy)
    }
}

/// Parse `docker compose ps --all --format json`. Compose prints one JSON object per line;
/// some versions print a single JSON array instead, which is accepted too.
pub fn parse_ps(json: &str) -> Result<Vec<ServiceStatus>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Publisher {
        #[serde(rename = "URL", default)]
        url: String,
        #[serde(rename = "TargetPort", default)]
        target_port: u16,
        #[serde(rename = "PublishedPort", default)]
        published_port: u16,
        #[serde(rename = "Protocol", default)]
        protocol: String,
    }
    #[derive(Deserialize)]
    struct Row {
        #[serde(rename = "Service")]
        service: String,
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "State", default)]
        state: String,
        #[serde(rename = "Health", default)]
        health: String,
        #[serde(rename = "ExitCode", default)]
        exit_code: i32,
        #[serde(rename = "Status", default)]
        status: String,
        #[serde(rename = "Publishers", default)]
        publishers: Option<Vec<Publisher>>,
    }
    let json = json.trim();
    let rows: Vec<Row> = if json.starts_with('[') {
        serde_json::from_str(json)?
    } else {
        json.lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?
    };
    Ok(rows
        .into_iter()
        .map(|r| ServiceStatus {
            service: r.service,
            container: r.name,
            state: State::parse(&r.state),
            health: Health::parse(&r.health),
            exit_code: r.exit_code,
            status: r.status,
            published: r
                .publishers
                .unwrap_or_default()
                .into_iter()
                // Compose lists exposed-but-unpublished ports with a zero host port.
                .filter(|p| p.published_port != 0)
                .map(|p| Published {
                    address: p.url,
                    host_port: p.published_port,
                    container_port: p.target_port,
                    protocol: p.protocol,
                })
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_parse_from_the_status_string() {
        let states = parse_states("exited(1), running(2)");
        assert_eq!(states.get("exited"), Some(&1));
        assert_eq!(states.get("running"), Some(&2));
        assert!(parse_states("").is_empty());
        assert!(parse_states("garbage").is_empty());
    }

    #[test]
    fn an_empty_ls_is_no_projects() {
        assert!(parse_ls("").unwrap().is_empty());
        assert!(parse_ls("[]\n").unwrap().is_empty());
    }

    #[test]
    fn ps_accepts_an_array_too() {
        let rows = parse_ps(
            r#"[{"Service":"web","Name":"p-web-1","State":"running","Health":"","ExitCode":0,
                 "Status":"Up","Publishers":null}]"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_ready());
        assert!(rows[0].published.is_empty());
    }
}
