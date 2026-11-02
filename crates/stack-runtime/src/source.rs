//! Overlay sources: what an instance's environment takes from outside the store.

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use async_trait::async_trait;
use olai_stack_state::{InstanceId, VarSet};

use crate::secrets::SecretValue;

/// What a [`Source`] contributes to an instance's environment.
#[derive(Clone, Default)]
pub struct Overlay {
    /// Plain variables.
    pub vars: BTreeMap<String, String>,
    /// Where hosted services are reached (e.g. a function's URL), as variables.
    pub endpoints: BTreeMap<String, String>,
    /// Credentials, as variables (e.g. a database password, vended storage keys). Masked
    /// wherever the environment is shown.
    pub credentials: BTreeMap<String, SecretValue>,
    /// When the credentials stop working, if they expire.
    pub expires_at: Option<SystemTime>,
}

impl fmt::Debug for Overlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlay")
            .field("vars", &self.vars)
            .field("endpoints", &self.endpoints)
            .field("credentials", &self.credentials.keys().collect::<Vec<_>>())
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Overlay {
    /// Whether the credentials have expired at `now`.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }
}

/// What a source is asked for.
#[derive(Clone, Debug)]
pub struct SourceCtx {
    /// The instance.
    pub instance: InstanceId,
    /// Its workspace's name.
    pub workspace: String,
    /// Its environment's name.
    pub environment: String,
    /// The revision it runs.
    pub revision: u32,
}

/// Why a source failed.
#[derive(Debug, thiserror::Error)]
#[error("source `{source_name}`: {reason}")]
pub struct SourceError {
    /// The source.
    pub source_name: String,
    /// What went wrong.
    pub reason: String,
}

/// Something outside the store an instance takes variables, endpoints and credentials from,
/// as an overlay on its environment (a Neon branch, say: its Postgres URL, its object-storage
/// endpoint and vended keys, its function URLs). An instance names the sources it takes; they
/// apply in that order, above the store's vars and below the instance's own overrides.
#[async_trait]
pub trait Source: Send + Sync {
    /// The name instances refer to it by.
    fn name(&self) -> &str;

    /// The overlay for the instance `ctx` describes.
    async fn overlay(&self, ctx: &SourceCtx) -> Result<Overlay, SourceError>;

    /// A fresh overlay once `current` has expired. The default asks again.
    async fn refresh(&self, ctx: &SourceCtx, current: &Overlay) -> Result<Overlay, SourceError> {
        let _ = current;
        self.overlay(ctx).await
    }
}

/// The user's own variables, as a source named `user`: literal values only (secrets belong in
/// the store as references).
#[derive(Clone, Debug, Default)]
pub struct UserVars {
    vars: BTreeMap<String, String>,
}

impl UserVars {
    /// A source of `vars`.
    pub fn new(vars: BTreeMap<String, String>) -> Self {
        UserVars { vars }
    }

    /// The literal values of `set` (its secret references are skipped).
    pub fn from_var_set(set: &VarSet) -> Self {
        UserVars::new(
            set.0
                .iter()
                .filter_map(|(k, v)| match v {
                    olai_stack_state::VarValue::Literal(l) => Some((k.clone(), l.clone())),
                    olai_stack_state::VarValue::Secret(_) => None,
                })
                .collect(),
        )
    }
}

#[async_trait]
impl Source for UserVars {
    fn name(&self) -> &str {
        "user"
    }

    async fn overlay(&self, _ctx: &SourceCtx) -> Result<Overlay, SourceError> {
        Ok(Overlay {
            vars: self.vars.clone(),
            ..Overlay::default()
        })
    }
}
