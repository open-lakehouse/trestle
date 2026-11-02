use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;

/// A JWKS document that can't be used.
#[derive(Debug, thiserror::Error)]
#[error("invalid JWKS document: {0}")]
pub struct JwkError(#[from] serde_json::Error);

/// Ed25519 verification keys by `kid`, parsed from a JWKS document (RFC 7517).
///
/// Only `OKP` keys on the `Ed25519` curve that have a `kid` are kept. Other keys (RSA,
/// EC, encryption keys) are skipped rather than rejected, so an issuer can publish them
/// alongside.
#[derive(Debug, Clone, Default)]
pub struct JwkSet {
    keys: HashMap<String, VerifyingKey>,
}

#[derive(Deserialize)]
struct RawSet {
    keys: Vec<RawKey>,
}

#[derive(Deserialize)]
struct RawKey {
    kty: String,
    crv: Option<String>,
    x: Option<String>,
    kid: Option<String>,
    alg: Option<String>,
    #[serde(rename = "use")]
    use_: Option<String>,
}

impl RawKey {
    fn into_entry(self) -> Option<(String, VerifyingKey)> {
        if self.kty != "OKP" || self.crv.as_deref() != Some("Ed25519") {
            return None;
        }
        // `Ed25519` is the fully specified name for the same algorithm (RFC 9864).
        if self
            .alg
            .as_deref()
            .is_some_and(|alg| alg != "EdDSA" && alg != "Ed25519")
        {
            return None;
        }
        if self.use_.as_deref().is_some_and(|use_| use_ != "sig") {
            return None;
        }
        let kid = self.kid?;
        let x = URL_SAFE_NO_PAD.decode(self.x?).ok()?;
        let Ok(key) = VerifyingKey::try_from(x.as_slice()) else {
            tracing::warn!(kid, "skipping JWK with an invalid Ed25519 public key");
            return None;
        };
        Some((kid, key))
    }
}

impl JwkSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a JWKS document (`{"keys": [...]}`), keeping its Ed25519 signing keys.
    ///
    /// # Errors
    ///
    /// When the document is not JSON or has no `keys` array.
    pub fn from_json(json: &[u8]) -> Result<Self, JwkError> {
        let raw: RawSet = serde_json::from_slice(json)?;
        Ok(Self {
            keys: raw
                .keys
                .into_iter()
                .filter_map(RawKey::into_entry)
                .collect(),
        })
    }

    /// Adds `key` under `kid`, replacing any key with the same `kid`.
    pub fn insert(&mut self, kid: impl Into<String>, key: VerifyingKey) {
        self.keys.insert(kid.into(), key);
    }

    /// The key with `kid`, if the set has one.
    pub fn get(&self, kid: &str) -> Option<&VerifyingKey> {
        self.keys.get(kid)
    }

    /// The number of keys.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the set has no keys.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{jwks, signing_key};

    #[test]
    fn keeps_only_ed25519_signing_keys() {
        let ed = URL_SAFE_NO_PAD.encode(signing_key(1).verifying_key().as_bytes());
        let doc = serde_json::json!({ "keys": [
            { "kty": "OKP", "crv": "Ed25519", "x": ed, "kid": "a", "alg": "EdDSA", "use": "sig" },
            { "kty": "OKP", "crv": "Ed25519", "x": ed, "kid": "b", "alg": "Ed25519" },
            { "kty": "OKP", "crv": "Ed25519", "x": ed },
            { "kty": "OKP", "crv": "Ed25519", "x": ed, "kid": "enc", "use": "enc" },
            { "kty": "OKP", "crv": "Ed25519", "x": ed, "kid": "es", "alg": "ES256" },
            { "kty": "OKP", "crv": "X25519", "x": ed, "kid": "x" },
            { "kty": "RSA", "n": "AQAB", "e": "AQAB", "kid": "rsa" },
            { "kty": "OKP", "crv": "Ed25519", "x": "AAAA", "kid": "short" },
        ]});
        let set = JwkSet::from_json(doc.to_string().as_bytes()).unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.get("a"), Some(&signing_key(1).verifying_key()));
        assert!(set.get("b").is_some());
    }

    #[test]
    fn round_trips_test_documents() {
        let set = JwkSet::from_json(&jwks(&[("k1", 1), ("k2", 2)])).unwrap();
        assert_eq!(set.get("k2"), Some(&signing_key(2).verifying_key()));
    }

    #[test]
    fn rejects_documents_without_keys() {
        assert!(JwkSet::from_json(b"{}").is_err());
        assert!(JwkSet::from_json(b"not json").is_err());
    }
}
