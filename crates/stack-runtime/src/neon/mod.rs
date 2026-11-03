//! Neon branches as an environment source (feature `neon`).
//!
//! An instance can be bound to a branch of a Neon project. The branch then supplies what the
//! plan's Neon-hosted placements need (see
//! [`Plan::neon_requirements`](olai_stack_topology::Plan::neon_requirements)): its Postgres,
//! its object storage and the URLs of the functions deployed on it. Everything goes through the
//! Neon CLI behind [`NeonControl`]: [`NeonCli`] runs `neon`, and `FakeNeon` (feature
//! `testing`) stands in for it in tests. A pure compose environment never touches it.
//!
//! - [`bind`] checks the branch against the plan's requirements (a missing function is an
//!   error then, not at start), creates the databases and buckets it lacks, and mints a storage
//!   credential on the branch for the instance. A child branch inherits its parent's
//!   credentials, so reusing one would leak access into every later branch; each binding mints
//!   its own, and [`unbind`] revokes it.
//! - [`NeonBranchSource`] is the [`Source`](crate::Source) an instance takes the branch's
//!   overlay from: `DATABASE_URL`, `DATABASE_URL_UNPOOLED`, `PG*`, the `NEON_S3_*` storage
//!   coordinates and each function's `<SLUG>_FUNCTION_URL`.
//! - Branches trestle creates ([`create_owned_branch`]) carry an ownership annotation and an
//!   expiry. Only owned branches are reset or deleted ([`reset_owned_branch`],
//!   [`delete_owned_branch`]).
//!
//! See `docs/neon.md` for what the CLI prints.

mod cli;
#[cfg(feature = "testing")]
mod fake;
mod source;

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use olai_stack_topology::NeonRequirements;
use serde::{Deserialize, Serialize};

use crate::secrets::SecretValue;

pub use cli::{NeonCli, locate_neon};
#[cfg(feature = "testing")]
pub use fake::{FakeNeon, NeonCall};
pub use source::{NEON_SOURCE, NeonBinding, NeonBranchSource, StorageCredentialRef};

/// The annotation key that marks a branch as created by trestle.
pub const OWNER_ANNOTATION: &str = "trestle.owner";
/// The value of [`OWNER_ANNOTATION`] on a trestle-created branch.
pub const OWNER: &str = "trestle";
/// The annotation key naming the instance a trestle-created branch was made for.
pub const INSTANCE_ANNOTATION: &str = "trestle.instance";

/// What can go wrong talking to Neon.
#[derive(Debug, thiserror::Error)]
pub enum NeonError {
    /// Neither `neon` nor `neonctl` was found.
    #[error("the Neon CLI (`neon` or `neonctl`) is not installed")]
    NotInstalled,
    /// The CLI isn't signed in.
    #[error("the Neon CLI is not signed in: {0}")]
    NotAuthenticated(String),
    /// A command ran and failed.
    #[error("`neon {command}` failed: {stderr}")]
    Failed {
        /// The arguments.
        command: String,
        /// What it printed on stderr.
        stderr: String,
    },
    /// A command's output didn't parse.
    #[error("parsing the output of `neon {command}`: {reason}")]
    Parse {
        /// The arguments.
        command: String,
        /// What was wrong.
        reason: String,
    },
    /// Spawning the CLI failed.
    #[error("running `neon {command}`: {source}")]
    Io {
        /// The arguments.
        command: String,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// No branch of that name or id in the project.
    #[error("no branch `{0}` in the project")]
    NoSuchBranch(String),
    /// The branch wasn't created by trestle, so it is left alone.
    #[error("branch `{0}` was not created by trestle; refusing to reset or delete it")]
    NotOwned(String),
    /// Functions the plan runs on the branch aren't deployed there.
    #[error(
        "branch `{branch}` has no function {}; deploy it on the branch (a branch only copies \
         its parent's functions when it is created)",
        slugs.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ")
    )]
    MissingFunctions {
        /// The branch.
        branch: String,
        /// The missing slugs.
        slugs: Vec<String>,
    },
    /// The branch has no object storage, but the plan uses it.
    #[error("branch `{0}` has no object storage enabled")]
    NoStorage(String),
}

/// A branch of a Neon project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// Its id (`br-…`).
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its parent's id.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// When it expires (RFC 3339), if it does.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Whether it is the project's default branch.
    #[serde(default)]
    pub default: bool,
    /// Its annotations (only the API lists them).
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl Branch {
    /// Whether trestle created it.
    pub fn is_owned(&self) -> bool {
        self.annotations.get(OWNER_ANNOTATION).map(String::as_str) == Some(OWNER)
    }

    /// Whether `name_or_id` names it.
    pub fn is(&self, name_or_id: &str) -> bool {
        self.id == name_or_id || self.name == name_or_id
    }
}

