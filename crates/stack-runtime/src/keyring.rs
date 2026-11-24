//! Keyrings: the versioned key encryption keys (KEKs) the secret server's key service wraps
//! and unwraps a module's data keys under.
//!
//! A module declares a key ([`Provides::keys`](olai_stack_topology::Provides::keys)); the
//! secret server reads its keyring from one launch variable
//! ([`keyring_var`](olai_stack_topology::plan::keyring_var)) as JSON:
//!
//! ```json
//! {"versions": {"1": "<base64 of 32 bytes>", "2": "…"}, "active": 2, "min_decrypt": 1,
//!  "aliases": {"kek-1": 1}}
//! ```
//!
//! [`Keyring`] is that value, with the operations the CLI and the desktop offer on it:
//! [`import`](Keyring::import) an existing key as version 1, [`rotate`](Keyring::rotate) to a
//! new random version, [`retire_below`](Keyring::retire_below) to raise `min_decrypt`, and
//! [`summary`](Keyring::summary) to show it without material.
//!
//! **Keyrings are add-only.** No operation removes a version's material: a retired version
//! stops unwrapping, but its key stays, so an un-swept blob can still be recovered by lowering
//! `min_decrypt` again. A caller storing a keyring checks the write with
//! [`check_extends`](Keyring::check_extends) first, which refuses one that would drop or change
//! a version (or an alias) the stored keyring has.

use std::collections::BTreeMap;
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::secrets::SecretValue;

/// The length of a key: AES-256.
pub const KEY_LEN: usize = 32;

/// Why a keyring couldn't be read, built or changed. Messages name versions and aliases,
/// never key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyringError {
    /// The stored value is not a valid keyring.
    #[error("not a valid keyring: {0}")]
    Invalid(String),
    /// Key material is not 32 bytes of standard base64.
    #[error("{0}")]
    Material(String),
    /// `retire_below(n)` with `n` above the active version.
    #[error("can't retire versions below {below}: the active version is {active}")]
    AboveActive {
        /// The requested `min_decrypt`.
        below: u32,
        /// The active version.
        active: u32,
    },
    /// A write would drop or change a version's material, or an alias, the stored keyring has.
    #[error("refusing a keyring write that would {0}: keyrings are add-only")]
    WouldDrop(String),
    /// The operating system's random number generator failed.
    #[error("no randomness for a new key: {0}")]
    Random(String),
}

/// One version's key. Wiped on drop; its `Debug` prints `<redacted>`.
#[derive(Clone, PartialEq, Eq)]
struct KeyMaterial(Zeroizing<[u8; KEY_LEN]>);

impl KeyMaterial {
    fn random() -> Result<Self, KeyringError> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(bytes.as_mut()).map_err(|e| KeyringError::Random(e.to_string()))?;
        Ok(Self(bytes))
    }

    fn from_base64(encoded: &str, what: &str) -> Result<Self, KeyringError> {
        let decoded = Zeroizing::new(
            STANDARD
                .decode(encoded.trim().as_bytes())
                .map_err(|_| KeyringError::Material(format!("{what} is not standard base64")))?,
        );
        let bytes: [u8; KEY_LEN] = decoded.as_slice().try_into().map_err(|_| {
            KeyringError::Material(format!(
                "{what} is {} bytes; a key is {KEY_LEN} bytes",
                decoded.len()
            ))
        })?;
        Ok(Self(Zeroizing::new(bytes)))
    }

    fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(self.0.as_ref()))
    }
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A key's versions, which one is active, the lowest one that still unwraps, and aliases.
///
/// The material never leaves through formatting: `Debug` shows versions and aliases only, and
/// the JSON form is a [`SecretValue`] ([`to_secret`](Self::to_secret)).
#[derive(Clone, PartialEq, Eq)]
pub struct Keyring {
    versions: BTreeMap<u32, KeyMaterial>,
    active: u32,
    min_decrypt: u32,
    aliases: BTreeMap<String, u32>,
}

/// A keyring as `trestle key show` lists it: no material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KeyringSummary {
    /// Every version, lowest first, retired ones included.
    pub versions: Vec<u32>,
    /// The version new wraps use.
    pub active: u32,
    /// The lowest version the key service unwraps.
    pub min_decrypt: u32,
    /// Other KEK ids for versions (e.g. `kek-1` for an imported `LocalKeyProvider` key).
    pub aliases: BTreeMap<String, u32>,
}

/// A string wiped on drop: a version's base64 while it is read or written.
struct Encoded(Zeroizing<String>);

impl Serialize for Encoded {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Encoded {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|s| Self(Zeroizing::new(s)))
    }
}

/// The JSON form, as the secret server reads it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeyring {
    versions: BTreeMap<String, Encoded>,
    active: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_decrypt: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    aliases: BTreeMap<String, u32>,
}

