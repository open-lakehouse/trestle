use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::BoxError;

/// The prefix every API key starts with.
pub const API_KEY_PREFIX: &str = "olk_";

/// The [`Principal::issuer`](crate::Principal::issuer) of a caller that authenticated
/// with an API key.
pub const API_KEY_ISSUER: &str = "urn:olai-auth:api-key";

/// The id of a well-formed API key, or `None` if `key` is not one.
///
/// A key is `olk_<id>_<secret>`: `id` is 16 lowercase hex characters (8 bytes) and
/// `secret` 64 (32 bytes). The id is public and is what the [`ApiKeyStore`] looks up;
/// only the SHA-256 of the whole key string is stored.
///
/// ```
/// let key = format!("olk_{}_{}", "0a".repeat(8), "f1".repeat(32));
/// assert_eq!(olai_auth::parse_api_key(&key), Some("0a0a0a0a0a0a0a0a"));
/// assert_eq!(olai_auth::parse_api_key("olk_nope"), None);
/// ```
pub fn parse_api_key(key: &str) -> Option<&str> {
    let (id, secret) = key.strip_prefix(API_KEY_PREFIX)?.split_once('_')?;
    (is_lower_hex(id, 16) && is_lower_hex(secret, 64)).then_some(id)
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The SHA-256 of the whole key string, as an [`ApiKeyRecord`] stores it.
pub fn api_key_digest(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

/// A live API key as the [`ApiKeyStore`] knows it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKeyRecord {
    /// The SHA-256 of the whole key string ([`api_key_digest`]).
    pub sha256: [u8; 32],
    /// The id of the service account the key belongs to.
    pub account_id: String,
    /// The service account's name.
    pub account_name: String,
    /// The scopes the key grants.
    pub scopes: Vec<String>,
}

impl fmt::Debug for ApiKeyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyRecord")
            .field("sha256", &"<redacted>")
            .field("account_id", &self.account_id)
            .field("account_name", &self.account_name)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl ApiKeyRecord {
    /// Whether `key` hashes to this record's digest, compared in constant time.
    pub fn matches(&self, key: &str) -> bool {
        api_key_digest(key).ct_eq(&self.sha256).into()
    }
}

/// Looks up API keys by id, implemented by the host against its key table.
///
/// [`Authenticator`](crate::Authenticator) compares the digest itself and caches
/// answers for a short time (see
/// [`api_key_cache_ttl`](crate::AuthenticatorBuilder::api_key_cache_ttl)).
#[async_trait]
pub trait ApiKeyStore: Send + Sync + 'static {
    /// The key with `id`, or `None` unless it is live: not revoked, not expired, and
    /// its service account not disabled.
    async fn lookup(&self, id: &str) -> Result<Option<ApiKeyRecord>, BoxError>;
}

/// The most keys the cache holds before it drops entries.
const MAX_ENTRIES: usize = 10_000;

/// An [`ApiKeyStore`] wrapper that remembers live keys for `ttl`.
///
/// Only hits are cached, so a key issued a moment ago works at once, and a revoked key
/// stops working once its entry expires.
pub(crate) struct CachedApiKeys {
    store: Box<dyn ApiKeyStore>,
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, ApiKeyRecord)>>,
}

impl CachedApiKeys {
    pub(crate) fn new(store: Box<dyn ApiKeyStore>, ttl: Duration) -> Self {
        Self {
            store,
            ttl,
            entries: Mutex::default(),
        }
    }