/// How to create a branch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreateBranch {
    /// Its name.
    pub name: String,
    /// The parent branch (name or id); the project's default when `None`.
    pub parent: Option<String>,
    /// When it expires (RFC 3339).
    pub expires_at: Option<String>,
    /// Annotations to set.
    pub annotations: BTreeMap<String, String>,
}

/// A branch's Postgres connection, as `neon connection-string --extended` prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    /// The full URL (with the password).
    pub url: SecretValue,
    /// The host.
    pub host: String,
    /// The role.
    pub role: String,
    /// The role's password.
    pub password: SecretValue,
    /// The database.
    pub database: String,
    /// The query options (`sslmode=require&channel_binding=require`).
    pub options: String,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("url", &"<redacted>")
            .field("host", &self.host)
            .field("role", &self.role)
            .field("password", &"<redacted>")
            .field("database", &self.database)
            .field("options", &self.options)
            .finish()
    }
}

/// A branch's object storage, as `neon api /projects/<p>/branches/<b>/storage` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Storage {
    /// Whether the branch has it.
    pub enabled: bool,
    /// The S3 endpoint.
    #[serde(default)]
    pub s3_endpoint: String,
    /// The region.
    #[serde(default)]
    pub region: String,
    /// Whether clients must address buckets path-style (Neon's always does).
    #[serde(default)]
    pub force_path_style: bool,
}

/// A function deployed on a branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// The slug it was deployed under.
    pub slug: String,
    /// Where it is invoked.
    pub invocation_url: String,
}

/// A storage credential minted on a branch: the S3 access key id is its `token_id`.
#[derive(Clone, PartialEq, Eq)]
pub struct StorageCredential {
    /// Its id, which is also the S3 access key id.
    pub token_id: String,
    /// The S3 secret access key.
    pub secret_access_key: SecretValue,
}

impl fmt::Debug for StorageCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageCredential")
            .field("token_id", &self.token_id)
            .field("secret_access_key", &"<redacted>")
            .finish()
    }
}

/// What trestle asks of Neon, per project. [`NeonCli`] runs the Neon CLI; `FakeNeon` (feature
/// `testing`) is an in-memory stand-in.
#[async_trait]
pub trait NeonControl: Send + Sync {
    /// Who the CLI is signed in as (an error when it isn't).
    async fn auth_status(&self) -> Result<String, NeonError>;

    /// The project's branches, with their annotations.
    async fn branches(&self, project: &str) -> Result<Vec<Branch>, NeonError>;

    /// Create a branch (never printing its credentials).
    async fn create_branch(&self, project: &str, spec: &CreateBranch) -> Result<Branch, NeonError>;

    /// Delete the branch `id`.
    async fn delete_branch(&self, project: &str, id: &str) -> Result<(), NeonError>;

    /// Reset the branch `id` to its parent's current state.
    async fn reset_branch(&self, project: &str, id: &str) -> Result<(), NeonError>;

    /// The branch's Postgres connection to `database` (the main one when `None`: `neondb`, or
    /// the first when it has none), through the connection pooler when `pooled`.
    async fn connection(
        &self,
        project: &str,
        branch: &str,
        database: Option<&str>,
        pooled: bool,
    ) -> Result<Connection, NeonError>;

    /// The branch's databases.
    async fn databases(&self, project: &str, branch: &str) -> Result<Vec<String>, NeonError>;

