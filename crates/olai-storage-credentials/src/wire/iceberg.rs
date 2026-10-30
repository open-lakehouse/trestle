//! Iceberg REST: `storage-credentials` on `loadTable` and from the
//! `/credentials` endpoint, and the FileIO config keys they carry.
//!
//! ```json
//! {
//!   "metadata-location": "s3://bucket/warehouse/tab/metadata/00001.metadata.json",
//!   "metadata": { "…": "…" },
//!   "config": { "client.region": "us-east-1" },
//!   "storage-credentials": [{
//!     "prefix": "s3://bucket/warehouse/tab/",
//!     "config": {
//!       "s3.access-key-id": "…",
//!       "s3.secret-access-key": "…",
//!       "s3.session-token": "…",
//!       "s3.session-token-expires-at-ms": "1700000000000"
//!     }
//!   }]
//! }
//! ```
//!
//! Servers that predate `storage-credentials` put the same keys in the table `config`;
//! [`LoadTableCredentials`] falls back to it, scoped to the requested location.
//!
//! One config map may hold credentials for several stores, and Azure SAS tokens are
//! keyed per account (`adls.sas-token.<account>.dfs.core.windows.net`, as Iceberg Java
//! and Lakekeeper write them, or `adls.sas-token.<account>`). So the credential is
//! picked by the store of the prefix it is for. Expiries are separate keys
//! (`*-expires-at-ms`), in epoch milliseconds. The protocol does not state the access
//! level, so it is taken from the request.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{RedactedConfig, VendContext, parse_expiry_millis, prefix_string, resolve_expiry};
use crate::error::{Error, Result};
use crate::location::{StorageLocation, StorageProvider, StorageScope};
use crate::model::{StorageCredentials, StoreHints, VendedCredential};
use crate::sas;

/// S3 access key id.
pub const S3_ACCESS_KEY_ID: &str = "s3.access-key-id";
/// S3 secret access key.
pub const S3_SECRET_ACCESS_KEY: &str = "s3.secret-access-key";
/// S3 session token.
pub const S3_SESSION_TOKEN: &str = "s3.session-token";
/// S3 session token expiry, epoch milliseconds.
pub const S3_SESSION_TOKEN_EXPIRES_AT_MS: &str = "s3.session-token-expires-at-ms";
/// S3 endpoint.
pub const S3_ENDPOINT: &str = "s3.endpoint";
/// S3 region (PyIceberg, iceberg-rust).
pub const S3_REGION: &str = "s3.region";
/// AWS client region (Iceberg Java).
pub const CLIENT_REGION: &str = "client.region";
/// S3 path-style addressing, `true` or `false`.
pub const S3_PATH_STYLE_ACCESS: &str = "s3.path-style-access";
/// Prefix of per-bucket S3 access points: `s3.access-points.<bucket>`.
pub const S3_ACCESS_POINTS_PREFIX: &str = "s3.access-points.";
/// Azure SAS, unqualified or (with a `.` and the account host appended) per account.
pub const ADLS_SAS_TOKEN: &str = "adls.sas-token";
/// Azure SAS expiry, epoch milliseconds, qualified like [`ADLS_SAS_TOKEN`].
pub const ADLS_SAS_TOKEN_EXPIRES_AT_MS: &str = "adls.sas-token-expires-at-ms";
/// Whether the server signs S3 requests for the client (`true`) instead of vending
/// credentials.
pub const S3_REMOTE_SIGNING_ENABLED: &str = "s3.remote-signing-enabled";
/// GCS OAuth2 token.
pub const GCS_OAUTH2_TOKEN: &str = "gcs.oauth2.token";
/// GCS OAuth2 token expiry, epoch milliseconds.
pub const GCS_OAUTH2_TOKEN_EXPIRES_AT: &str = "gcs.oauth2.token-expires-at";

/// One entry of `storage-credentials`.
///
/// `Debug` shows config keys but no values.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageCredential {
    /// The storage prefix the credential is for.
    pub prefix: String,
    /// The credential, as FileIO config keys.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
}

impl StorageCredential {
    /// Converts into the model, for the request `ctx`.
    ///
    /// Returns `Ok(None)` when the prefix is unrelated to `ctx.scope`; a broader prefix
    /// is clamped to it. Fails if the prefix does not parse or the config lacks a
    /// credential for the prefix's store.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<Option<VendedCredential>> {
        let offered: StorageScope = self.prefix.parse()?;
        let Some(scope) = ctx.clamp(&offered) else {
            return Ok(None);
        };
        // Look the credential up by the offered prefix's store: a clamped scope is in
        // the same bucket and account.
        match from_config(&self.config, offered.location())? {
            Some(found) => Ok(Some(found.into_vended(scope, ctx))),
            None => Err(Error::invalid_credential(format!(
                "storage credential for {offered} has no credential keys for its store"
            ))),
        }
    }
}

