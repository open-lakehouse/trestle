//! A KMS-style key service: services wrap and unwrap their data keys through a server that
//! holds the key encryption keys (KEKs), so a service never holds a KEK itself.
//!
//! - [`v1`] is the `olai.keys.v1` gRPC API (`Wrap`, `Unwrap`, `GetKey`, `WatchKey`), generated
//!   from `proto/olai/keys/v1/keys.proto` with prost and tonic 0.14, client and server stubs
//!   both. `olai-secret-server` serves it on the same per-consumer Unix sockets as SDS.
//! - With the `client` feature, [`KeyServiceProvider`] is an [`olai_store::KeyProvider`]
//!   over a consumer's socket, for [`olai_store::EnvelopeEncryptor`].
//!
//! # KEK ids and the wrap format
//!
//! A key has numbered versions. The KEK id a sealed value records is `<key>@v<version>`, or
//! an alias the keyring maps to a version (`kek-1`, say, for a key imported from an earlier
//! setup). A wrapped DEK is AES-256-GCM: a random 96-bit nonce, then the ciphertext and tag,
//! with the KEK id as the associated data. That is exactly what olai-store's
//! [`LocalKeyProvider`](olai_store::LocalKeyProvider) produces for a KEK of that id, so a
//! key moves between the two without re-sealing anything.
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::fmt;

/// The `olai.keys.v1` API: messages, the `KeyService` client and the server trait.
///
/// [`v1::WrapRequest`] and [`v1::UnwrapResponse`] carry a plaintext DEK. Their `Debug`
/// prints `<redacted>` in its place.
#[allow(clippy::all, missing_docs)]
pub mod v1 {
    include!("gen/olai.keys.v1.rs");
}

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use client::{KeyServiceProvider, KeyServiceProviderBuilder};

/// The KEK id of `version` of `key`: `<key>@v<version>`.
///
/// This is the id a value sealed through the key service records, and the associated data
/// of its wrapped DEK.
pub fn kek_id(key: &str, version: u32) -> String {
    format!("{key}@v{version}")
}

/// The version in `kek_id` when it is the canonical id of a version of `key`
/// (`<key>@v<version>`, with version 1 or higher and no leading zeros).
pub fn parse_kek_id(key: &str, kek_id: &str) -> Option<u32> {
    let digits = kek_id.strip_prefix(key)?.strip_prefix("@v")?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

impl fmt::Debug for v1::WrapRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WrapRequest")
            .field("key", &self.key)
            .field("dek", &"<redacted>")
            .field("version", &self.version)
            .field("alias", &self.alias)
            .finish()
    }
}

impl fmt::Debug for v1::UnwrapResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnwrapResponse")
            .field("dek", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kek_ids_round_trip() {
        assert_eq!(kek_id("server-kek", 3), "server-kek@v3");
        assert_eq!(parse_kek_id("server-kek", "server-kek@v3"), Some(3));
        assert_eq!(parse_kek_id("server-kek", "server-kek@v12"), Some(12));
    }

    #[test]
    fn rejects_non_canonical_ids() {
        for id in [
            "kek-1",
            "server-kek",
            "server-kek@v",
            "server-kek@v0",
            "server-kek@v01",
            "server-kek@v+1",
            "server-kek@v1x",
            "other@v1",
            "server-kek@v99999999999",
        ] {
            assert_eq!(parse_kek_id("server-kek", id), None, "{id}");
        }
    }

    #[test]
    fn debug_redacts_the_dek() {
        let request = v1::WrapRequest {
            key: "k".into(),
            dek: b"hunter2-plaintext-dek".to_vec(),
            ..Default::default()
        };
        let response = v1::UnwrapResponse {
            dek: b"hunter2-plaintext-dek".to_vec(),
        };
        let rendered = format!("{request:?} {response:#?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("104, 117"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }
}
