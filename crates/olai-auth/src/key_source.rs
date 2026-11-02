use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ed25519_dalek::VerifyingKey;

use crate::error::{AuthError, BoxError, InvalidToken};
use crate::jwk::{JwkError, JwkSet};

/// Where an issuer's signing keys come from.
///
/// Each issuer has its own source. [`JwtVerifier`](crate::JwtVerifier) caches the keys
/// and calls [`fetch`](Self::fetch) again only when a token names a `kid` it doesn't
/// know, at most once per refresh interval.
#[async_trait]
pub trait KeySource: Send + Sync + 'static {
    /// The issuer's current keys.
    async fn fetch(&self) -> Result<JwkSet, BoxError>;
}

/// A [`KeySource`] the host fills in.
///
/// For hosts that already have an HTTP client (or get the keys some other way, such as
/// from a JavaScript runtime) and don't want this crate to link one. Clones share the
/// same keys, so keep one and call [`set_jwks`](Self::set_jwks) whenever the issuer's
/// keys change. Since a refresh only reads memory, the verifier's refresh interval can
/// be [`Duration::ZERO`].
#[derive(Debug, Clone, Default)]
pub struct PushKeySource {
    keys: Arc<RwLock<JwkSet>>,
}

impl PushKeySource {
    /// A source with no keys yet: every token is rejected until keys are pushed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the keys with those in a JWKS document.
    ///
    /// # Errors
    ///
    /// When the document can't be parsed. The current keys are kept.
    pub fn set_jwks(&self, json: &[u8]) -> Result<(), JwkError> {
        self.set(JwkSet::from_json(json)?);
        Ok(())
    }

    /// Replaces the keys.
    pub fn set(&self, keys: JwkSet) {
        *self.keys.write().unwrap_or_else(|e| e.into_inner()) = keys;
    }
}

#[async_trait]
impl KeySource for PushKeySource {
    async fn fetch(&self) -> Result<JwkSet, BoxError> {
        Ok(self.keys.read().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

/// A [`KeySource`] that fetches a JWKS document over HTTP(S).
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
#[derive(Debug, Clone)]
pub struct HttpKeySource {
    client: reqwest::Client,
    url: String,
}

#[cfg(feature = "http")]
impl HttpKeySource {
    /// Fetches `url` with a default client.
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_client(reqwest::Client::new(), url)
    }

    /// Fetches `url` with `client`, to share its connection pool and settings.
    pub fn with_client(client: reqwest::Client, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
        }
    }
}

#[cfg(feature = "http")]
#[async_trait]
impl KeySource for HttpKeySource {
    async fn fetch(&self) -> Result<JwkSet, BoxError> {
        let body = self
            .client
            .get(&self.url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(JwkSet::from_json(&body)?)
    }
}

/// The time between refreshes when the verifier isn't configured otherwise.
pub(crate) const DEFAULT_MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Keys from a [`KeySource`], refreshed when a `kid` is missing.
pub(crate) struct KeyCache {
    source: Box<dyn KeySource>,
    keys: RwLock<Arc<JwkSet>>,
    /// Held across a fetch, so concurrent misses share one.
    refresh: tokio::sync::Mutex<RefreshState>,
    pub(crate) min_refresh_interval: Duration,
}

#[derive(Default)]
struct RefreshState {
    /// When the last fetch started, successful or not.
    last_attempt: Option<Instant>,
    /// Whether that fetch failed.
    failed: bool,
}

impl KeyCache {
    pub(crate) fn new(source: impl KeySource) -> Self {
        Self {
            source: Box::new(source),
            keys: RwLock::default(),
            refresh: tokio::sync::Mutex::default(),
            min_refresh_interval: DEFAULT_MIN_REFRESH_INTERVAL,
        }
    }

    fn cached(&self, kid: &str) -> Option<VerifyingKey> {
        self.keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(kid)
            .copied()
    }

