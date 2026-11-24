//! Keyrings: the versioned key encryption keys (KEKs) the key service wraps and unwraps
//! under.
//!
//! A keyring arrives in one environment variable as JSON:
//!
//! ```json
//! {"versions": {"1": "<base64 of 32 bytes>", "2": "…"}, "active": 2, "min_decrypt": 1,
//!  "aliases": {"kek-1": 1}}
//! ```
//!
//! - `versions`: each version's AES-256 key, standard base64. Versions are 1 or higher.
//! - `active`: the version new wraps use.
//! - `min_decrypt` (default: the lowest version): the lowest version the server unwraps.
//! - `aliases` (default: none): other KEK ids for versions, such as the id a key had in an
//!   earlier `LocalKeyProvider` setup.
//!
//! A wrap under version `n` of key `k` binds the KEK id `k@vn` as associated data. A wrap
//! under an alias binds the alias. That is olai-store's `LocalKeyProvider` format with those
//! ids, which is what this module wraps with, so blobs sealed by such a provider unwrap here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use olai_key_service::v1::KeyInfo;
use olai_key_service::{kek_id, parse_kek_id};
use olai_store::{KeyProvider, LocalKeyProvider};
use serde::Deserialize;
use tokio::sync::watch;
use zeroize::Zeroizing;

/// A KEK id is recorded in each envelope behind a one-byte length.
const MAX_KEK_ID_LEN: usize = 255;
/// The longest key name, so that its KEK ids (`<name>@v<u32>`) fit in the 255 bytes an
/// envelope records.
pub const MAX_KEY_NAME_LEN: usize = MAX_KEK_ID_LEN - "@v4294967295".len();
/// AES-256.
const KEK_LEN: usize = 32;
/// The largest DEK the service wraps.
pub const MAX_DEK_LEN: usize = 1024;

/// Why a keyring could not be read. Messages name versions and aliases, never key bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct KeyringError(String);

/// Why a wrap or unwrap was refused. Messages name keys and versions, never material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The version or alias does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The version exists but may not be used for this operation now.
    #[error("{0}")]
    FailedPrecondition(String),
    /// The request is malformed, or the wrapped DEK does not authenticate.
    #[error("{0}")]
    InvalidArgument(String),
}

/// One key's versions and their material.
///
/// The material never leaves through formatting: `Debug` shows versions and aliases only, and
/// there is no `Display` or `Serialize`. It is wiped on drop.
pub struct Keyring {
    name: String,
    versions: BTreeSet<u32>,
    active: u32,
    min_decrypt: u32,
    aliases: BTreeMap<String, u32>,
    /// One entry per version (`<name>@v<n>`) and per alias, each with its version's key.
    provider: LocalKeyProvider,
}

/// A string that is wiped on drop, for the base64 key material while it is parsed.
struct SecretString(Zeroizing<String>);

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|s| Self(Zeroizing::new(s)))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeyring {
    versions: BTreeMap<String, SecretString>,
    active: u32,
    min_decrypt: Option<u32>,
    #[serde(default)]
    aliases: BTreeMap<String, u32>,
}

