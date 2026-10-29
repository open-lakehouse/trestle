//! The canonical vended-credential model.
//!
//! Every vending protocol (UC REST, the UC Delta API, Iceberg REST, Open Sharing) maps
//! its wire shape onto [`VendedCredential`]: a short-lived [`StorageCredentials`] secret,
//! the [`StorageScope`] it covers, the [`AccessLevel`] it grants, when it expires, and
//! the [`StoreHints`] a client needs to build a store for it.
//!
//! This module never reads the clock: `SystemTime::now()` panics on
//! `wasm32-unknown-unknown`, so every time-dependent helper takes `now` from the caller.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::location::{StorageLocation, StorageScope};
use crate::sas;

/// What a credential lets its holder do within its scope.
///
/// Protocols have richer *request* vocabularies (UC's `PATH_CREATE_TABLE`, volume
/// operations); protocol modules map those onto this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    /// Read and list objects.
    Read,
    /// Read, list, write and delete objects.
    ReadWrite,
}

impl AccessLevel {
    /// Whether a credential at this level satisfies a request needing `required`.
    pub fn satisfies(self, required: AccessLevel) -> bool {
        self >= required
    }
}

/// The secret part of a vended credential.
///
/// `Debug` never prints secret values: it shows which variant and fields are present,
/// with every secret rendered as `<redacted>`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageCredentials {
    /// AWS access key, with a session token for temporary credentials. Also used for
    /// Cloudflare R2 and other S3-compatible stores.
    Aws {
        /// Access key id.
        key_id: String,
        /// Secret access key.
        secret_key: String,
        /// Session token; `None` for long-lived keys.
        token: Option<String>,
    },
    /// An Azure SAS, as already percent-decoded `(key, value)` query pairs.
    ///
    /// Decoded pairs are what `object_store`'s `AzureCredential::SASToken` expects; it
    /// encodes them exactly once when signing a request.
    AzureSas(Vec<(String, String)>),
    /// An Azure AD (Entra ID) bearer token for storage.
    AzureBearer(String),
    /// A Google OAuth2 access token.
    GcpBearer(String),
    /// No credential is needed (`file://`, `memory://`, anonymous access).
    None,
}

impl StorageCredentials {
    /// AWS credentials. An empty session token is treated as absent: several servers
    /// send `""` for long-lived keys, and signing with an empty token fails.
    pub fn aws(
        key_id: impl Into<String>,
        secret_key: impl Into<String>,
        token: Option<String>,
    ) -> Self {
        Self::Aws {
            key_id: key_id.into(),
            secret_key: secret_key.into(),
            token: token.filter(|t| !t.is_empty()),
        }
    }

    /// An Azure SAS from its query-string form (with or without a leading `?`).
    ///
    /// See [`sas::parse_sas`] for the accepted format.
    pub fn azure_sas(token: &str) -> Result<Self> {
        sas::parse_sas(token).map(Self::AzureSas)
    }
}

impl fmt::Debug for StorageCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const REDACTED: &str = "<redacted>";
        match self {
            Self::Aws { token, .. } => f
                .debug_struct("Aws")
                .field("key_id", &REDACTED)
                .field("secret_key", &REDACTED)
                .field("token", &token.as_ref().map(|_| REDACTED))
                .finish(),
            // Parameter names are not secret and help debugging (which permissions, is
            // there an expiry); the values, `sig` above all, are.
            Self::AzureSas(pairs) => f
                .debug_tuple("AzureSas")
                .field(
                    &pairs
                        .iter()
                        .map(|(k, _)| (k.as_str(), REDACTED))
                        .collect::<Vec<_>>(),
                )
                .finish(),
            Self::AzureBearer(_) => f.debug_tuple("AzureBearer").field(&REDACTED).finish(),
            Self::GcpBearer(_) => f.debug_tuple("GcpBearer").field(&REDACTED).finish(),
            Self::None => f.write_str("None"),
        }
    }
}

/// Facts a client needs to build a store for a credential, beyond the credential itself.
///
/// Each field is `None` when the source said nothing, leaving the decision to the store
/// builder's configuration and defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreHints {
    /// Service endpoint, for R2, Azurite or another S3-compatible store.
    pub endpoint: Option<String>,
    /// S3 region.
    pub region: Option<String>,
    /// S3 access point ARN or alias the credential is bound to.
    pub access_point: Option<String>,
    /// Use path-style rather than virtual-hosted S3 addressing.
    pub path_style: Option<bool>,
    /// Permit plain-`http` endpoints (emulators).
    pub allow_http: Option<bool>,
}

impl StoreHints {
    /// Whether no hint is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A short-lived credential for one storage scope.
///
/// `Debug` is derived: the only secret-bearing field is
/// [`credentials`](Self::credentials), whose `Debug` redacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendedCredential {
    /// The secret.
    pub credentials: StorageCredentials,
    /// The storage the credential covers.
    pub scope: StorageScope,
    /// What the credential allows within its scope.
    pub access: AccessLevel,
    /// When the credential stops working; `None` if it does not expire.
    pub expires_at: Option<DateTime<Utc>>,
    /// Store-building hints.
    pub hints: StoreHints,
}