    /// Create the database `name` on the branch, owned by the main database's owner.
    async fn create_database(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<(), NeonError>;

    /// The functions deployed on the branch.
    async fn functions(&self, project: &str, branch: &str) -> Result<Vec<Function>, NeonError>;

    /// The branch's object storage.
    async fn storage(&self, project: &str, branch: &str) -> Result<Storage, NeonError>;

    /// The branch's buckets.
    async fn buckets(&self, project: &str, branch: &str) -> Result<Vec<String>, NeonError>;

    /// Create the bucket `name` on the branch.
    async fn create_bucket(&self, project: &str, branch: &str, name: &str)
    -> Result<(), NeonError>;

    /// Mint a storage credential (read and write) called `name` on the branch.
    async fn create_credential(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<StorageCredential, NeonError>;

    /// Revoke the credential `token_id` on the branch.
    async fn revoke_credential(
        &self,
        project: &str,
        branch: &str,
        token_id: &str,
    ) -> Result<(), NeonError>;
}

/// The branch `name_or_id` of the project.
pub async fn find_branch(
    neon: &dyn NeonControl,
    project: &str,
    name_or_id: &str,
) -> Result<Branch, NeonError> {
    neon.branches(project)
        .await?
        .into_iter()
        .find(|b| b.is(name_or_id))
        .ok_or_else(|| NeonError::NoSuchBranch(name_or_id.to_owned()))
}

/// Create a branch owned by trestle: annotated with [`OWNER_ANNOTATION`] (and the
/// `instance`, when it is made for one) and expiring at `expires_at`.
pub async fn create_owned_branch(
    neon: &dyn NeonControl,
    project: &str,
    name: &str,
    parent: Option<&str>,
    expires_at: Option<String>,
    instance: Option<&str>,
) -> Result<Branch, NeonError> {
    let mut annotations = BTreeMap::from([(OWNER_ANNOTATION.to_owned(), OWNER.to_owned())]);
    if let Some(instance) = instance {
        annotations.insert(INSTANCE_ANNOTATION.to_owned(), instance.to_owned());
    }
    neon.create_branch(
        project,
        &CreateBranch {
            name: name.to_owned(),
            parent: parent.map(str::to_owned),
            expires_at,
            annotations,
        },
    )
    .await
}

/// Delete the branch `name_or_id`, if trestle created it.
pub async fn delete_owned_branch(
    neon: &dyn NeonControl,
    project: &str,
    name_or_id: &str,
) -> Result<Branch, NeonError> {
    let branch = owned(neon, project, name_or_id).await?;
    neon.delete_branch(project, &branch.id).await?;
    Ok(branch)
}

/// Reset the branch `name_or_id` to its parent, if trestle created it.
pub async fn reset_owned_branch(
    neon: &dyn NeonControl,
    project: &str,
    name_or_id: &str,
) -> Result<Branch, NeonError> {
    let branch = owned(neon, project, name_or_id).await?;
    neon.reset_branch(project, &branch.id).await?;
    Ok(branch)
}

async fn owned(
    neon: &dyn NeonControl,
    project: &str,
    name_or_id: &str,
) -> Result<Branch, NeonError> {
    let branch = find_branch(neon, project, name_or_id).await?;
    if branch.is_owned() {
        Ok(branch)
    } else {
        Err(NeonError::NotOwned(branch.name))
    }
}

/// An RFC 3339 timestamp `ttl` from now, for [`CreateBranch::expires_at`].
pub fn expires_in(ttl: std::time::Duration) -> String {
    let at = chrono::Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Bind an instance to the branch `name_or_id` of `project`, for a plan that needs `req`:
///
/// - every function the plan runs must be deployed on the branch ([`NeonError::MissingFunctions`]
///   lists the ones that aren't);
/// - the databases and buckets it uses are created where missing;
/// - when it uses object storage, a credential called `credential_name` is minted on the
///   branch (returned with the binding: the caller stores the secret, the binding only its id).
///
/// The returned [`NeonBinding`] is the instance's source config.
pub async fn bind(
    neon: &dyn NeonControl,
    project: &str,
    name_or_id: &str,
    req: &NeonRequirements,
    credential_name: &str,
) -> Result<(NeonBinding, Option<StorageCredential>), NeonError> {
    let branch = find_branch(neon, project, name_or_id).await?;
    if !req.functions.is_empty() {
        let deployed = neon.functions(project, &branch.id).await?;
        let missing: Vec<String> = req
            .functions
            .iter()
            .filter(|f| !deployed.iter().any(|d| d.slug == f.slug))
            .map(|f| f.slug.clone())
            .collect();
        if !missing.is_empty() {
            return Err(NeonError::MissingFunctions {
                branch: branch.name,
                slugs: missing,
            });
        }
    }
    if !req.databases.is_empty() {
        let existing = neon.databases(project, &branch.id).await?;
        for db in req.databases.iter().filter(|d| !existing.contains(d)) {
            neon.create_database(project, &branch.id, db).await?;
        }
    }
    let uses_storage = !req.buckets.is_empty()
        || req
            .vars
            .iter()
            .any(|v| v.starts_with(source::S3_VAR_PREFIX));
    let mut credential = None;
    if uses_storage {
        if !neon.storage(project, &branch.id).await?.enabled {
            return Err(NeonError::NoStorage(branch.name));
        }
        let existing = neon.buckets(project, &branch.id).await?;
        for bucket in req.buckets.iter().filter(|b| !existing.contains(b)) {
            neon.create_bucket(project, &branch.id, bucket).await?;
        }
        credential = Some(
            neon.create_credential(project, &branch.id, credential_name)
                .await?,
        );
    }
    let binding = NeonBinding {
        project_id: project.to_owned(),
        branch: branch.id,
        branch_name: branch.name,
        functions: req
            .functions
            .iter()
            .filter_map(|f| Some((f.slug.clone(), f.url_var.clone()?)))
            .collect(),
        storage_credential: None,
    };
    Ok((binding, credential))
}

/// Undo [`bind`]: revoke the binding's storage credential, if it has one.
pub async fn unbind(neon: &dyn NeonControl, binding: &NeonBinding) -> Result<(), NeonError> {
    if let Some(credential) = &binding.storage_credential {
        neon.revoke_credential(&binding.project_id, &binding.branch, &credential.token_id)
            .await?;
    }
    Ok(())
}