/// A version number as a keyring's JSON spells it: digits, no leading zeros, 1 or higher.
fn parse_version(s: &str) -> Option<u32> {
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn invalid(message: impl Into<String>) -> KeyringError {
    KeyringError(message.into())
}

impl Keyring {
    /// Reads the keyring of key `name` from its JSON form.
    ///
    /// # Errors
    ///
    /// The JSON is malformed, a version or alias is invalid, a key is not 32 bytes of
    /// standard base64, `active` is not one of the versions, or `min_decrypt` is above it.
    pub fn from_json(name: &str, json: &[u8]) -> Result<Self, KeyringError> {
        if name.is_empty() || name.len() > MAX_KEY_NAME_LEN {
            return Err(invalid(format!(
                "a key name must be 1 to {MAX_KEY_NAME_LEN} bytes"
            )));
        }
        // serde_json's messages can quote the input, so only its position and category leave.
        let raw: RawKeyring = serde_json::from_slice(json).map_err(|e| {
            invalid(format!(
                "not a keyring: {:?} error at line {}, column {} (expected \
                 {{\"versions\": {{\"1\": \"<base64>\"}}, \"active\": 1}})",
                e.classify(),
                e.line(),
                e.column()
            ))
        })?;

        let mut keys: Vec<(String, Vec<u8>)> = Vec::new();
        let mut material: BTreeMap<u32, Zeroizing<Vec<u8>>> = BTreeMap::new();
        for (version, encoded) in &raw.versions {
            let number = parse_version(version).ok_or_else(|| {
                invalid(format!(
                    "version \"{version}\" is not a number of 1 or higher"
                ))
            })?;
            let bytes = Zeroizing::new(
                STANDARD
                    .decode(encoded.0.as_bytes())
                    .map_err(|_| invalid(format!("version {number} is not standard base64")))?,
            );
            if bytes.len() != KEK_LEN {
                return Err(invalid(format!(
                    "version {number} is {} bytes; a key is {KEK_LEN} bytes",
                    bytes.len()
                )));
            }
            material.insert(number, bytes);
        }
        if material.is_empty() {
            return Err(invalid("a keyring needs at least one version"));
        }
        let versions: BTreeSet<u32> = material.keys().copied().collect();
        if !versions.contains(&raw.active) {
            return Err(invalid(format!(
                "active version {} is not one of the versions",
                raw.active
            )));
        }
        let lowest = *versions.first().expect("not empty");
        let min_decrypt = raw.min_decrypt.unwrap_or(lowest);
        if min_decrypt == 0 || min_decrypt > raw.active {
            return Err(invalid(format!(
                "min_decrypt {min_decrypt} must be between 1 and the active version {}",
                raw.active
            )));
        }
        for (alias, version) in &raw.aliases {
            if alias.is_empty() || alias.len() > MAX_KEK_ID_LEN {
                return Err(invalid(format!(
                    "an alias must be 1 to {MAX_KEK_ID_LEN} bytes"
                )));
            }
            if parse_kek_id(name, alias).is_some() {
                return Err(invalid(format!(
                    "alias {alias} has the form of a version's own id"
                )));
            }
            let Some(bytes) = material.get(version) else {
                return Err(invalid(format!(
                    "alias {alias} names version {version}, which is not one of the versions"
                )));
            };
            keys.push((alias.clone(), bytes.to_vec()));
        }
        for (version, bytes) in &material {
            keys.push((kek_id(name, *version), bytes.to_vec()));
        }
        let provider = LocalKeyProvider::new(kek_id(name, raw.active), keys)
            .map_err(|_| invalid("the keyring's keys could not be loaded"))?;
        Ok(Self {
            name: name.to_string(),
            versions,
            active: raw.active,
            min_decrypt,
            aliases: raw.aliases,
            provider,
        })
    }

    /// The key's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version new wraps use.
    pub fn active(&self) -> u32 {
        self.active
    }

    /// The lowest version the keyring unwraps.
    pub fn min_decrypt(&self) -> u32 {
        self.min_decrypt
    }

    /// The versions the keyring holds, including any below `min_decrypt`.
    pub fn versions(&self) -> impl Iterator<Item = u32> + '_ {
        self.versions.iter().copied()
    }

    /// The key's versions, as the key service reports them. Holds no material.
    pub fn info(&self) -> KeyInfo {
        KeyInfo {
            key: self.name.clone(),
            active_version: self.active,
            min_decrypt_version: self.min_decrypt,
            aliases: self.aliases.clone().into_iter().collect(),
        }
    }

    /// The version and KEK id (the associated data) a request names, checked against the
    /// versions the keyring holds. `version` 0 means the active version, or the alias's.
    fn target(&self, version: u32, alias: &str) -> Result<(u32, String), KeyError> {
        if alias.is_empty() {
            let version = if version == 0 { self.active } else { version };
            if !self.versions.contains(&version) {
                return Err(KeyError::NotFound(format!(
                    "key {} has no version {version}",
                    self.name
                )));
            }
            return Ok((version, kek_id(&self.name, version)));
        }
        let Some(&aliased) = self.aliases.get(alias) else {
            return Err(KeyError::NotFound(format!(
                "key {} has no alias {alias}",
                self.name
            )));
        };
        if version != 0 && version != aliased {
            return Err(KeyError::InvalidArgument(format!(
                "alias {alias} of key {} is version {aliased}, not {version}",
                self.name
            )));
        }
        Ok((aliased, alias.to_string()))
    }

    /// Wraps `dek` under `version` (0: the active version), or under `alias`.
    ///
    /// # Errors
    ///
    /// The DEK is empty or larger than [`MAX_DEK_LEN`], the version or alias doesn't exist,
    /// or the version is outside `min_decrypt..=active`.
    pub async fn wrap(
        &self,
        dek: &[u8],
        version: u32,
        alias: &str,
    ) -> Result<(u32, Vec<u8>), KeyError> {
        if dek.is_empty() || dek.len() > MAX_DEK_LEN {
            return Err(KeyError::InvalidArgument(format!(
                "a DEK must be 1 to {MAX_DEK_LEN} bytes"
            )));
        }
        let (version, id) = self.target(version, alias)?;
        if version > self.active || version < self.min_decrypt {
            return Err(KeyError::FailedPrecondition(format!(
                "key {} wraps only under versions {} to {}, not {version}",
                self.name, self.min_decrypt, self.active
            )));
        }
        let wrapped = self
            .provider
            .wrap_dek(dek, &id)
            .await
            .map_err(|_| KeyError::InvalidArgument("the DEK could not be wrapped".into()))?;
        Ok((version, wrapped))
    }

    /// Unwraps `wrapped`, a DEK wrapped under `version` or `alias`.
    ///
    /// # Errors
    ///
    /// The version or alias doesn't exist, the version is below `min_decrypt`, or the wrapped
    /// DEK doesn't authenticate under it.
    pub async fn unwrap(
        &self,
        wrapped: &[u8],
        version: u32,
        alias: &str,
    ) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        if version == 0 && alias.is_empty() {
            return Err(KeyError::InvalidArgument(
                "an unwrap needs a version or an alias".into(),
            ));
        }
        let (version, id) = self.target(version, alias)?;
        if version < self.min_decrypt {
            return Err(KeyError::FailedPrecondition(format!(
                "version {version} of key {} is below its min_decrypt version {}",
                self.name, self.min_decrypt
            )));
        }
        self.provider.unwrap_dek(wrapped, &id).await.map_err(|_| {
            KeyError::InvalidArgument(format!(
                "the wrapped DEK does not unwrap under version {version} of key {}",
                self.name
            ))
        })
    }
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyring")
            .field("name", &self.name)
            .field("versions", &self.versions)
            .field("active", &self.active)
            .field("min_decrypt", &self.min_decrypt)
            .field("aliases", &self.aliases)
            .field("material", &"<redacted>")
            .finish()
    }
}

