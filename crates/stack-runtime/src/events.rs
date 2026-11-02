//! Container events, parsed from `docker events --format '{{json .}}'`.

use serde::{Deserialize, Serialize};

use crate::status::Health;

/// The label Compose puts on every container with its project's name.
pub const PROJECT_LABEL: &str = "com.docker.compose.project";
/// The label Compose puts on every container with its service's name.
pub const SERVICE_LABEL: &str = "com.docker.compose.service";

/// What happened to a container.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// The container was created.
    Create,
    /// It started.
    Start,
    /// It was asked to stop.
    Stop,
    /// It was sent a signal.
    Kill,
    /// Its process exited.
    Die {
        /// The exit code, when Docker reported one.
        exit_code: Option<i32>,
    },
    /// It was removed.
    Destroy,
    /// Its healthcheck changed state.
    Health(Health),
    /// Something else (`pause`, `restart`, `oom`, …), by Docker's name.
    Other(String),
}

/// One event about a container of a Compose project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineEvent {
    /// The project the container belongs to.
    pub project: String,
    /// The compose service.
    pub service: String,
    /// The container's name.
    pub container: String,
    /// What happened.
    pub action: EventAction,
    /// When, in nanoseconds since the Unix epoch.
    pub time_nanos: i64,
}

/// Parse one line of `docker events --format '{{json .}}'`. `Ok(None)` for an event that isn't
/// about a Compose container, and for the `exec_*` events every healthcheck probe raises.
pub fn parse_event(line: &str) -> Result<Option<EngineEvent>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Actor {
        #[serde(rename = "Attributes", default)]
        attributes: std::collections::BTreeMap<String, String>,
    }
    #[derive(Deserialize)]
    struct Raw {
        #[serde(rename = "Type", default)]
        kind: String,
        #[serde(rename = "Action", default)]
        action: String,
        #[serde(rename = "Actor")]
        actor: Actor,
        #[serde(rename = "timeNano", default)]
        time_nano: i64,
    }
    let raw: Raw = serde_json::from_str(line)?;
    if raw.kind != "container" {
        return Ok(None);
    }
    let attrs = &raw.actor.attributes;
    let (Some(project), Some(service)) = (attrs.get(PROJECT_LABEL), attrs.get(SERVICE_LABEL))
    else {
        return Ok(None);
    };
    let action = match raw.action.split_once(':') {
        Some(("health_status", health)) => EventAction::Health(Health::parse(health)),
        Some((name, _)) if name.starts_with("exec_") => return Ok(None),
        _ => match raw.action.as_str() {
            "create" => EventAction::Create,
            "start" => EventAction::Start,
            "stop" => EventAction::Stop,
            "kill" => EventAction::Kill,
            "die" => EventAction::Die {
                exit_code: attrs.get("exitCode").and_then(|c| c.parse().ok()),
            },
            "destroy" => EventAction::Destroy,
            other if other.starts_with("exec_") => return Ok(None),
            other => EventAction::Other(other.to_owned()),
        },
    };
    Ok(Some(EngineEvent {
        project: project.clone(),
        service: service.clone(),
        container: attrs.get("name").cloned().unwrap_or_default(),
        action,
        time_nanos: raw.time_nano,
    }))
}

/// Which projects' events to watch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventFilter {
    /// One project, by name.
    Project(String),
    /// Every project whose name starts with this prefix.
    Prefix(String),
    /// Every Compose project.
    All,
}

impl EventFilter {
    /// Whether `event` passes the filter.
    pub fn matches(&self, event: &EngineEvent) -> bool {
        match self {
            EventFilter::Project(name) => event.project == *name,
            EventFilter::Prefix(prefix) => event.project.starts_with(prefix.as_str()),
            EventFilter::All => true,
        }
    }
}
