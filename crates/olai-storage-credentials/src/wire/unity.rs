//! Unity Catalog REST: the response of `temporary-{table,volume,path}-credentials`.
//!
//! ```json
//! {
//!   "aws_temp_credentials": { "access_key_id": "…", "secret_access_key": "…", "session_token": "…" },
//!   "expiration_time": 1700000000000,
//!   "url": "s3://bucket/warehouse/tab"
//! }
//! ```
//!
//! Exactly one of `aws_temp_credentials`, `azure_user_delegation_sas`, `azure_aad`,
//! `gcp_oauth_token` and `r2_temp_credentials` is set; none for local storage.
//! `expiration_time` is in epoch milliseconds and may be a string (proto JSON), `null`
//! or non-positive, which all mean "does not expire". The response does not say which
//! access level was granted, so it is taken from the request.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::common::{self as c, Payload};
pub use super::common::{
    AwsTempCredentials, AzureAad, AzureUserDelegationSas, GcpOauthToken, R2TempCredentials,
};
use super::{VendContext, int64};
use crate::error::Result;
use crate::location::StorageProvider;
use crate::model::{StorageCredentials, VendedCredential};

/// A UC REST temporary credential.
///
/// `Debug` redacts every secret.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemporaryCredential {
    /// AWS session credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_temp_credentials: Option<AwsTempCredentials>,
    /// An Azure SAS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_user_delegation_sas: Option<AzureUserDelegationSas>,
    /// An Azure AD bearer token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_aad: Option<AzureAad>,
    /// A GCP OAuth token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gcp_oauth_token: Option<GcpOauthToken>,
    /// Cloudflare R2 session credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r2_temp_credentials: Option<R2TempCredentials>,
    /// Expiry in epoch milliseconds.
    #[serde(with = "int64", skip_serializing_if = "Option::is_none")]
    pub expiration_time: Option<i64>,
    /// The storage location the credential covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl TemporaryCredential {
    /// Converts into the model, for the request `ctx`.
    ///
    /// The scope is `url` clamped to `ctx.scope` (see [`VendContext::clamp`]), or
    /// `ctx.scope` when the response has no `url`. Fails if `url` is unrelated to the
    /// request, if more than one credential is set, if none is set for cloud storage,
    /// or if a credential is malformed.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<VendedCredential> {
        c::to_vended(
            self.url.as_deref(),
            [
                self.aws_temp_credentials.as_ref().map(Payload::Aws),
                self.r2_temp_credentials.as_ref().map(Payload::R2),
                self.azure_user_delegation_sas
                    .as_ref()
                    .map(Payload::AzureSas),
                self.azure_aad.as_ref().map(Payload::AzureAad),
                self.gcp_oauth_token.as_ref().map(Payload::Gcp),
            ],
            self.expiration_time,
            ctx,
        )
    }
}

impl From<&VendedCredential> for TemporaryCredential {
    fn from(vended: &VendedCredential) -> Self {
        let mut wire = Self {
            expiration_time: vended.expires_at.map(|t| t.timestamp_millis()),
            url: Some(vended.scope.to_string()),
            ..Self::default()
        };
        match &vended.credentials {
            StorageCredentials::Aws {
                key_id,
                secret_key,
                token,
            } => {
                let aws = c::aws_payload(key_id, secret_key, token, &vended.hints);
                if vended.scope.location().provider() == StorageProvider::R2 {
                    wire.r2_temp_credentials = Some(c::r2_payload(aws));
                } else {
                    wire.aws_temp_credentials = Some(aws);
                }
            }
            StorageCredentials::AzureSas(pairs) => {
                wire.azure_user_delegation_sas = Some(c::sas_payload(pairs));
            }
            StorageCredentials::AzureBearer(token) => {
                wire.azure_aad = Some(AzureAad {
                    aad_token: token.clone(),
                });
            }
            StorageCredentials::GcpBearer(token) => {
                wire.gcp_oauth_token = Some(GcpOauthToken {
                    oauth_token: token.clone(),
                });
            }
            StorageCredentials::None => {}
        }
        wire
    }
}

impl fmt::Debug for TemporaryCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TemporaryCredential")
            .field("aws_temp_credentials", &self.aws_temp_credentials)
            .field("azure_user_delegation_sas", &self.azure_user_delegation_sas)
            .field("azure_aad", &self.azure_aad)
            .field("gcp_oauth_token", &self.gcp_oauth_token)
            .field("r2_temp_credentials", &self.r2_temp_credentials)
            .field("expiration_time", &self.expiration_time)
            .field("url", &self.url)
            .finish()
    }
}