/// A consumer's view of one keyring: the current one, and a way to follow its changes.
///
/// Clones share the keyring. [`replace`](Self::replace) swaps it for every clone, and every
/// `WatchKey` stream sends the new versions.
#[derive(Clone)]
pub struct KeyHandle {
    current: Arc<watch::Sender<Arc<Keyring>>>,
}

impl KeyHandle {
    /// A handle on `keyring`.
    pub fn new(keyring: Keyring) -> Self {
        Self {
            current: Arc::new(watch::channel(Arc::new(keyring)).0),
        }
    }

    /// The keyring as it is now.
    pub fn current(&self) -> Arc<Keyring> {
        Arc::clone(&self.current.borrow())
    }

    /// Replaces the keyring, for example after a rotation.
    ///
    /// # Errors
    ///
    /// `keyring` is for a different key name; the handle keeps the keyring it has.
    pub fn replace(&self, keyring: Keyring) -> Result<(), KeyringError> {
        if keyring.name != self.current.borrow().name {
            return Err(invalid(format!(
                "a keyring for key {} cannot replace key {}",
                keyring.name,
                self.current.borrow().name
            )));
        }
        self.current.send_replace(Arc::new(keyring));
        Ok(())
    }

    /// Follows the keyring's changes.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Keyring>> {
        self.current.subscribe()
    }
}