/// The KEK id of version `version` of key `key`: `<key>@v<version>`.
pub fn kek_id(key: &str, version: u32) -> String {
    format!("{key}@v{version}")
}

fn invalid(message: impl Into<String>) -> KeyringError {
    KeyringError::Invalid(message.into())
}

/// A version number as a keyring's JSON spells it: digits, no leading zeros, 1 or higher.
fn parse_version(s: &str) -> Option<u32> {
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Whether `alias` has the form of a version's own id (`…@v<n>`), which the server refuses.
fn looks_like_kek_id(alias: &str) -> bool {
    alias
        .rsplit_once("@v")
        .is_some_and(|(_, n)| parse_version(n).is_some())
}

fn check_alias(alias: &str) -> Result<(), KeyringError> {
    if alias.is_empty() || alias.len() > 255 {
        return Err(invalid("an alias must be 1 to 255 bytes"));
    }
    if looks_like_kek_id(alias) {
        return Err(invalid(format!(
            "alias {alias} has the form of a version's own id"
        )));
    }
    Ok(())
}

impl Keyring {
    /// A keyring of one version, `1`, holding an existing key: `base64` is 32 bytes in standard
    /// base64 (surrounding whitespace ignored), the form of `UC_SERVER_KEK` and of
    /// `openssl rand -base64 32`. `alias` keeps the id the key had before (`kek-1` for a
    /// `LocalKeyProvider` key), so blobs sealed under it still open.
    ///
    /// # Errors
    ///
    /// The key is not 32 bytes of standard base64, or the alias is invalid.
    pub fn import(base64: &str, alias: Option<&str>) -> Result<Self, KeyringError> {
        let material = KeyMaterial::from_base64(base64, "the key")?;
        let mut aliases = BTreeMap::new();
        if let Some(alias) = alias {
            check_alias(alias)?;
            aliases.insert(alias.to_string(), 1);
        }
        Ok(Self {
            versions: BTreeMap::from([(1, material)]),
            active: 1,
            min_decrypt: 1,
            aliases,
        })
    }

    /// A keyring of one new random version, `1`.
    ///
    /// # Errors
    ///
    /// The operating system's random number generator failed.
    pub fn generate() -> Result<Self, KeyringError> {
        Ok(Self {
            versions: BTreeMap::from([(1, KeyMaterial::random()?)]),
            active: 1,
            min_decrypt: 1,
            aliases: BTreeMap::new(),
        })
    }

    /// Reads a keyring from its JSON form, checked as the secret server checks it.
    ///
    /// # Errors
    ///
    /// The JSON is malformed, a version or alias is invalid, a key is not 32 bytes of standard
    /// base64, `active` is not one of the versions, or `min_decrypt` is not between 1 and it.
    pub fn from_json(json: &str) -> Result<Self, KeyringError> {
        // serde_json's messages can quote the input, so only its position and category leave.
        let raw: RawKeyring = serde_json::from_str(json).map_err(|e| {
            invalid(format!(
                "{:?} error at line {}, column {}",
                e.classify(),
                e.line(),
                e.column()
            ))
        })?;
        let mut versions = BTreeMap::new();
        for (version, encoded) in &raw.versions {
            let number = parse_version(version).ok_or_else(|| {
                invalid(format!(
                    "version \"{version}\" is not a number of 1 or higher"
                ))
            })?;
            versions.insert(
                number,
                KeyMaterial::from_base64(&encoded.0, &format!("version {number}"))?,
            );
        }
        let Some(&lowest) = versions.keys().next() else {
            return Err(invalid("a keyring needs at least one version"));
        };
        if !versions.contains_key(&raw.active) {
            return Err(invalid(format!(
                "active version {} is not one of the versions",
                raw.active
            )));
        }
        let min_decrypt = raw.min_decrypt.unwrap_or(lowest);
        if min_decrypt == 0 || min_decrypt > raw.active {
            return Err(invalid(format!(
                "min_decrypt {min_decrypt} must be between 1 and the active version {}",
                raw.active
            )));
        }
        for (alias, version) in &raw.aliases {
            check_alias(alias)?;
            if !versions.contains_key(version) {
                return Err(invalid(format!(
                    "alias {alias} names version {version}, which is not one of the versions"
                )));
            }
        }
        Ok(Self {
            versions,
            active: raw.active,
            min_decrypt,
            aliases: raw.aliases,
        })
    }

    /// The JSON form, as the secret server reads it from its launch variable.
    pub fn to_secret(&self) -> SecretValue {
        let raw = RawKeyring {
            versions: self
                .versions
                .iter()
                .map(|(n, k)| (n.to_string(), Encoded(k.to_base64())))
                .collect(),
            active: self.active,
            min_decrypt: Some(self.min_decrypt),
            aliases: self.aliases.clone(),
        };
        let json = Zeroizing::new(serde_json::to_string(&raw).expect("a keyring serializes"));
        SecretValue::new(json.as_str())
    }

    /// Adds a new random version and makes it active. Returns its number, one above the
    /// highest version.
    ///
    /// # Errors
    ///
    /// The operating system's random number generator failed, or the versions are exhausted.
    pub fn rotate(&mut self) -> Result<u32, KeyringError> {
        let material = KeyMaterial::random()?;
        self.add_active(material)
    }

    fn add_active(&mut self, material: KeyMaterial) -> Result<u32, KeyringError> {
        let highest = self.versions.keys().next_back().copied().unwrap_or(0);
        let next = highest
            .checked_add(1)
            .ok_or_else(|| invalid("no version above the highest"))?;
        self.versions.insert(next, material);
        self.active = next;
        Ok(next)
    }

    /// Raises `min_decrypt` to `below`, so the key service refuses to unwrap anything sealed
    /// under a lower version. Every version's material stays. Returns whether it changed
    /// (`false` when `min_decrypt` already is `below` or higher).
    ///
    /// Only safe once the consumer's sweep (olai-store's `rewrap_all`, then `kek_usage`)
    /// reports no blob below `below`: those blobs stop opening.
    ///
    /// # Errors
    ///
    /// `below` is above the active version.
    pub fn retire_below(&mut self, below: u32) -> Result<bool, KeyringError> {
        if below > self.active {
            return Err(KeyringError::AboveActive {
                below,
                active: self.active,
            });
        }
        if below <= self.min_decrypt {
            return Ok(false);
        }
        self.min_decrypt = below;
        Ok(true)
    }

    /// Checks that storing `self` in place of `stored` loses nothing: every version `stored`
    /// has is here with the same material, and so is every alias, naming the same version.
    ///
    /// # Errors
    ///
    /// [`KeyringError::WouldDrop`], naming the version or alias.
    pub fn check_extends(&self, stored: &Keyring) -> Result<(), KeyringError> {
        for (version, material) in &stored.versions {
            match self.versions.get(version) {
                None => {
                    return Err(KeyringError::WouldDrop(format!("drop version {version}")));
                }
                Some(other) if other != material => {
                    return Err(KeyringError::WouldDrop(format!(
                        "replace version {version}'s key"
                    )));
                }
                Some(_) => {}
            }
        }
        for (alias, version) in &stored.aliases {
            if self.aliases.get(alias) != Some(version) {
                return Err(KeyringError::WouldDrop(format!(
                    "drop or move alias {alias}"
                )));
            }
        }
        Ok(())
    }

    /// The version new wraps use.
    pub fn active(&self) -> u32 {
        self.active
    }

    /// The lowest version the key service unwraps.
    pub fn min_decrypt(&self) -> u32 {
        self.min_decrypt
    }

    /// Versions, active, `min_decrypt` and aliases: everything but the material.
    pub fn summary(&self) -> KeyringSummary {
        KeyringSummary {
            versions: self.versions.keys().copied().collect(),
            active: self.active,
            min_decrypt: self.min_decrypt,
            aliases: self.aliases.clone(),
        }
    }
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keyring")
            .field("versions", &self.versions.keys().collect::<Vec<_>>())
            .field("active", &self.active)
            .field("min_decrypt", &self.min_decrypt)
            .field("aliases", &self.aliases)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

    fn material(byte: u8) -> KeyMaterial {
        KeyMaterial(Zeroizing::new([byte; KEY_LEN]))
    }

    #[test]
    fn an_imported_key_is_version_one_with_its_alias() {
        let ring = Keyring::import(&format!("  {K1}\n"), Some("kek-1")).unwrap();
        let summary = ring.summary();
        assert_eq!(summary.versions, [1]);
        assert_eq!((summary.active, summary.min_decrypt), (1, 1));
        assert_eq!(summary.aliases, BTreeMap::from([("kek-1".into(), 1)]));
        let json: serde_json::Value = serde_json::from_str(ring.to_secret().expose()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "versions": {"1": K1}, "active": 1, "min_decrypt": 1, "aliases": {"kek-1": 1}
            })
        );
    }

    #[test]
    fn imports_refuse_what_is_not_a_key() {
        for bad in ["not base64!", "AQID", ""] {
            assert!(matches!(
                Keyring::import(bad, None),
                Err(KeyringError::Material(_))
            ));
        }
        assert!(Keyring::import(K1, Some("k@v2")).is_err());
        assert!(Keyring::import(K1, Some("")).is_err());
    }

    #[test]
    fn rotation_adds_an_active_version_and_keeps_the_others() {
        let mut ring = Keyring::import(K1, Some("kek-1")).unwrap();
        let before = ring.clone();
        assert_eq!(ring.rotate().unwrap(), 2);
        assert_eq!(ring.rotate().unwrap(), 3);
        assert_eq!(ring.active(), 3);
        assert_eq!(ring.min_decrypt(), 1);
        ring.check_extends(&before).unwrap();
        assert_ne!(ring.versions[&2], ring.versions[&3]);
        assert_eq!(ring.versions[&1], before.versions[&1]);
    }

    #[test]
    fn retiring_raises_min_decrypt_and_keeps_the_material() {
        let mut ring = Keyring::import(K1, None).unwrap();
        ring.rotate().unwrap();
        let before = ring.clone();
        assert_eq!(
            ring.retire_below(3),
            Err(KeyringError::AboveActive {
                below: 3,
                active: 2
            })
        );
        assert!(ring.retire_below(2).unwrap());
        assert!(!ring.retire_below(2).unwrap());
        assert!(!ring.retire_below(1).unwrap());
        assert_eq!(ring.min_decrypt(), 2);
        assert_eq!(ring.summary().versions, [1, 2]);
        ring.check_extends(&before).unwrap();
    }

    #[test]
    fn a_write_that_loses_a_version_or_alias_is_refused() {
        let mut stored = Keyring::import(K1, Some("kek-1")).unwrap();
        stored.add_active(material(2)).unwrap();

        let mut dropped = stored.clone();
        dropped.versions.remove(&1);
        dropped.aliases.clear();
        let err = dropped.check_extends(&stored).unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing a keyring write that would drop version 1: keyrings are add-only"
        );

        let mut replaced = stored.clone();
        replaced.versions.insert(2, material(9));
        assert!(matches!(
            replaced.check_extends(&stored),
            Err(KeyringError::WouldDrop(m)) if m.contains("replace version 2")
        ));

        let mut moved = stored.clone();
        moved.aliases.insert("kek-1".into(), 2);
        assert!(matches!(
            moved.check_extends(&stored),
            Err(KeyringError::WouldDrop(m)) if m.contains("alias kek-1")
        ));
    }

    #[test]
    fn the_json_form_round_trips_and_is_checked() {
        let mut ring = Keyring::import(K1, Some("kek-1")).unwrap();
        ring.rotate().unwrap();
        ring.retire_below(2).unwrap();
        let back = Keyring::from_json(ring.to_secret().expose()).unwrap();
        assert_eq!(back, ring);

        let cases = [
            r#"{"versions": {}, "active": 1}"#,
            &format!(r#"{{"versions": {{"0": "{K1}"}}, "active": 0}}"#),
            &format!(r#"{{"versions": {{"01": "{K1}"}}, "active": 1}}"#),
            &format!(r#"{{"versions": {{"1": "{K1}"}}, "active": 2}}"#),
            &format!(r#"{{"versions": {{"1": "{K1}"}}, "active": 1, "min_decrypt": 2}}"#),
            &format!(r#"{{"versions": {{"1": "{K1}"}}, "active": 1, "aliases": {{"a": 3}}}}"#),
            &format!(r#"{{"versions": {{"1": "{K1}"}}, "active": 1, "extra": 1}}"#),
            r#"{"versions": {"1": "AQID"}, "active": 1}"#,
            "not json",
        ];
        for case in cases {
            assert!(Keyring::from_json(case).is_err(), "{case}");
        }
        // Without `min_decrypt`, the lowest version.
        let json = format!(r#"{{"versions": {{"2": "{K1}", "3": "{K1}"}}, "active": 3}}"#);
        assert_eq!(Keyring::from_json(&json).unwrap().min_decrypt(), 2);
    }

    #[test]
    fn errors_and_debug_show_no_material() {
        let ring = Keyring::import(K1, Some("kek-1")).unwrap();
        let debug = format!("{ring:?}");
        assert!(!debug.contains(K1), "{debug}");
        assert!(debug.contains("kek-1"), "{debug}");
        assert_eq!(format!("{:?}", ring.versions[&1]), "<redacted>");
        assert_eq!(format!("{:?}", ring.to_secret()), "<redacted>");
        let leaked = format!(r#"{{"versions": {{"1": "{K1}"}}, "active": 1, "nope": "{K1}"}}"#);
        let err = Keyring::from_json(&leaked).unwrap_err().to_string();
        assert!(!err.contains(K1), "{err}");
    }

    #[test]
    fn generated_keys_differ() {
        let a = Keyring::generate().unwrap();
        let b = Keyring::generate().unwrap();
        assert_ne!(a.versions[&1], b.versions[&1]);
        assert_eq!(kek_id("server-kek", 2), "server-kek@v2");
    }
}
