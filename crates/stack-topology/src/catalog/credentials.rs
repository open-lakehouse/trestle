//! The credentials the in-stack services run with: the Postgres password, the RustFS root
//! keys, Authelia's signing and encryption secrets.
//!
//! The baseline catalog defaults each one to a fixed, publicly known development value, so a
//! render is reproducible. A consumer that ships the rendered environment somewhere (e.g. a
//! downloaded bundle) fills [`LocalCredentials`] with fresh random values instead and builds
//! the catalog with [`baseline_catalog_with`](crate::baseline_catalog_with). Every file that
//! needs a credential reads it from that one place, so the services and their consumers
//! always agree.
//!
//! A rendered environment keeps the values it was built with in its `secrets.env`, under
//! each credential's [`env_var`](LocalCredential::env_var) name, so a later render reads them
//! back ([`LocalCredentials::from_secrets`]) instead of reverting to the defaults.
//!
//! Hosted resources' credentials are not covered here: they come from the operator through
//! [`Secrets`].

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use crate::Secrets;

/// One in-stack credential [`LocalCredentials`] can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalCredential {
    /// Its key in [`LocalCredentials`] (e.g. `"postgres_password"`).
    pub key: &'static str,
    /// The id of the module whose services use it.
    pub module: &'static str,
    /// What it is, for a reader.
    pub description: &'static str,
    /// The fixed development value used when none is set.
    pub default: &'static str,
    /// The shortest value the service accepts.
    pub min_len: usize,
    /// The longest value the service accepts.
    pub max_len: usize,
}

impl LocalCredential {
    /// The name it is kept under in `secrets.env`: `LOCAL_` and its key in upper case (e.g.
    /// `LOCAL_POSTGRES_PASSWORD`).
    pub fn env_var(&self) -> String {
        format!("LOCAL_{}", self.key.to_ascii_uppercase())
    }
}

/// The Postgres superuser password.
pub const POSTGRES_PASSWORD: &str = "postgres_password";
/// The RustFS root access key.
pub const RUSTFS_ACCESS_KEY: &str = "rustfs_access_key";
/// The RustFS root secret key.
pub const RUSTFS_SECRET_KEY: &str = "rustfs_secret_key";
/// The secret Authelia signs session cookies with.
pub const AUTHELIA_SESSION_SECRET: &str = "authelia_session_secret";
/// The key Authelia encrypts its storage with.
pub const AUTHELIA_STORAGE_ENCRYPTION_KEY: &str = "authelia_storage_encryption_key";
/// The secret Authelia signs its identity-verification JWTs with.
pub const AUTHELIA_JWT_SECRET: &str = "authelia_jwt_secret";

/// Every credential [`LocalCredentials`] can set, with its development default.
pub const LOCAL_CREDENTIALS: &[LocalCredential] = &[
    LocalCredential {
        key: POSTGRES_PASSWORD,
        module: "postgres",
        description: "Postgres superuser password",
        default: "postgres",
        min_len: 8,
        max_len: 128,
    },
    LocalCredential {
        key: RUSTFS_ACCESS_KEY,
        module: "rustfs",
        description: "RustFS root access key",
        default: "rustfs-root",
        min_len: 3,
        max_len: 20,
    },
    LocalCredential {
        key: RUSTFS_SECRET_KEY,
        module: "rustfs",
        description: "RustFS root secret key",
        default: "rustfs-root-secret",
        min_len: 8,
        max_len: 40,
    },
    LocalCredential {
        key: AUTHELIA_SESSION_SECRET,
        module: "authelia",
        description: "Authelia session secret",
        default: "insecure_session_secret_change_me",
        min_len: 20,
        max_len: 128,
    },
    LocalCredential {
        key: AUTHELIA_STORAGE_ENCRYPTION_KEY,
        module: "authelia",
        description: "Authelia storage encryption key",
        default: "insecure_storage_encryption_key_change_me",
        min_len: 20,
        max_len: 128,
    },
    LocalCredential {
        key: AUTHELIA_JWT_SECRET,
        module: "authelia",
        description: "Authelia identity-verification JWT secret",
        default: "insecure_jwt_secret_change_me",
        min_len: 20,
        max_len: 128,
    },
];

/// Why a value was refused by [`LocalCredentials::set`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LocalCredentialError {
    /// The key is not one of [`LOCAL_CREDENTIALS`].
    #[error("unknown local credential `{0}`")]
    UnknownKey(String),
    /// The value's length is outside the credential's bounds, or it has a character outside
    /// `[A-Za-z0-9_-]`.
    #[error(
        "local credential `{key}` must be {min_len}–{max_len} characters of [A-Za-z0-9_-] \
         (it is embedded in URLs, env files and properties files)"
    )]
    InvalidValue {
        /// The credential's key.
        key: String,
        /// Its shortest accepted length.
        min_len: usize,
        /// Its longest accepted length.
        max_len: usize,
    },
}