impl fmt::Debug for KeyHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.current.borrow().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(byte: u8) -> String {
        STANDARD.encode([byte; KEK_LEN])
    }

    fn ring(json: &str) -> Result<Keyring, KeyringError> {
        Keyring::from_json("server-kek", json.as_bytes())
    }

    fn two_versions() -> Keyring {
        ring(&format!(
            r#"{{"versions": {{"1": "{}", "2": "{}"}}, "active": 2, "aliases": {{"kek-1": 1}}}}"#,
            b64(1),
            b64(2)
        ))
        .unwrap()
    }

    #[test]
    fn parses_versions_aliases_and_defaults() {
        let keyring = two_versions();
        assert_eq!(keyring.active(), 2);
        assert_eq!(keyring.min_decrypt(), 1);
        assert_eq!(keyring.versions().collect::<Vec<_>>(), [1, 2]);
        let info = keyring.info();
        assert_eq!(info.key, "server-kek");
        assert_eq!(info.aliases.get("kek-1"), Some(&1));
    }

    #[test]
    fn rejects_invalid_keyrings() {
        let k = b64(1);
        for (json, expected) in [
            (
                r#"{"versions": {}, "active": 1}"#.to_string(),
                "at least one version",
            ),
            (
                format!(r#"{{"versions": {{"0": "{k}"}}, "active": 0}}"#),
                "not a number of 1 or higher",
            ),
            (
                format!(r#"{{"versions": {{"01": "{k}"}}, "active": 1}}"#),
                "not a number",
            ),
            (
                format!(r#"{{"versions": {{"x": "{k}"}}, "active": 1}}"#),
                "not a number",
            ),
            (
                format!(r#"{{"versions": {{"1": "{k}"}}, "active": 2}}"#),
                "active version 2",
            ),
            (
                format!(r#"{{"versions": {{"1": "{k}"}}, "active": 1, "min_decrypt": 2}}"#),
                "min_decrypt 2",
            ),
            (
                format!(r#"{{"versions": {{"1": "{k}"}}, "active": 1, "aliases": {{"a": 3}}}}"#),
                "alias a names version 3",
            ),
            (
                format!(
                    r#"{{"versions": {{"1": "{k}"}}, "active": 1, "aliases": {{"server-kek@v1": 1}}}}"#
                ),
                "form of a version's own id",
            ),
            (
                format!(
                    r#"{{"versions": {{"1": "{}"}}, "active": 1}}"#,
                    STANDARD.encode([1u8; 16])
                ),
                "16 bytes",
            ),
            (
                r#"{"versions": {"1": "not base64!"}, "active": 1}"#.to_string(),
                "not standard base64",
            ),
            (
                format!(r#"{{"versions": {{"1": "{k}"}}, "active": 1, "extra": true}}"#),
                "not a keyring",
            ),
        ] {
            let err = ring(&json).unwrap_err().to_string();
            assert!(err.contains(expected), "{json}: {err}");
        }
    }

    #[test]
    fn errors_never_quote_the_material() {
        // The material in the wrong place: serde would quote it in its message.
        let json = r#"{"versions": {"1": "c2VjcmV0"}, "active": "hunter2hunter2"}"#;
        let err = ring(json).unwrap_err().to_string();
        assert!(!err.contains("hunter2"), "{err}");
        assert!(err.contains("line 1"), "{err}");
        let json = r#"{"versions": {"1": "hunter2hunter2"}, "active": 1}"#;
        let err = ring(json).unwrap_err().to_string();
        assert!(!err.contains("hunter2"), "{err}");
    }

    #[test]
    fn debug_shows_no_material() {
        let keyring = two_versions();
        let rendered = format!("{keyring:?} {:?}", KeyHandle::new(two_versions()));
        assert!(!rendered.contains(&b64(1)), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[tokio::test]
    async fn wraps_in_the_local_key_provider_format() {
        let keyring = two_versions();
        let dek = [7u8; 32];
        let (version, wrapped) = keyring.wrap(&dek, 0, "").await.unwrap();
        assert_eq!(version, 2);
        // A LocalKeyProvider holding version 2 under its canonical id opens it.
        let local = LocalKeyProvider::single("server-kek@v2", vec![2; KEK_LEN]).unwrap();
        assert_eq!(
            local
                .unwrap_dek(&wrapped, "server-kek@v2")
                .await
                .unwrap()
                .as_slice(),
            dek
        );
        // And the other way round, through the alias.
        let local = LocalKeyProvider::single("kek-1", vec![1; KEK_LEN]).unwrap();
        let sealed = local.wrap_dek(&dek, "kek-1").await.unwrap();
        assert_eq!(
            keyring
                .unwrap(&sealed, 0, "kek-1")
                .await
                .unwrap()
                .as_slice(),
            dek
        );
        assert_eq!(
            keyring
                .unwrap(&sealed, 1, "kek-1")
                .await
                .unwrap()
                .as_slice(),
            dek
        );
        // The alias is the associated data: the same blob does not open as version 1.
        assert!(matches!(
            keyring.unwrap(&sealed, 1, "").await,
            Err(KeyError::InvalidArgument(_))
        ));
    }

    #[tokio::test]
    async fn enforces_the_version_window() {
        let keyring = ring(&format!(
            r#"{{"versions": {{"1": "{}", "2": "{}", "3": "{}"}}, "active": 2, "min_decrypt": 2}}"#,
            b64(1),
            b64(2),
            b64(3)
        ))
        .unwrap();
        let dek = [7u8; 32];
        assert!(matches!(
            keyring.wrap(&dek, 1, "").await,
            Err(KeyError::FailedPrecondition(_))
        ));
        assert!(matches!(
            keyring.wrap(&dek, 3, "").await,
            Err(KeyError::FailedPrecondition(_))
        ));
        assert!(matches!(
            keyring.wrap(&dek, 4, "").await,
            Err(KeyError::NotFound(_))
        ));
        assert!(matches!(
            keyring.wrap(&[], 0, "").await,
            Err(KeyError::InvalidArgument(_))
        ));
        let (_, wrapped) = keyring.wrap(&dek, 2, "").await.unwrap();
        assert!(keyring.unwrap(&wrapped, 2, "").await.is_ok());
        assert!(matches!(
            keyring.unwrap(&wrapped, 1, "").await,
            Err(KeyError::FailedPrecondition(_))
        ));
        assert!(matches!(
            keyring.unwrap(&wrapped, 9, "").await,
            Err(KeyError::NotFound(_))
        ));
        assert!(matches!(
            keyring.unwrap(&wrapped, 0, "").await,
            Err(KeyError::InvalidArgument(_))
        ));
        assert!(matches!(
            keyring.unwrap(&wrapped, 0, "nope").await,
            Err(KeyError::NotFound(_))
        ));
    }

    #[test]
    fn a_handle_only_takes_its_own_key() {
        let handle = KeyHandle::new(two_versions());
        let other = Keyring::from_json(
            "other",
            format!(r#"{{"versions": {{"1": "{}"}}, "active": 1}}"#, b64(1)).as_bytes(),
        )
        .unwrap();
        assert!(handle.replace(other).is_err());
        let rx = handle.subscribe();
        handle
            .replace(
                ring(&format!(
                    r#"{{"versions": {{"3": "{}"}}, "active": 3}}"#,
                    b64(3)
                ))
                .unwrap(),
            )
            .unwrap();
        assert!(rx.has_changed().unwrap());
        assert_eq!(handle.current().active(), 3);
    }
}
