//! Credential payloads shared by UC REST and Open Sharing.
//!
//! Both protocols carry the same per-cloud objects (`aws_temp_credentials`,
//! `azure_user_delegation_sas`, ...) next to an `expiration_time` and a `url`. Every
//! type here redacts its secrets in `Debug`. Fields default when absent, so a server
//! that omits empty strings (proto JSON does) still parses.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::VendContext;
use crate::error::{Error, Result};
use crate::location::{StorageLocation, StorageProvider, StorageScope};
use crate::model::{StorageCredentials, StoreHints};
use crate::sas;

const REDACTED: &str = "<redacted>";

/// AWS STS session credentials.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AwsTempCredentials {
    /// Access key id.
    pub access_key_id: String,
    /// Secret access key.
    pub secret_access_key: String,
    /// Session token; empty or absent for long-lived keys.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_token: String,
    /// The S3 access point the credential is bound to (UC REST only).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub access_point: String,
}

impl fmt::Debug for AwsTempCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsTempCredentials")
            .field("access_key_id", &REDACTED)
            .field("secret_access_key", &REDACTED)
            .field(
                "session_token",
                &(!self.session_token.is_empty()).then_some(REDACTED),
            )
            .field("access_point", &self.access_point)
            .finish()
    }
}

/// Cloudflare R2 session credentials.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct R2TempCredentials {
    /// Access key id.
    pub access_key_id: String,
    /// Secret access key.
    pub secret_access_key: String,
    /// Session token.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_token: String,
}

impl fmt::Debug for R2TempCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("R2TempCredentials")
            .field("access_key_id", &REDACTED)
            .field("secret_access_key", &REDACTED)
            .field(
                "session_token",
                &(!self.session_token.is_empty()).then_some(REDACTED),
            )
            .finish()
    }
}

/// An Azure SAS token, user-delegation or service.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AzureUserDelegationSas {
    /// The SAS query string, percent-encoded.
    pub sas_token: String,
}

impl fmt::Debug for AzureUserDelegationSas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureUserDelegationSas")
            .field("sas_token", &REDACTED)
            .finish()
    }
}

/// An Azure AD (Entra ID) bearer token.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AzureAad {
    /// The bearer token.
    pub aad_token: String,
}

impl fmt::Debug for AzureAad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureAad")
            .field("aad_token", &REDACTED)
            .finish()
    }
}

/// A Google OAuth2 access token.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GcpOauthToken {
    /// The access token.
    pub oauth_token: String,
}

impl fmt::Debug for GcpOauthToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpOauthToken")
            .field("oauth_token", &REDACTED)
            .finish()
    }
}

/// The scope of a credential whose response named `url`: `url` clamped to the request,
/// or the requested scope when `url` is absent or empty.
pub(crate) fn scope_for_url(url: Option<&str>, ctx: &VendContext) -> Result<StorageScope> {
    let Some(url) = url.filter(|u| !u.is_empty()) else {
        return Ok(ctx.scope.clone());
    };
    let offered: StorageScope = url.parse()?;
    ctx.clamp(&offered).ok_or_else(|| {
        Error::invalid_credential(format!(
            "credential for {offered} does not cover the requested {}",
            ctx.scope
        ))
    })
}

/// The one credential payload a response carried, borrowed from it.
pub(crate) enum Payload<'a> {
    Aws(&'a AwsTempCredentials),
    R2(&'a R2TempCredentials),
    AzureSas(&'a AzureUserDelegationSas),
    // Open Sharing has no Azure AD variant.
    #[cfg_attr(not(feature = "unity"), allow(dead_code))]
    AzureAad(&'a AzureAad),
    Gcp(&'a GcpOauthToken),
}

/// Picks the single payload out of a response's optional fields.
///
/// Fails if more than one is set. `Ok(None)` means none is, which is only valid for
/// storage that needs no credential.
pub(crate) fn single<'a>(
    candidates: impl IntoIterator<Item = Option<Payload<'a>>>,
) -> Result<Option<Payload<'a>>> {
    let mut set = candidates.into_iter().flatten();
    let first = set.next();
    if set.next().is_some() {
        return Err(Error::invalid_credential(
            "response carries more than one credential",
        ));
    }
    Ok(first)
}

/// Converts a payload into the model's credentials plus the hints it implies, for a
/// credential covering `location`.
pub(crate) fn to_credentials(
    payload: Option<Payload<'_>>,
    location: &StorageLocation,
) -> Result<(StorageCredentials, StoreHints)> {
    let mut hints = StoreHints::default();
    let creds = match payload {
        None => match location.provider() {
            StorageProvider::Local | StorageProvider::Memory => StorageCredentials::None,
            _ => return Err(Error::invalid_credential("response carries no credential")),
        },
        Some(Payload::Aws(aws)) => {
            if !aws.access_point.is_empty() {
                hints.access_point = Some(aws.access_point.clone());
            }
            aws_credentials(
                &aws.access_key_id,
                &aws.secret_access_key,
                &aws.session_token,
            )?
        }
        Some(Payload::R2(r2)) => {
            if let Some(account) = location.account() {
                hints.endpoint = Some(format!("https://{account}.r2.cloudflarestorage.com"));
                hints.region = Some("auto".to_owned());
            }
            aws_credentials(&r2.access_key_id, &r2.secret_access_key, &r2.session_token)?
        }
        Some(Payload::AzureSas(sas)) => StorageCredentials::azure_sas(&sas.sas_token)?,
        Some(Payload::AzureAad(aad)) => {
            non_empty(&aad.aad_token, "aad_token")?;
            StorageCredentials::AzureBearer(aad.aad_token.clone())
        }
        Some(Payload::Gcp(gcp)) => {
            non_empty(&gcp.oauth_token, "oauth_token")?;
            StorageCredentials::GcpBearer(gcp.oauth_token.clone())
        }
    };
    Ok((creds, hints))
}

fn aws_credentials(key_id: &str, secret: &str, token: &str) -> Result<StorageCredentials> {
    non_empty(key_id, "access_key_id")?;
    non_empty(secret, "secret_access_key")?;
    Ok(StorageCredentials::aws(
        key_id,
        secret,
        Some(token.to_owned()),
    ))
}

fn non_empty(value: &str, field: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::invalid_credential(format!("`{field}` is empty")));
    }
    Ok(())
}

/// The AWS payload for a model credential.
pub(crate) fn aws_payload(
    key_id: &str,
    secret_key: &str,
    token: &Option<String>,
    hints: &StoreHints,
) -> AwsTempCredentials {
    AwsTempCredentials {
        access_key_id: key_id.to_owned(),
        secret_access_key: secret_key.to_owned(),
        session_token: token.clone().unwrap_or_default(),
        access_point: hints.access_point.clone().unwrap_or_default(),
    }
}

/// The SAS payload for decoded SAS pairs.
pub(crate) fn sas_payload(pairs: &[(String, String)]) -> AzureUserDelegationSas {
    AzureUserDelegationSas {
        sas_token: sas::encode_sas(pairs),
    }
}
