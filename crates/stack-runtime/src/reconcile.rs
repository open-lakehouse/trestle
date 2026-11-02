//! Matching what Docker runs against what the caller manages.

use std::collections::BTreeSet;

use crate::engine::ComposeEngine;
use crate::error::Result;
use crate::status::ProjectStatus;

/// Which projects a caller manages.
///
/// Prefer [`Exact`](Self::Exact) when the caller knows its project names: a
/// [`Prefix`](Self::Prefix) is only unambiguous when the prefix ends in a separator that the
/// rest of a name can't contain. Workspace projects are `<workspace>-<env>` and both parts may
/// contain dashes, so `demo-` also matches the projects of a workspace called `demo-x`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectMatch {
    /// These names.
    Exact(BTreeSet<String>),
    /// Every name starting with this prefix.
    Prefix(String),
}

impl ProjectMatch {
    /// Whether `name` is one of the caller's.
    pub fn matches(&self, name: &str) -> bool {
        match self {
            ProjectMatch::Exact(names) => names.contains(name),
            ProjectMatch::Prefix(prefix) => name.starts_with(prefix.as_str()),
        }
    }
}

/// The projects Docker knows of (running or stopped) that `which` picks.
pub async fn managed_projects(
    engine: &dyn ComposeEngine,
    which: &ProjectMatch,
) -> Result<Vec<ProjectStatus>> {
    Ok(engine
        .ls()
        .await?
        .into_iter()
        .filter(|p| which.matches(&p.name))
        .collect())
}

/// Remove the shared `network` once none of `members` is known to Docker any more; returns
/// whether it was removed.
///
/// Compose labels a network with the project that created it, so no member's `down` removes
/// a network others joined: whoever stops the last member calls this.
pub async fn remove_mesh_if_idle(
    engine: &dyn ComposeEngine,
    network: &str,
    members: &BTreeSet<String>,
) -> Result<bool> {
    let running = managed_projects(engine, &ProjectMatch::Exact(members.clone())).await?;
    if !running.is_empty() {
        return Ok(false);
    }
    engine.network_rm(network).await
}