impl VendedCredential {
    /// A non-expiring credential with no hints.
    pub fn new(credentials: StorageCredentials, scope: StorageScope, access: AccessLevel) -> Self {
        Self {
            credentials,
            scope,
            access,
            expires_at: None,
            hints: StoreHints::default(),
        }
    }

    /// Sets the expiry.
    pub fn with_expiry(mut self, expires_at: Option<DateTime<Utc>>) -> Self {
        self.expires_at = expires_at;
        self
    }

    /// Sets the store hints.
    pub fn with_hints(mut self, hints: StoreHints) -> Self {
        self.hints = hints;
        self
    }

    /// Whether the credential has expired at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_within(now, Duration::ZERO)
    }

    /// Whether the credential expires within `margin` of `now`, i.e. is due for refresh.
    ///
    /// A credential without an expiry never is.
    pub fn expires_within(&self, now: DateTime<Utc>, margin: Duration) -> bool {
        let margin = TimeDelta::from_std(margin).unwrap_or(TimeDelta::MAX);
        self.expires_at
            .is_some_and(|expiry| now.checked_add_signed(margin).is_none_or(|t| t >= expiry))
    }

    /// Whether this credential is usable for `access` on `location`.
    pub fn covers(&self, location: &StorageLocation, access: AccessLevel) -> bool {
        self.access.satisfies(access) && self.scope.covers(location)
    }
}

/// Converts a wire expiry in epoch milliseconds into an expiry time.
///
/// Zero or negative values mean "does not expire": UC sends `0` for credentials on
/// local paths, and some servers send `-1`. So do values past the representable range:
/// some servers send `i64::MAX` for long-lived credentials.
pub fn expiry_from_epoch_millis(millis: i64) -> Option<DateTime<Utc>> {
    if millis <= 0 {
        return None;
    }
    DateTime::from_timestamp_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRETS: &[&str] = &["AKIDSECRET", "s3cr3t", "t0k3n", "sigvalue", "bearer-abc"];

    fn scope() -> StorageScope {
        "s3://bucket/tab".parse().unwrap()
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn debug_never_prints_secrets() {
        let creds = [
            StorageCredentials::aws("AKIDSECRET", "s3cr3t", Some("t0k3n".into())),
            StorageCredentials::azure_sas("sv=2024&sig=sigvalue").unwrap(),
            StorageCredentials::AzureBearer("bearer-abc".into()),
            StorageCredentials::GcpBearer("bearer-abc".into()),
        ];
        for c in creds {
            let rendered = format!("{:?}", VendedCredential::new(c, scope(), AccessLevel::Read));
            for secret in SECRETS {
                assert!(!rendered.contains(secret), "{rendered} leaks {secret}");
            }
            assert!(rendered.contains("<redacted>"), "{rendered}");
        }
    }

    #[test]
    fn debug_shows_presence_and_sas_parameter_names() {
        let with = StorageCredentials::aws("k", "s", Some("t".into()));
        let without = StorageCredentials::aws("k", "s", None);
        assert!(format!("{with:?}").contains(r#"token: Some("<redacted>")"#));
        assert!(format!("{without:?}").contains("token: None"));

        let sas = StorageCredentials::azure_sas("sv=2024&se=2030-01-01&sig=x").unwrap();
        let rendered = format!("{sas:?}");
        assert!(rendered.contains(r#"("se", "<redacted>")"#), "{rendered}");
    }

    #[test]
    fn empty_session_token_is_absent() {
        let StorageCredentials::Aws { token, .. } =
            StorageCredentials::aws("k", "s", Some(String::new()))
        else {
            unreachable!()
        };
        assert_eq!(token, None);
    }

    #[test]
    fn expiry_from_wire_millis() {
        assert_eq!(expiry_from_epoch_millis(0), None);
        assert_eq!(expiry_from_epoch_millis(-1), None);
        assert_eq!(
            expiry_from_epoch_millis(1_700_000_000_000),
            Some(at(1_700_000_000))
        );
    }

    #[test]
    fn refresh_window() {
        let cred = VendedCredential::new(StorageCredentials::None, scope(), AccessLevel::Read)
            .with_expiry(Some(at(1_000)));
        assert!(!cred.is_expired(at(999)));
        assert!(cred.is_expired(at(1_000)));
        assert!(!cred.expires_within(at(900), Duration::from_secs(60)));
        assert!(cred.expires_within(at(940), Duration::from_secs(60)));
        assert!(cred.expires_within(at(0), Duration::MAX));

        let forever = VendedCredential::new(StorageCredentials::None, scope(), AccessLevel::Read);
        assert!(!forever.expires_within(at(i32::MAX.into()), Duration::MAX));
    }

    #[test]
    fn covers_checks_scope_and_access() {
        let cred = VendedCredential::new(StorageCredentials::None, scope(), AccessLevel::Read);
        let inside: StorageLocation = "s3://bucket/tab/x".parse().unwrap();
        let sibling: StorageLocation = "s3://bucket/table2/x".parse().unwrap();
        assert!(cred.covers(&inside, AccessLevel::Read));
        assert!(!cred.covers(&inside, AccessLevel::ReadWrite));
        assert!(!cred.covers(&sibling, AccessLevel::Read));
        assert!(AccessLevel::ReadWrite.satisfies(AccessLevel::Read));
    }
}