    /// The key with `kid`, refreshing the cache if it isn't there.
    ///
    /// Refreshes are rate-limited, so tokens with made-up `kid`s can't turn into a
    /// stream of fetches: within the interval after a fetch, a miss is
    /// [`InvalidToken::UnknownKey`], or [`AuthError::Unavailable`] if that fetch failed.
    pub(crate) async fn key(&self, kid: &str) -> Result<VerifyingKey, AuthError> {
        if let Some(key) = self.cached(kid) {
            return Ok(key);
        }
        let mut state = self.refresh.lock().await;
        // Another request may have refreshed while this one waited for the lock.
        if let Some(key) = self.cached(kid) {
            return Ok(key);
        }
        if let Some(last) = state.last_attempt
            && last.elapsed() < self.min_refresh_interval
        {
            return Err(if state.failed {
                AuthError::Unavailable("signing keys could not be fetched".into())
            } else {
                InvalidToken::UnknownKey.into()
            });
        }
        state.last_attempt = Some(Instant::now());
        let keys = match self.source.fetch().await {
            Ok(keys) => keys,
            Err(err) => {
                state.failed = true;
                return Err(AuthError::Unavailable(err));
            }
        };
        state.failed = false;
        let key = keys.get(kid).copied();
        *self.keys.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(keys);
        key.ok_or(InvalidToken::UnknownKey.into())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::test_support::{jwks, signing_key};

    /// Serves the JWKS documents in turn (repeating the last) and counts fetches.
    #[derive(Clone)]
    pub(crate) struct Counting {
        docs: Arc<Vec<Result<Vec<u8>, &'static str>>>,
        pub(crate) fetches: Arc<AtomicUsize>,
    }

    impl Counting {
        pub(crate) fn new(docs: Vec<Result<Vec<u8>, &'static str>>) -> Self {
            Self {
                docs: Arc::new(docs),
                fetches: Arc::default(),
            }
        }

        pub(crate) fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl KeySource for Counting {
        async fn fetch(&self) -> Result<JwkSet, BoxError> {
            let n = self.fetches.fetch_add(1, Ordering::SeqCst);
            match &self.docs[n.min(self.docs.len() - 1)] {
                Ok(doc) => Ok(JwkSet::from_json(doc)?),
                Err(msg) => Err((*msg).into()),
            }
        }
    }

    fn cache(source: &Counting, interval: Duration) -> KeyCache {
        let mut cache = KeyCache::new(source.clone());
        cache.min_refresh_interval = interval;
        cache
    }

    #[tokio::test]
    async fn fetches_on_first_use_then_serves_from_cache() {
        let source = Counting::new(vec![Ok(jwks(&[("k1", 1)]))]);
        let cache = cache(&source, Duration::ZERO);
        assert_eq!(
            cache.key("k1").await.unwrap(),
            signing_key(1).verifying_key()
        );
        assert_eq!(
            cache.key("k1").await.unwrap(),
            signing_key(1).verifying_key()
        );
        assert_eq!(source.fetches(), 1);
    }

    #[tokio::test]
    async fn unknown_kid_refreshes_and_picks_up_rotated_keys() {
        let source = Counting::new(vec![
            Ok(jwks(&[("k1", 1)])),
            Ok(jwks(&[("k1", 1), ("k2", 2)])),
        ]);
        let cache = cache(&source, Duration::ZERO);
        cache.key("k1").await.unwrap();
        assert_eq!(
            cache.key("k2").await.unwrap(),
            signing_key(2).verifying_key()
        );
        assert_eq!(source.fetches(), 2);
    }

    #[tokio::test]
    async fn unknown_kid_refresh_is_rate_limited() {
        let source = Counting::new(vec![Ok(jwks(&[("k1", 1)]))]);
        let cache = cache(&source, Duration::from_secs(3600));
        cache.key("k1").await.unwrap();
        for kid in ["bogus-1", "bogus-2", "bogus-3"] {
            assert!(matches!(
                cache.key(kid).await,
                Err(AuthError::InvalidToken(InvalidToken::UnknownKey))
            ));
        }
        assert_eq!(source.fetches(), 1);
    }

    #[tokio::test]
    async fn failed_fetch_is_unavailable_until_the_next_refresh() {
        let source = Counting::new(vec![Err("down"), Ok(jwks(&[("k1", 1)]))]);
        let limited = cache(&source, Duration::from_secs(3600));
        assert!(matches!(
            limited.key("k1").await,
            Err(AuthError::Unavailable(_))
        ));
        // Rate-limited: still unavailable, without another fetch.
        assert!(matches!(
            limited.key("k1").await,
            Err(AuthError::Unavailable(_))
        ));
        assert_eq!(source.fetches(), 1);

        let source = Counting::new(vec![Err("down"), Ok(jwks(&[("k1", 1)]))]);
        let unlimited = cache(&source, Duration::ZERO);
        assert!(unlimited.key("k1").await.is_err());
        assert!(unlimited.key("k1").await.is_ok());
    }

    #[tokio::test]
    async fn push_source_serves_the_latest_keys() {
        let source = PushKeySource::new();
        assert!(source.fetch().await.unwrap().is_empty());
        source.set_jwks(&jwks(&[("k1", 1)])).unwrap();
        // A clone shares the keys, so the host can keep one and the verifier another.
        let mut cache = KeyCache::new(source.clone());
        cache.min_refresh_interval = Duration::ZERO;
        assert!(cache.key("k1").await.is_ok());
        assert!(cache.key("k2").await.is_err());

        source.set_jwks(&jwks(&[("k2", 2)])).unwrap();
        assert!(cache.key("k2").await.is_ok());
        assert!(source.set_jwks(b"garbage").is_err());
        assert_eq!(source.fetch().await.unwrap().len(), 1);
    }

    #[cfg(feature = "http")]
    #[tokio::test]
    async fn http_source_fetches_the_jwks_url() {
        let mut server = mockito::Server::new_async().await;
        let ok = server
            .mock("GET", "/jwks.json")
            .with_header("content-type", "application/json")
            .with_body(jwks(&[("k1", 1)]))
            .create_async()
            .await;
        let source = HttpKeySource::new(format!("{}/jwks.json", server.url()));
        let keys = source.fetch().await.unwrap();
        assert_eq!(keys.get("k1"), Some(&signing_key(1).verifying_key()));
        ok.assert_async().await;

        let missing = HttpKeySource::new(format!("{}/missing", server.url()));
        server
            .mock("GET", "/missing")
            .with_status(404)
            .create_async()
            .await;
        assert!(missing.fetch().await.is_err());
    }
}
