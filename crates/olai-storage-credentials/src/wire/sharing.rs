//! Open Sharing: `SharingTemporaryCredentials`, the response of the
//! `temporary-*-credentials` endpoints for volumes and other storage-backed assets.
//!
//! The payloads match UC REST's, with two differences: the R2 variant is
//! `r2_credentials`, and there is no Azure AD variant. The shape is generated from
//! proto, so field names are snake_case with camelCase accepted on input, and
//! `expiration_time` is written in the proto JSON string form for `int64`.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::common::{self as c, Payload};
pub use super::common::{
    AwsTempCredentials, AzureUserDelegationSas, GcpOauthToken, R2TempCredentials,
};
use super::{VendContext, int64, resolve_expiry};
use crate::error::{Error, Result};
use crate::location::StorageProvider;
use crate::model::{StorageCredentials, VendedCredential, expiry_from_epoch_millis};

/// An Open Sharing temporary credential.
///
/// `Debug` redacts every secret.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharingTemporaryCredentials {
    /// AWS session credentials.
    #[serde(alias = "awsTempCredentials", skip_serializing_if = "Option::is_none")]
    pub aws_temp_credentials: Option<AwsTempCredentials>,
    /// An Azure SAS.
    #[serde(
        alias = "azureUserDelegationSas",
        skip_serializing_if = "Option::is_none"
    )]
    pub azure_user_delegation_sas: Option<AzureUserDelegationSas>,
    /// A GCP OAuth token.
    #[serde(alias = "gcpOauthToken", skip_serializing_if = "Option::is_none")]
    pub gcp_oauth_token: Option<GcpOauthToken>,
    /// Cloudflare R2 session credentials.
    #[serde(alias = "r2Credentials", skip_serializing_if = "Option::is_none")]
    pub r2_credentials: Option<R2TempCredentials>,
    /// Expiry in epoch milliseconds.
    #[serde(
        alias = "expirationTime",
        deserialize_with = "int64::deserialize",
        serialize_with = "int64::as_string::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub expiration_time: Option<i64>,
    /// The storage location the credential covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl SharingTemporaryCredentials {
    /// Converts into the model, for the request `ctx`.
    ///
    /// The same rules as UC REST: `url` is clamped to the request (`ctx.scope` when
    /// absent), and exactly one credential must be set for cloud storage.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<VendedCredential> {
        let scope = c::scope_for_url(self.url.as_deref(), ctx)?;
        let payload = c::single([
            self.aws_temp_credentials.as_ref().map(Payload::Aws),
            self.r2_credentials.as_ref().map(Payload::R2),
            self.azure_user_delegation_sas
                .as_ref()
                .map(Payload::AzureSas),
            self.gcp_oauth_token.as_ref().map(Payload::Gcp),
        ])?;
        let (credentials, hints) = c::to_credentials(payload, scope.location())?;
        let expiry = resolve_expiry(
            self.expiration_time.and_then(expiry_from_epoch_millis),
            &credentials,
        );
        Ok(VendedCredential::new(credentials, scope, ctx.access)
            .with_expiry(expiry)
            .with_hints(hints))
    }
}

impl TryFrom<&VendedCredential> for SharingTemporaryCredentials {
    type Error = Error;

    /// Fails for an Azure AD bearer token, which Open Sharing cannot carry.
    fn try_from(vended: &VendedCredential) -> Result<Self> {
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
                    wire.r2_credentials = Some(R2TempCredentials {
                        access_key_id: aws.access_key_id,
                        secret_access_key: aws.secret_access_key,
                        session_token: aws.session_token,
                    });
                } else {
                    // Open Sharing has no access-point field.
                    wire.aws_temp_credentials = Some(AwsTempCredentials {
                        access_point: String::new(),
                        ..aws
                    });
                }
            }
            StorageCredentials::AzureSas(pairs) => {
                wire.azure_user_delegation_sas = Some(c::sas_payload(pairs));
            }
            StorageCredentials::GcpBearer(token) => {
                wire.gcp_oauth_token = Some(GcpOauthToken {
                    oauth_token: token.clone(),
                });
            }
            StorageCredentials::AzureBearer(_) => {
                return Err(Error::invalid_credential(
                    "Open Sharing cannot carry an Azure AD token",
                ));
            }
            StorageCredentials::None => {}
        }
        Ok(wire)
    }
}

impl fmt::Debug for SharingTemporaryCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharingTemporaryCredentials")
            .field("aws_temp_credentials", &self.aws_temp_credentials)
            .field("azure_user_delegation_sas", &self.azure_user_delegation_sas)
            .field("gcp_oauth_token", &self.gcp_oauth_token)
            .field("r2_credentials", &self.r2_credentials)
            .field("expiration_time", &self.expiration_time)
            .field("url", &self.url)
            .finish()
    }
}