/// Values for the in-stack credentials (see [`LOCAL_CREDENTIALS`]); a key left unset uses its
/// development default.
///
/// Values are restricted to `[A-Za-z0-9_-]` because they are embedded verbatim in connection
/// URLs, env files and Java properties files, and to each credential's
/// [`min_len`](LocalCredential::min_len)..=[`max_len`](LocalCredential::max_len). Hex-encoded
/// random bytes of that length fit.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LocalCredentials(BTreeMap<String, String>);

impl LocalCredentials {
    /// No values set: every credential uses its development default.
    pub fn new() -> Self {
        LocalCredentials::default()
    }

    /// Build from `key → value` pairs, rejecting the first unknown key or invalid value.
    pub fn from_map(
        values: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, LocalCredentialError> {
        let mut out = LocalCredentials::new();
        for (key, value) in values {
            out.set(&key, value)?;
        }
        Ok(out)
    }

    /// Read the values kept in `secrets` under each credential's
    /// [`env_var`](LocalCredential::env_var) name; one not there keeps its default.
    pub fn from_secrets(secrets: &Secrets) -> Result<Self, LocalCredentialError> {
        let mut out = LocalCredentials::new();
        for spec in LOCAL_CREDENTIALS {
            if let Some(value) = secrets.get(&spec.env_var()) {
                out.set(spec.key, value)?;
            }
        }
        Ok(out)
    }

    /// The values set, as `secrets.env` lines (`LOCAL_…=value`) that
    /// [`from_secrets`](Self::from_secrets) reads back. Empty when none is set.
    pub fn to_secrets_env(&self) -> String {
        let mut out = String::new();
        for spec in LOCAL_CREDENTIALS {
            if let Some(value) = self.0.get(spec.key) {
                let _ = writeln!(out, "{}={value}", spec.env_var());
            }
        }
        out
    }

    /// Set the credential `key` to `value`.
    pub fn set(&mut self, key: &str, value: impl Into<String>) -> Result<(), LocalCredentialError> {
        let Some(spec) = LOCAL_CREDENTIALS.iter().find(|c| c.key == key) else {
            return Err(LocalCredentialError::UnknownKey(key.into()));
        };
        let value = value.into();
        let valid = (spec.min_len..=spec.max_len).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !valid {
            return Err(LocalCredentialError::InvalidValue {
                key: key.into(),
                min_len: spec.min_len,
                max_len: spec.max_len,
            });
        }
        self.0.insert(key.into(), value);
        Ok(())
    }

    /// The value of `key`: the one set, else its development default.
    ///
    /// # Panics
    ///
    /// If `key` is not one of [`LOCAL_CREDENTIALS`] (a catalog bug).
    pub fn get(&self, key: &str) -> &str {
        if let Some(value) = self.0.get(key) {
            return value;
        }
        LOCAL_CREDENTIALS
            .iter()
            .find(|c| c.key == key)
            .map(|c| c.default)
            .unwrap_or_else(|| panic!("unknown local credential `{key}`"))
    }

    /// Whether no value is set (every credential is its development default).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for LocalCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|k| (k, "<redacted>")))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_keys_fall_back_to_defaults() {
        let creds = LocalCredentials::new();
        assert_eq!(creds.get(POSTGRES_PASSWORD), "postgres");
        assert_eq!(creds.get(RUSTFS_ACCESS_KEY), "rustfs-root");
    }

    #[test]
    fn set_overrides_and_validates() {
        let mut creds = LocalCredentials::new();
        creds.set(POSTGRES_PASSWORD, "a".repeat(32)).unwrap();
        assert_eq!(creds.get(POSTGRES_PASSWORD), "a".repeat(32));
        assert_eq!(
            creds.set("nope", "a".repeat(32)),
            Err(LocalCredentialError::UnknownKey("nope".into()))
        );
        for bad in [
            "short",
            "has space and is long enough",
            "quote'isnotallowedhere",
        ] {
            assert!(
                matches!(
                    creds.set(POSTGRES_PASSWORD, bad),
                    Err(LocalCredentialError::InvalidValue { .. })
                ),
                "{bad}"
            );
        }
        // RustFS caps its access key at 20 characters.
        assert!(creds.set(RUSTFS_ACCESS_KEY, "a".repeat(21)).is_err());
        creds.set(RUSTFS_ACCESS_KEY, "a".repeat(20)).unwrap();
    }

    #[test]
    fn round_trips_through_secrets_env() {
        let mut creds = LocalCredentials::new();
        creds.set(RUSTFS_SECRET_KEY, "b".repeat(40)).unwrap();
        let text = creds.to_secrets_env();
        assert_eq!(
            text,
            format!("LOCAL_RUSTFS_SECRET_KEY={}\n", "b".repeat(40))
        );
        let secrets: Secrets = text.lines().filter_map(|l| l.split_once('=')).collect();
        assert_eq!(LocalCredentials::from_secrets(&secrets).unwrap(), creds);
    }

    #[test]
    fn debug_redacts_values() {
        let mut creds = LocalCredentials::new();
        creds.set(AUTHELIA_JWT_SECRET, "s3cr3t".repeat(5)).unwrap();
        let shown = format!("{creds:?}");
        assert!(shown.contains("authelia_jwt_secret"));
        assert!(!shown.contains("s3cr3t"));
    }
}