impl TryFrom<&VendedCredential> for StorageCredential {
    type Error = Error;

    /// Fails for Azure AD bearer tokens, which have no FileIO key.
    fn try_from(vended: &VendedCredential) -> Result<Self> {
        let millis = |t: DateTime<Utc>| t.timestamp_millis().to_string();
        let mut config = BTreeMap::new();
        let mut put = |k: &str, v: String| {
            config.insert(k.to_owned(), v);
        };
        match &vended.credentials {
            StorageCredentials::Aws {
                key_id,
                secret_key,
                token,
            } => {
                put(S3_ACCESS_KEY_ID, key_id.clone());
                put(S3_SECRET_ACCESS_KEY, secret_key.clone());
                if let Some(token) = token {
                    put(S3_SESSION_TOKEN, token.clone());
                }
                if let Some(t) = vended.expires_at {
                    put(S3_SESSION_TOKEN_EXPIRES_AT_MS, millis(t));
                }
                let hints = &vended.hints;
                if let Some(endpoint) = &hints.endpoint {
                    put(S3_ENDPOINT, endpoint.clone());
                }
                if let Some(region) = &hints.region {
                    put(CLIENT_REGION, region.clone());
                    put(S3_REGION, region.clone());
                }
                if let Some(path_style) = hints.path_style {
                    put(S3_PATH_STYLE_ACCESS, path_style.to_string());
                }
                if let Some(access_point) = &hints.access_point {
                    let bucket = vended.scope.location().bucket();
                    put(
                        &format!("{S3_ACCESS_POINTS_PREFIX}{bucket}"),
                        access_point.clone(),
                    );
                }
            }
            StorageCredentials::AzureSas(pairs) => {
                let suffix = vended
                    .scope
                    .location()
                    .account()
                    .map(|account| format!(".{account}.dfs.core.windows.net"))
                    .unwrap_or_default();
                put(&format!("{ADLS_SAS_TOKEN}{suffix}"), sas::encode_sas(pairs));
                if let Some(t) = vended.expires_at {
                    put(
                        &format!("{ADLS_SAS_TOKEN_EXPIRES_AT_MS}{suffix}"),
                        millis(t),
                    );
                }
            }
            StorageCredentials::GcpBearer(token) => {
                put(GCS_OAUTH2_TOKEN, token.clone());
                if let Some(t) = vended.expires_at {
                    put(GCS_OAUTH2_TOKEN_EXPIRES_AT, millis(t));
                }
            }
            StorageCredentials::AzureBearer(_) => {
                return Err(Error::invalid_credential(
                    "Iceberg FileIO has no key for an Azure AD token",
                ));
            }
            StorageCredentials::None => {}
        }
        Ok(Self {
            prefix: prefix_string(&vended.scope),
            config,
        })
    }
}

impl fmt::Debug for StorageCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageCredential")
            .field("prefix", &self.prefix)
            .field("config", &RedactedConfig(&self.config))
            .finish()
    }
}

/// The response of the `/credentials` endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LoadCredentialsResponse {
    /// The credentials.
    pub storage_credentials: Vec<StorageCredential>,
}

impl LoadCredentialsResponse {
    /// Converts every credential related to `ctx.scope`.
    ///
    /// Fails if the response has credentials but none relates to the request.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<Vec<VendedCredential>> {
        convert_all(&self.storage_credentials, ctx)
    }
}

/// The credential-bearing parts of a `loadTable` response (`LoadTableResult`).
///
/// Deserializes from a whole `LoadTableResult`; `metadata` and the other fields are
/// ignored.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LoadTableCredentials {
    /// Table config; servers without `storage-credentials` put credentials here.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// The vended credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub storage_credentials: Vec<StorageCredential>,
}

impl LoadTableCredentials {
    /// Converts the vended credentials related to `ctx.scope`.
    ///
    /// Uses `storage-credentials` when present. Otherwise reads the table `config` as a
    /// credential for `ctx.scope` itself, and returns an empty list if it holds none
    /// (for example, a server using remote signing instead; see
    /// [`remote_signing`](Self::remote_signing)). Callers must refuse an empty list:
    /// building a store from nothing would send unsigned requests.
    pub fn to_vended(&self, ctx: &VendContext) -> Result<Vec<VendedCredential>> {
        if !self.storage_credentials.is_empty() {
            return convert_all(&self.storage_credentials, ctx);
        }
        Ok(from_config(&self.config, ctx.scope.location())?
            .map(|found| found.into_vended(ctx.scope.clone(), ctx))
            .into_iter()
            .collect())
    }
}

