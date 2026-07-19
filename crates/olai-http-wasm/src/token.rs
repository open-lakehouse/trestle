//! A wasm-safe temporary-token cache, API-compatible with `olai_http::TokenCache`.
//!
//! `olai_http::TokenCache` is native-only: it uses `std::time::Instant` (which
//! panics on `wasm32-unknown-unknown`) and `tokio::sync::Mutex` (tokio is not a
//! wasm dependency). This is the browser counterpart, mirroring the same public
//! surface — [`TemporaryToken`], [`TokenCache`], `Default`, [`TokenCache::with_min_ttl`],
//! and [`TokenCache::get_or_insert_with`] — so a consumer can select the cache by
//! target (`olai_http` on native, `olai_http_wasm` on wasm) and share call sites.
//!
//! It swaps [`std::time::Instant`] for [`web_time::Instant`] and `tokio::sync::Mutex`
//! for [`futures::lock::Mutex`], neither of which needs a runtime reactor, so it
//! works single-threaded in the browser and also builds/tests natively.

use std::future::Future;

use futures::lock::Mutex;
use web_time::{Duration, Instant};

/// A temporary authentication token with an associated expiry.
#[derive(Debug, Clone)]
pub struct TemporaryToken<T> {
    /// The temporary credential.
    pub token: T,
    /// The instant at which this credential is no longer valid; `None` means the
    /// credential does not expire.
    pub expiry: Option<Instant>,
}

/// Thread-safe cache for a [`TemporaryToken`] that proactively refreshes before
/// the token expires.
///
/// Mirrors `olai_http::TokenCache`'s min-TTL + fetch-backoff strategy: a cached
/// token is served only while its remaining lifetime exceeds `min_ttl` (default
/// 5 minutes); a `fetch_backoff` guard (default 100 ms) avoids a thundering herd
/// of re-fetches in the window where the token is below `min_ttl` but not yet
/// expired.
#[derive(Debug)]
pub struct TokenCache<T> {
    cache: Mutex<Option<(TemporaryToken<T>, Instant)>>,
    min_ttl: Duration,
    fetch_backoff: Duration,
}

impl<T> Default for TokenCache<T> {
    fn default() -> Self {
        Self {
            cache: Default::default(),
            min_ttl: Duration::from_secs(300),
            fetch_backoff: Duration::from_millis(100),
        }
    }
}

impl<T: Clone> TokenCache<T> {
    /// Override the minimum remaining TTL for a cached token to be used.
    pub fn with_min_ttl(self, min_ttl: Duration) -> Self {
        Self { min_ttl, ..self }
    }

    /// Return the cached token if it is still comfortably valid, otherwise fetch a
    /// fresh one via `f`, cache it, and return it.
    ///
    /// Matches `olai_http::TokenCache::get_or_insert_with` semantics.
    pub async fn get_or_insert_with<F, Fut, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<TemporaryToken<T>, E>>,
    {
        let now = Instant::now();
        let mut locked = self.cache.lock().await;

        if let Some((cached, fetched_at)) = locked.as_ref() {
            match cached.expiry {
                Some(ttl) => {
                    if ttl.checked_duration_since(now).unwrap_or_default() > self.min_ttl
                        // Recently attempted a fetch and the token is not actually
                        // expired: serve the stale-but-valid token rather than
                        // re-fetching.
                        || (fetched_at.elapsed() < self.fetch_backoff
                            && ttl.checked_duration_since(now).is_some())
                    {
                        return Ok(cached.token.clone());
                    }
                }
                None => return Ok(cached.token.clone()),
            }
        }

        let cached = f().await?;
        let token = cached.token.clone();
        *locked = Some((cached, Instant::now()));

        Ok(token)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod test {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn create_token(expiry_duration: Option<Duration>) -> TemporaryToken<String> {
        TemporaryToken {
            token: "test_token".to_string(),
            expiry: expiry_duration.map(|d| Instant::now() + d),
        }
    }

    #[test]
    fn expired_token_is_refreshed() {
        let cache = TokenCache::default();
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        async fn get_token() -> Result<TemporaryToken<String>, String> {
            COUNTER.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(create_token(Some(Duration::from_secs(0))))
        }

        futures::executor::block_on(async {
            let _ = cache.get_or_insert_with(get_token).await.unwrap();
            assert_eq!(COUNTER.load(Ordering::SeqCst), 1);

            // The token expired immediately, so the next call re-fetches.
            let _ = cache.get_or_insert_with(get_token).await.unwrap();
            assert_eq!(COUNTER.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn valid_token_within_ttl_is_reused() {
        let cache = TokenCache::default();
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        async fn get_token() -> Result<TemporaryToken<String>, String> {
            COUNTER.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(create_token(Some(Duration::from_secs(3600))))
        }

        futures::executor::block_on(async {
            let _ = cache.get_or_insert_with(get_token).await.unwrap();
            for _ in 0..5 {
                let _ = cache.get_or_insert_with(get_token).await.unwrap();
            }
            assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn non_expiring_token_is_reused() {
        let cache = TokenCache::default();
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        async fn get_token() -> Result<TemporaryToken<String>, String> {
            COUNTER.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(create_token(None))
        }

        futures::executor::block_on(async {
            let _ = cache.get_or_insert_with(get_token).await.unwrap();
            let _ = cache.get_or_insert_with(get_token).await.unwrap();
            assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
        });
    }
}
