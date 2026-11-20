//! A team's shared trestle state on its Neon project.
//!
//! The workspaces and environments a team shares live in the database [`STATE_DATABASE`] on
//! a branch of the team's project (its default branch unless told otherwise), through
//! `olai-stack-state`'s Postgres backend. Each machine attaches it in its local store
//! ([`attach`]) and opens it ([`open_attached`]) with the user's own Neon CLI login, so
//! project membership is the access control and no credential is ever stored.

use std::convert::Infallible;
use std::path::Path;

use olai_stack_state::{Attachment, AttachmentId, RemoteLocation, Store};

use super::{NeonControl, NeonError};
use crate::secrets::SecretValue;

/// The database a team's shared state lives in.
pub const STATE_DATABASE: &str = "trestle_state";

/// What can go wrong reaching a team's shared state.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Neon couldn't be asked.
    #[error(transparent)]
    Neon(#[from] NeonError),
    /// The store refused.
    #[error(transparent)]
    Store(#[from] olai_stack_state::Error),
}

/// The location of the shared state on `project`'s `branch` (its default branch when
/// `None`).
pub fn state_location(project: &str, branch: Option<&str>) -> RemoteLocation {
    RemoteLocation::Neon {
        org: None,
        project: project.to_owned(),
        branch: branch.map(str::to_owned),
        database: STATE_DATABASE.to_owned(),
    }
}

/// `branch`, or the project's default branch when `None`.
async fn branch_or_default(
    neon: &dyn NeonControl,
    project: &str,
    branch: Option<&str>,
) -> Result<String, NeonError> {
    if let Some(branch) = branch {
        return Ok(branch.to_owned());
    }
    neon.branches(project)
        .await?
        .into_iter()
        .find(|b| b.default)
        .map(|b| b.id)
        .ok_or_else(|| NeonError::NoSuchBranch("(the default branch)".into()))
}

/// The connection string of the database at `location`, as the signed-in user. Direct, not
/// pooled: the store holds row locks and advisory locks across statements.
pub async fn state_url(
    neon: &dyn NeonControl,
    location: &RemoteLocation,
) -> Result<SecretValue, StateError> {
    let RemoteLocation::Neon {
        project,
        branch,
        database,
        ..
    } = location;
    let branch = branch_or_default(neon, project, branch.as_deref()).await?;
    Ok(neon
        .connection(project, &branch, Some(database), false)
        .await?
        .url)
}

/// Create the state database on `project`'s `branch` if it has none, and return its location
/// and connection string. The schema isn't touched: [`init_state`] migrates it.
pub async fn prepare_state_database(
    neon: &dyn NeonControl,
    project: &str,
    branch: Option<&str>,
) -> Result<(RemoteLocation, SecretValue), StateError> {
    let resolved = branch_or_default(neon, project, branch).await?;
    if !neon
        .databases(project, &resolved)
        .await?
        .iter()
        .any(|d| d == STATE_DATABASE)
    {
        neon.create_database(project, &resolved, STATE_DATABASE)
            .await?;
    }
    let location = state_location(project, branch);
    let url = state_url(neon, &location).await?;
    Ok((location, url))
}

/// Set up the team's shared state on `project`'s `branch`: create the database if needed and
/// create or migrate its schema. Safe to run again (after an upgrade, say). Returns where it
/// is, to [`attach`].
pub async fn init_state(
    neon: &dyn NeonControl,
    project: &str,
    branch: Option<&str>,
) -> Result<RemoteLocation, StateError> {
    let (location, url) = prepare_state_database(neon, project, branch).await?;
    Store::init_remote(url.expose())?;
    Ok(location)
}

/// Record the shared state at `location` in `local` as `name`, once it has checked the state
/// is there and readable. Nothing is recorded when it isn't.
pub async fn attach(
    neon: &dyn NeonControl,
    local: &Store,
    name: &str,
    location: RemoteLocation,
) -> Result<Attachment, StateError> {
    let url = state_url(neon, &location).await?;
    let attachment = local.attach(name, location)?;
    let opened = open_with(local.path(), &attachment.id, &url);
    if let Err(e) = opened {
        let _ = local.detach(&attachment.id);
        return Err(e);
    }
    Ok(attachment)
}

/// Open the attached store `attachment` of the local store at `path`, connecting as the
/// signed-in user.
pub async fn open_attached(
    neon: &dyn NeonControl,
    path: &Path,
    attachment: &AttachmentId,
) -> Result<Store, StateError> {
    let location = Store::open(path)?.attachment(attachment)?.location;
    let url = state_url(neon, &location).await?;
    open_with(path, attachment, &url)
}

fn open_with(
    path: &Path,
    attachment: &AttachmentId,
    url: &SecretValue,
) -> Result<Store, StateError> {
    let url = url.expose().to_owned();
    Ok(Store::open_attached(path, attachment, move |_| {
        Ok::<_, Infallible>(url)
    })?)
}
