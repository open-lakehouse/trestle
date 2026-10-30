//! UC Delta API: `storage-credentials` in table, staging-table and credentials
//! responses.
//!
//! ```json
//! {
//!   "storage-credentials": [{
//!     "prefix": "s3://bucket/warehouse/tab/",
//!     "operation": "READ_WRITE",
//!     "config": { "s3.access-key-id": "…", "s3.secret-access-key": "…", "s3.session-token": "…" },
//!     "expiration-time-ms": 1700000000000
//!   }]
//! }
//! ```
//!
//! The config keys are Iceberg-like with UC names: `s3.*`, `azure.sas-token` and
//! `gcs.oauth-token`. `expiration-time-ms` is `null` for credentials that do not
//! expire, such as those for local paths. Unlike the other protocols, each credential
//! states the access level it grants.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::{RedactedConfig, VendContext, no_covering_credential, prefix_string, resolve_expiry};
use crate::error::{Error, Result};
use crate::location::{StorageProvider, StorageScope};
use crate::model::{AccessLevel, StorageCredentials, VendedCredential, expiry_from_epoch_millis};
use crate::sas;

/// S3 access key id.
pub const S3_ACCESS_KEY_ID: &str = "s3.access-key-id";
/// S3 secret access key.
pub const S3_SECRET_ACCESS_KEY: &str = "s3.secret-access-key";
/// S3 session token.
pub const S3_SESSION_TOKEN: &str = "s3.session-token";
/// Azure SAS token.
pub const AZURE_SAS_TOKEN: &str = "azure.sas-token";
/// GCS OAuth token.
pub const GCS_OAUTH_TOKEN: &str = "gcs.oauth-token";

/// The access level of a Delta API credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Operation {
    /// Read and list.
    Read,
    /// Read, list, write and delete.
    ReadWrite,
}

impl From<Operation> for AccessLevel {
    fn from(op: Operation) -> Self {
        match op {
            Operation::Read => AccessLevel::Read,
            Operation::ReadWrite => AccessLevel::ReadWrite,
        }
    }
}

impl From<AccessLevel> for Operation {
    fn from(access: AccessLevel) -> Self {
        match access {
            AccessLevel::Read => Operation::Read,
            AccessLevel::ReadWrite => Operation::ReadWrite,
        }
    }
}

/// One credential of a `storage-credentials` array.
///
/// `Debug` shows config keys but no values.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StorageCredential {
    /// The storage prefix the credential covers.
    pub prefix: String,
    /// The access level granted.
    pub operation: Operation,
    /// The credential, as config keys.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// Expiry in epoch milliseconds; `None` if it does not expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration_time_ms: Option<i64>,
}

impl StorageCredential {
    /// Converts into the model, for the request `ctx`.
    ///
    /// Returns `Ok(None)` when the prefix is unrelated to `ctx.scope`, including a
    /// prefix this crate cannot parse; a broader prefix is clamped to the request. Fails
    /// if the config lacks the credential for the prefix's store.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<Option<VendedCredential>> {
        // The request always parses, so storage this crate cannot address is unrelated
        // to it. Skipping it lets a server also vend for stores we do not know.
        let Ok(offered) = self.prefix.parse::<StorageScope>() else {
            return Ok(None);
        };
        let Some(scope) = ctx.clamp(&offered) else {
            return Ok(None);
        };
        let get = |key: &str| self.config.get(key).filter(|v| !v.is_empty());
        let missing = |key: &str| Error::invalid_credential(format!("config has no `{key}`"));
        let credentials = match scope.location().provider() {
            StorageProvider::Aws | StorageProvider::R2 => StorageCredentials::aws(
                get(S3_ACCESS_KEY_ID).ok_or_else(|| missing(S3_ACCESS_KEY_ID))?,
                get(S3_SECRET_ACCESS_KEY).ok_or_else(|| missing(S3_SECRET_ACCESS_KEY))?,
                get(S3_SESSION_TOKEN).cloned(),
            ),
            StorageProvider::Azure => StorageCredentials::azure_sas(
                get(AZURE_SAS_TOKEN).ok_or_else(|| missing(AZURE_SAS_TOKEN))?,
            )?,
            StorageProvider::Gcp => StorageCredentials::GcpBearer(
                get(GCS_OAUTH_TOKEN)
                    .ok_or_else(|| missing(GCS_OAUTH_TOKEN))?
                    .clone(),
            ),
            _ => StorageCredentials::None,
        };
        let expiry = resolve_expiry(
            self.expiration_time_ms.and_then(expiry_from_epoch_millis),
            &credentials,
        );
        Ok(Some(
            VendedCredential::new(credentials, scope, self.operation.into()).with_expiry(expiry),
        ))
    }
}

impl TryFrom<&VendedCredential> for StorageCredential {
    type Error = Error;

    /// Fails for Azure AD bearer tokens, which the Delta API cannot carry.
    fn try_from(vended: &VendedCredential) -> Result<Self> {
        let mut config = BTreeMap::new();
        match &vended.credentials {
            StorageCredentials::Aws {
                key_id,
                secret_key,
                token,
            } => {
                config.insert(S3_ACCESS_KEY_ID.to_owned(), key_id.clone());
                config.insert(S3_SECRET_ACCESS_KEY.to_owned(), secret_key.clone());
                if let Some(token) = token {
                    config.insert(S3_SESSION_TOKEN.to_owned(), token.clone());
                }
            }
            StorageCredentials::AzureSas(pairs) => {
                config.insert(AZURE_SAS_TOKEN.to_owned(), sas::encode_sas(pairs));
            }
            StorageCredentials::GcpBearer(token) => {
                config.insert(GCS_OAUTH_TOKEN.to_owned(), token.clone());
            }
            StorageCredentials::AzureBearer(_) => {
                return Err(Error::invalid_credential(
                    "the Delta API cannot carry an Azure AD token",
                ));
            }
            StorageCredentials::None => {}
        }
        Ok(Self {
            prefix: prefix_string(&vended.scope),
            operation: vended.access.into(),
            config,
            expiration_time_ms: vended.expires_at.map(|t| t.timestamp_millis()),
        })
    }
}

impl fmt::Debug for StorageCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageCredential")
            .field("prefix", &self.prefix)
            .field("operation", &self.operation)
            .field("config", &RedactedConfig(&self.config))
            .field("expiration_time_ms", &self.expiration_time_ms)
            .finish()
    }
}

/// A Delta API response carrying `storage-credentials`, such as the credentials
/// endpoint's. Other fields (a table response's `location`, ...) are ignored, so this
/// also reads table and staging-table responses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CredentialsResponse {
    /// The credentials.
    #[serde(default)]
    pub storage_credentials: Vec<StorageCredential>,
}

impl CredentialsResponse {
    /// Converts every credential related to `ctx.scope` (see
    /// [`StorageCredential::to_vended`]).
    ///
    /// Fails if the response has credentials but none relates to the request.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<Vec<VendedCredential>> {
        let mut vended = Vec::new();
        for credential in &self.storage_credentials {
            vended.extend(credential.to_vended(ctx)?);
        }
        if vended.is_empty() && !self.storage_credentials.is_empty() {
            return Err(no_covering_credential(
                self.storage_credentials.iter().map(|c| c.prefix.as_str()),
                ctx,
            ));
        }
        Ok(vended)
    }
}