    pub(crate) async fn lookup(&self, id: &str) -> Result<Option<ApiKeyRecord>, BoxError> {
        if self.ttl.is_zero() {
            return self.store.lookup(id).await;
        }
        if let Some((at, record)) = self.lock().get(id)
            && at.elapsed() < self.ttl
        {
            return Ok(Some(record.clone()));
        }
        let Some(record) = self.store.lookup(id).await? else {
            self.lock().remove(id);
            return Ok(None);
        };
        let mut entries = self.lock();
        if entries.len() >= MAX_ENTRIES {
            entries.retain(|_, (at, _)| at.elapsed() < self.ttl);
            if entries.len() >= MAX_ENTRIES {
                entries.clear();
            }
        }
        entries.insert(id.to_owned(), (Instant::now(), record.clone()));
        Ok(Some(record))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, ApiKeyRecord)>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, RwLock};

    use super::*;

    pub(crate) const KEY: &str =
        "olk_00112233aabbccdd_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    pub(crate) fn record(key: &str) -> ApiKeyRecord {
        ApiKeyRecord {
            sha256: api_key_digest(key),
            account_id: "7d4c1a52-8f0e-4b8e-9a51-1f2b3c4d5e6f".into(),
            account_name: "ingest-bot".into(),
            scopes: vec!["ingest".into()],
        }
    }

    /// Holds keys by id, which tests can revoke, and counts lookups.
    #[derive(Clone, Default)]
    pub(crate) struct Keys {
        pub(crate) keys: Arc<RwLock<HashMap<String, ApiKeyRecord>>>,
        pub(crate) lookups: Arc<AtomicUsize>,
    }

    impl Keys {
        pub(crate) fn with(key: &str) -> Self {
            let keys = Self::default();
            let id = parse_api_key(key).unwrap().to_owned();
            keys.keys.write().unwrap().insert(id, record(key));
            keys
        }

        pub(crate) fn revoke(&self, key: &str) {
            self.keys
                .write()
                .unwrap()
                .remove(parse_api_key(key).unwrap());
        }

        pub(crate) fn lookups(&self) -> usize {
            self.lookups.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ApiKeyStore for Keys {
        async fn lookup(&self, id: &str) -> Result<Option<ApiKeyRecord>, BoxError> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(self.keys.read().unwrap().get(id).cloned())
        }
    }

    #[test]
    fn parses_well_formed_keys_only() {
        assert_eq!(parse_api_key(KEY), Some("00112233aabbccdd"));
        let good = format!("olk_{}_{}", "a".repeat(16), "b".repeat(64));
        assert!(parse_api_key(&good).is_some());
        for bad in [
            "",
            "olk_",
            "olk__",
            &good.replacen("olk_", "olx_", 1),
            &good[..good.len() - 1],
            &good.to_uppercase(),
            &format!("olk_{}_{}", "a".repeat(15), "b".repeat(65)),
            &format!("olk_{}_{}", "g".repeat(16), "b".repeat(64)),
            &format!("{good}_extra"),
            &format!(" {good}"),
        ] {
            assert_eq!(parse_api_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn matches_the_digest_of_the_whole_key() {
        let record = record(KEY);
        assert!(record.matches(KEY));
        // Same id, different secret.
        let other = format!("{}{}", &KEY[..KEY.len() - 1], "0");
        assert!(!record.matches(&other));
        // The digest covers the prefix and id too, not just the secret.
        assert_ne!(api_key_digest(KEY), api_key_digest(&KEY[4..]));
    }

    #[test]
    fn debug_redacts_the_digest() {
        let shown = format!("{:?}", record(KEY));
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("ingest-bot"), "{shown}");
        assert!(
            !shown.contains(&format!("{:?}", api_key_digest(KEY))),
            "{shown}"
        );
    }

    #[tokio::test]
    async fn caches_live_keys_for_the_ttl() {
        let store = Keys::with(KEY);
        let cached = CachedApiKeys::new(Box::new(store.clone()), Duration::from_secs(3600));
        let id = parse_api_key(KEY).unwrap();
        assert!(cached.lookup(id).await.unwrap().is_some());
        assert!(cached.lookup(id).await.unwrap().is_some());
        assert_eq!(store.lookups(), 1);
    }

    #[tokio::test]
    async fn does_not_cache_misses() {
        let store = Keys::default();
        let cached = CachedApiKeys::new(Box::new(store.clone()), Duration::from_secs(3600));
        let id = parse_api_key(KEY).unwrap();
        assert!(cached.lookup(id).await.unwrap().is_none());
        // Issued after the miss: found on the next request.
        store
            .keys
            .write()
            .unwrap()
            .insert(id.to_owned(), record(KEY));
        assert!(cached.lookup(id).await.unwrap().is_some());
        assert_eq!(store.lookups(), 2);
    }

    #[tokio::test]
    async fn revocation_takes_effect_after_the_ttl() {
        let store = Keys::with(KEY);
        let ttl = Duration::from_millis(50);
        let cached = CachedApiKeys::new(Box::new(store.clone()), ttl);
        let id = parse_api_key(KEY).unwrap();
        assert!(cached.lookup(id).await.unwrap().is_some());
        store.revoke(KEY);
        tokio::time::sleep(ttl * 2).await;
        assert!(cached.lookup(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn zero_ttl_always_asks_the_store() {
        let store = Keys::with(KEY);
        let cached = CachedApiKeys::new(Box::new(store.clone()), Duration::ZERO);
        let id = parse_api_key(KEY).unwrap();
        cached.lookup(id).await.unwrap();
        store.revoke(KEY);
        assert!(cached.lookup(id).await.unwrap().is_none());
        assert_eq!(store.lookups(), 2);
    }
}