impl LoadTableCredentials {
    /// Whether the server asked for remote signing (`s3.remote-signing-enabled`)
    /// instead of vending S3 credentials. Clients that cannot remote-sign must refuse
    /// the table rather than fall back to unsigned requests.
    pub fn remote_signing(&self) -> bool {
        self.config
            .get(S3_REMOTE_SIGNING_ENABLED)
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }
}

impl fmt::Debug for LoadTableCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadTableCredentials")
            .field("config", &RedactedConfig(&self.config))
            .field("storage_credentials", &self.storage_credentials)
            .finish()
    }
}

fn convert_all(
    credentials: &[StorageCredential],
    ctx: &VendContext,
) -> Result<Vec<VendedCredential>> {
    let mut vended = Vec::new();
    for credential in credentials {
        vended.extend(credential.to_vended(ctx)?);
    }
    if vended.is_empty() && !credentials.is_empty() {
        return Err(Error::invalid_credential(format!(
            "no credential covers the requested {}",
            ctx.scope
        )));
    }
    Ok(vended)
}

/// A credential found in a config map, before it is bound to a scope.
struct Found {
    credentials: StorageCredentials,
    expires_at: Option<DateTime<Utc>>,
    hints: StoreHints,
}

impl Found {
    fn into_vended(self, scope: StorageScope, ctx: &VendContext) -> VendedCredential {
        let expiry = resolve_expiry(self.expires_at, &self.credentials);
        VendedCredential::new(self.credentials, scope, ctx.access)
            .with_expiry(expiry)
            .with_hints(self.hints)
    }
}

/// The credential in `config` for `location`'s store, if any.
fn from_config(
    config: &BTreeMap<String, String>,
    location: &StorageLocation,
) -> Result<Option<Found>> {
    let get = |key: &str| {
        config
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    };
    let expiry = |key: &str| {
        get(key)
            .map(|v| parse_expiry_millis(key, v))
            .transpose()
            .map(Option::flatten)
    };

    let found = match location.provider() {
        StorageProvider::Aws | StorageProvider::R2 => {
            let (Some(key_id), Some(secret)) = (get(S3_ACCESS_KEY_ID), get(S3_SECRET_ACCESS_KEY))
            else {
                return Ok(None);
            };
            let path_style = match get(S3_PATH_STYLE_ACCESS) {
                None => None,
                Some(v) => Some(v.parse::<bool>().map_err(|_| {
                    Error::invalid_credential(format!("`{S3_PATH_STYLE_ACCESS}` is not a boolean"))
                })?),
            };
            let access_point = get(&format!("{S3_ACCESS_POINTS_PREFIX}{}", location.bucket()));
            Found {
                credentials: StorageCredentials::aws(
                    key_id,
                    secret,
                    get(S3_SESSION_TOKEN).map(str::to_owned),
                ),
                expires_at: expiry(S3_SESSION_TOKEN_EXPIRES_AT_MS)?,
                hints: StoreHints {
                    endpoint: get(S3_ENDPOINT).map(str::to_owned),
                    region: get(CLIENT_REGION)
                        .or_else(|| get(S3_REGION))
                        .map(str::to_owned),
                    access_point: access_point.map(str::to_owned),
                    path_style,
                    allow_http: None,
                },
            }
        }
        StorageProvider::Azure => {
            let Some((key, token)) = adls_sas_token(config, location.account()) else {
                return Ok(None);
            };
            let qualifier = &key[ADLS_SAS_TOKEN.len()..];
            Found {
                credentials: StorageCredentials::azure_sas(token)?,
                expires_at: expiry(&format!("{ADLS_SAS_TOKEN_EXPIRES_AT_MS}{qualifier}"))?,
                hints: StoreHints::default(),
            }
        }
        StorageProvider::Gcp => {
            let Some(token) = get(GCS_OAUTH2_TOKEN) else {
                return Ok(None);
            };
            Found {
                credentials: StorageCredentials::GcpBearer(token.to_owned()),
                expires_at: expiry(GCS_OAUTH2_TOKEN_EXPIRES_AT)?,
                hints: StoreHints::default(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(found))
}

/// The SAS for `account`: `adls.sas-token.<account>[.<suffix>]`, else the unqualified
/// `adls.sas-token`. Returns the key it was found under.
fn adls_sas_token<'a>(
    config: &'a BTreeMap<String, String>,
    account: Option<&str>,
) -> Option<(&'a str, &'a str)> {
    let qualified = account.and_then(|account| {
        config.iter().find(|(k, v)| {
            !v.is_empty()
                && k.strip_prefix(ADLS_SAS_TOKEN)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .and_then(|host| host.split('.').next())
                    == Some(account)
        })
    });
    qualified
        .or_else(|| {
            config
                .get_key_value(ADLS_SAS_TOKEN)
                .filter(|(_, v)| !v.is_empty())
        })
        .map(|(k, v)| (k.as_str(), v.as_str()))
}
