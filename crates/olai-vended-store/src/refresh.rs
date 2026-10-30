//! An object_store `CredentialProvider` that re-vends before expiry.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use futures::lock::Mutex;
use object_store::CredentialProvider;
use olai_storage_credentials::{AccessLevel, StorageScope, VendedCredential};

use crate::convert::FromStorageCredentials;
use crate::error::{Error, Result};
use crate::source::CredentialSource;

/// How long before expiry a credential is re-vended, unless configured otherwise.
pub const DEFAULT_REFRESH_MARGIN: Duration = Duration::from_secs(300);

/// The shortest time between two vends of one provider.
pub const REFRESH_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// The current time. Injectable so refresh behaviour is testable without waiting.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// An object_store `CredentialProvider` over a [`CredentialSource`].
///
/// - It is seeded with the credential the store was built from, so the first request
///   does not re-vend.
/// - It re-vends [`refresh_margin`](Self::with_refresh_margin) before expiry
///   ([`DEFAULT_REFRESH_MARGIN`] by default). A credential whose whole lifetime is
///   shorter than twice the margin is re-vended halfway through instead, so a
///   short-lived grant is not re-vended on every request.
/// - Refreshes are single-flight: concurrent requests wait for one vend.
/// - A failed refresh keeps serving the cached credential until it expires, and only
///   then fails requests. Either way the next vend waits [`REFRESH_RETRY_INTERVAL`],
///   so an outage costs one vend per interval, not one per request.
/// - A refreshed credential that is already due (a skewed clock, a server vending
///   nearly expired credentials) is used for at least [`REFRESH_RETRY_INTERVAL`]
///   before the next vend.
/// - A refreshed credential must still cover the seed's scope and access level, or
///   the refresh fails with [`Error::RefreshNarrowed`].
///
/// `T` is object_store's `AwsCredential`, `AzureCredential` or `GcpCredential`.
pub struct RefreshingCredentialProvider<T> {
    source: Arc<dyn CredentialSource>,
    scope: StorageScope,
    access: AccessLevel,
    margin: Duration,
    clock: Clock,
    state: Mutex<Cached<T>>,
    _credential: PhantomData<fn() -> T>,
}

struct Cached<T> {
    credential: Arc<T>,
    expires_at: Option<DateTime<Utc>>,
    fetched_at: DateTime<Utc>,
    /// No vend before this time.
    next_vend_at: Option<DateTime<Utc>>,
    /// Why the last vend failed, while waiting to retry it.
    failure: Option<String>,
}

impl<T> Cached<T> {
    fn new(credential: Arc<T>, expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Self {
        Self {
            credential,
            expires_at,
            fetched_at: now,
            next_vend_at: None,
            failure: None,
        }
    }

    /// Whether the credential is due for a re-vend at `now`.
    fn is_due(&self, now: DateTime<Utc>, margin: Duration) -> bool {
        let Some(expires_at) = self.expires_at else {
            return false;
        };
        let margin = TimeDelta::from_std(margin).unwrap_or(TimeDelta::MAX);
        let lead = margin.min((expires_at - self.fetched_at) / 2);
        now >= expires_at - lead.max(TimeDelta::zero())
    }

    /// Whether the credential has expired at `now`.
    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }

    /// Whether a vend must wait at `now`.
    fn is_backing_off(&self, now: DateTime<Utc>) -> bool {
        self.next_vend_at.is_some_and(|at| now < at)
    }
}

/// `now` plus [`REFRESH_RETRY_INTERVAL`].
fn retry_at(now: DateTime<Utc>) -> DateTime<Utc> {
    let interval = TimeDelta::from_std(REFRESH_RETRY_INTERVAL).unwrap_or(TimeDelta::MAX);
    now.checked_add_signed(interval)
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

impl<T: FromStorageCredentials> RefreshingCredentialProvider<T> {
    /// A provider seeded with `seed`, re-vending from `source`.
    ///
    /// `source` must be the source `seed` was vended from: refreshes are checked
    /// against the seed's scope and access level.
    pub fn new(seed: &VendedCredential, source: Arc<dyn CredentialSource>) -> Result<Self> {
        let clock: Clock = Arc::new(Utc::now);
        let state = Cached::new(
            Arc::new(T::from_storage_credentials(&seed.credentials)?),
            seed.expires_at,
            clock(),
        );
        Ok(Self {
            source,
            scope: seed.scope.clone(),
            access: seed.access,
            margin: DEFAULT_REFRESH_MARGIN,
            clock,
            state: Mutex::new(state),
            _credential: PhantomData,
        })
    }

    /// Re-vend `margin` before expiry instead of [`DEFAULT_REFRESH_MARGIN`].
    pub fn with_refresh_margin(mut self, margin: Duration) -> Self {
        self.margin = margin;
        self
    }

    /// Read the time from `clock` instead of the system clock.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.state.get_mut().fetched_at = clock();
        self.clock = clock;
        self
    }

    async fn fetch(&self) -> Result<VendedCredential> {
        #[cfg(not(target_arch = "wasm32"))]
        let vended = self.source.fetch().await?;
        // object_store needs a `Send` future; wasm32 is single-threaded, so the
        // wrapper never sees another thread.
        #[cfg(target_arch = "wasm32")]
        let vended = send_wrapper::SendWrapper::new(self.source.fetch()).await?;

        if !(vended.access.satisfies(self.access) && vended.scope.contains(&self.scope)) {
            return Err(Error::RefreshNarrowed {
                expected: self.scope.to_string(),
                scope: vended.scope.to_string(),
                access: self.access,
            });
        }
        Ok(vended)
    }

    async fn credential(&self) -> Result<Arc<T>> {
        let mut state = self.state.lock().await;
        let now = (self.clock)();
        if !state.is_due(now, self.margin) {
            return Ok(Arc::clone(&state.credential));
        }
        if state.is_backing_off(now) {
            return match &state.failure {
                Some(failure) if state.is_expired(now) => Err(Error::vend(format!(
                    "credential for {} expired and its refresh failed: {failure}",
                    self.scope
                ))),
                _ => Ok(Arc::clone(&state.credential)),
            };
        }
        let refreshed = match self.fetch().await {
            Ok(vended) => T::from_storage_credentials(&vended.credentials)
                .map(|credential| (Arc::new(credential), vended.expires_at)),
            Err(err) => Err(err),
        };
        let now = (self.clock)();
        match refreshed {
            Ok((credential, expires_at)) => {
                let mut fresh = Cached::new(Arc::clone(&credential), expires_at, now);
                if fresh.is_due(now, self.margin) {
                    fresh.next_vend_at = Some(retry_at(now));
                }
                *state = fresh;
                Ok(credential)
            }
            Err(err) => {
                state.next_vend_at = Some(retry_at(now));
                state.failure = Some(err.to_string());
                if state.is_expired(now) {
                    return Err(err);
                }
                tracing::warn!(
                    scope = %self.scope,
                    error = %err,
                    "credential refresh failed; serving the cached credential until it expires"
                );
                Ok(Arc::clone(&state.credential))
            }
        }
    }
}

impl<T> fmt::Debug for RefreshingCredentialProvider<T> {
    // No credential in here: the cached secret stays out of logs and panics.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshingCredentialProvider")
            .field("source", &self.source)
            .field("scope", &self.scope)
            .field("access", &self.access)
            .field("margin", &self.margin)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<T: FromStorageCredentials> CredentialProvider for RefreshingCredentialProvider<T> {
    type Credential = T;

    async fn get_credential(&self) -> object_store::Result<Arc<T>> {
        Ok(self.credential().await?)
    }
}

#[cfg(all(test, feature = "aws"))]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use object_store::aws::AwsCredential;
    use olai_storage_credentials::StorageCredentials;

    use super::*;

    const T0: i64 = 1_700_000_000;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(T0 + secs, 0).unwrap()
    }

    fn scope(url: &str) -> StorageScope {
        url.parse().unwrap()
    }

    fn vended(
        key: &str,
        scope_url: &str,
        access: AccessLevel,
        expires: Option<i64>,
    ) -> VendedCredential {
        VendedCredential::new(
            StorageCredentials::aws(key, "secret", Some("token".into())),
            scope(scope_url),
            access,
        )
        .with_expiry(expires.map(at))
    }

    /// A clock tests move by hand.
    #[derive(Clone)]
    struct TestClock(Arc<StdMutex<DateTime<Utc>>>);

    impl TestClock {
        fn new() -> Self {
            Self(Arc::new(StdMutex::new(at(0))))
        }
        fn set(&self, secs: i64) {
            *self.0.lock().unwrap() = at(secs);
        }
        fn clock(&self) -> Clock {
            let inner = Arc::clone(&self.0);
            Arc::new(move || *inner.lock().unwrap())
        }
    }

    /// Hands out scripted results and counts fetches.
    #[derive(Debug)]
    struct ScriptedSource {
        script: StdMutex<Vec<Result<VendedCredential, String>>>,
        fetches: AtomicUsize,
        delay: Duration,
    }

    impl ScriptedSource {
        fn new(mut script: Vec<Result<VendedCredential, String>>) -> Arc<Self> {
            script.reverse();
            Arc::new(Self {
                script: StdMutex::new(script),
                fetches: AtomicUsize::new(0),
                delay: Duration::ZERO,
            })
        }
        fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl CredentialSource for ScriptedSource {
        async fn fetch(&self) -> Result<VendedCredential> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let next = self.script.lock().unwrap().pop().expect("unexpected fetch");
            next.map_err(Error::vend)
        }
    }

    fn provider(
        seed: &VendedCredential,
        source: Arc<ScriptedSource>,
        clock: &TestClock,
    ) -> RefreshingCredentialProvider<AwsCredential> {
        RefreshingCredentialProvider::new(seed, source)
            .unwrap()
            .with_clock(clock.clock())
    }

    const S3: &str = "s3://bucket/tab";

    #[tokio::test]
    async fn seed_is_served_without_vending() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, Arc::clone(&source), &clock);

        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        assert_eq!(source.fetches(), 0);
    }

    #[tokio::test]
    async fn re_vends_at_the_margin() {
        let clock = TestClock::new();
        let source =
            ScriptedSource::new(vec![Ok(vended("fresh", S3, AccessLevel::Read, Some(7200)))]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, Arc::clone(&source), &clock);

        clock.set(3600 - 301);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        clock.set(3600 - 300);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
        assert_eq!(source.fetches(), 1);
    }

    #[tokio::test]
    async fn margin_is_configurable() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![Ok(vended("fresh", S3, AccessLevel::Read, None))]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, Arc::clone(&source), &clock)
            .with_refresh_margin(Duration::from_secs(60));

        clock.set(3600 - 61);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        clock.set(3600 - 60);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
    }

    #[tokio::test]
    async fn non_expiring_credential_is_never_re_vended() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![]);
        let seed = vended("seed", S3, AccessLevel::Read, None);
        let p = provider(&seed, Arc::clone(&source), &clock);

        clock.set(100 * 365 * 24 * 3600);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        assert_eq!(source.fetches(), 0);
    }

    #[tokio::test]
    async fn short_lived_credential_refreshes_halfway() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![Ok(vended("fresh", S3, AccessLevel::Read, None))]);
        // 120s of lifetime, well under the 300s margin.
        let seed = vended("seed", S3, AccessLevel::Read, Some(120));
        let p = provider(&seed, Arc::clone(&source), &clock);

        clock.set(59);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        clock.set(60);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
        assert_eq!(source.fetches(), 1);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_vend() {
        let clock = TestClock::new();
        let source = Arc::new(ScriptedSource {
            delay: Duration::from_millis(50),
            ..Arc::into_inner(ScriptedSource::new(vec![Ok(vended(
                "fresh",
                S3,
                AccessLevel::Read,
                Some(7200),
            ))]))
            .unwrap()
        });
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = Arc::new(provider(&seed, Arc::clone(&source), &clock));
        clock.set(3600);

        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let p = Arc::clone(&p);
                tokio::spawn(async move { p.get_credential().await.unwrap().key_id.clone() })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), "fresh");
        }
        assert_eq!(source.fetches(), 1);
    }

    #[tokio::test]
    async fn failed_vend_is_retried_after_the_interval() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![
            Err("server unavailable".into()),
            Ok(vended("fresh", S3, AccessLevel::Read, Some(7200))),
        ]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, Arc::clone(&source), &clock);
        clock.set(3600);

        let err = p.get_credential().await.unwrap_err();
        assert!(err.to_string().contains("server unavailable"), "{err}");
        // Within the interval the failure is returned without vending again.
        clock.set(3609);
        let err = p.get_credential().await.unwrap_err();
        assert!(err.to_string().contains("server unavailable"), "{err}");
        assert_eq!(source.fetches(), 1);
        clock.set(3610);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
        assert_eq!(source.fetches(), 2);
    }

    #[tokio::test]
    async fn failed_refresh_serves_the_cached_credential_until_expiry() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![
            Err("server unavailable".into()),
            Err("server unavailable".into()),
            Ok(vended("fresh", S3, AccessLevel::Read, Some(7200))),
        ]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, Arc::clone(&source), &clock);

        // Inside the margin, a failed refresh keeps the still-valid seed.
        clock.set(3600 - 240);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        clock.set(3600 - 235);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        assert_eq!(source.fetches(), 1);
        clock.set(3600 - 230);
        assert_eq!(p.get_credential().await.unwrap().key_id, "seed");
        assert_eq!(source.fetches(), 2);
        clock.set(3600 - 220);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
        assert_eq!(source.fetches(), 3);
    }

    #[tokio::test]
    async fn an_outage_costs_one_vend_per_interval() {
        let clock = TestClock::new();
        let source = Arc::new(ScriptedSource {
            delay: Duration::from_millis(20),
            ..Arc::into_inner(ScriptedSource::new(vec![Err("server unavailable".into())])).unwrap()
        });
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = Arc::new(provider(&seed, Arc::clone(&source), &clock));
        clock.set(3600);

        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let p = Arc::clone(&p);
                tokio::spawn(async move { p.get_credential().await.is_err() })
            })
            .collect();
        for task in tasks {
            assert!(task.await.unwrap());
        }
        assert_eq!(source.fetches(), 1);
    }

    #[tokio::test]
    async fn an_already_due_refresh_is_kept_for_the_interval() {
        let clock = TestClock::new();
        // The server's clock is behind: what it vends has already expired here.
        let source = ScriptedSource::new(vec![
            Ok(vended("stale", S3, AccessLevel::Read, Some(3500))),
            Ok(vended("fresh", S3, AccessLevel::Read, Some(7200))),
        ]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, Arc::clone(&source), &clock);

        clock.set(3600);
        assert_eq!(p.get_credential().await.unwrap().key_id, "stale");
        clock.set(3609);
        assert_eq!(p.get_credential().await.unwrap().key_id, "stale");
        assert_eq!(source.fetches(), 1);
        clock.set(3610);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
        assert_eq!(source.fetches(), 2);
    }

    #[tokio::test]
    async fn narrowed_refresh_is_rejected() {
        let seed = vended("seed", S3, AccessLevel::ReadWrite, Some(3600));
        for narrowed in [
            // A sibling prefix, which a plain string prefix check would accept.
            vended(
                "x",
                "s3://bucket/table2",
                AccessLevel::ReadWrite,
                Some(7200),
            ),
            // A sub-prefix of the seed's scope.
            vended(
                "x",
                "s3://bucket/tab/part",
                AccessLevel::ReadWrite,
                Some(7200),
            ),
            // Downgraded access.
            vended("x", S3, AccessLevel::Read, Some(7200)),
        ] {
            let clock = TestClock::new();
            let source = ScriptedSource::new(vec![Ok(narrowed)]);
            let p = provider(&seed, source, &clock);
            clock.set(3600);
            let err = p.credential().await.unwrap_err();
            assert!(matches!(err, Error::RefreshNarrowed { .. }), "{err}");
        }
    }

    #[tokio::test]
    async fn wider_refresh_is_accepted() {
        let clock = TestClock::new();
        let source = ScriptedSource::new(vec![Ok(vended(
            "fresh",
            "s3://bucket",
            AccessLevel::ReadWrite,
            None,
        ))]);
        let seed = vended("seed", S3, AccessLevel::Read, Some(3600));
        let p = provider(&seed, source, &clock);
        clock.set(3600);
        assert_eq!(p.get_credential().await.unwrap().key_id, "fresh");
    }

    #[test]
    fn seed_of_the_wrong_kind_is_rejected() {
        let seed = VendedCredential::new(
            StorageCredentials::GcpBearer("ya29".into()),
            scope("gs://bucket/tab"),
            AccessLevel::Read,
        );
        let err =
            RefreshingCredentialProvider::<AwsCredential>::new(&seed, ScriptedSource::new(vec![]))
                .unwrap_err();
        assert!(matches!(err, Error::CredentialKind { .. }), "{err}");
    }

    #[test]
    fn debug_does_not_leak_the_credential() {
        let seed = vended("AKIDLEAK", S3, AccessLevel::Read, Some(3600));
        let p =
            RefreshingCredentialProvider::<AwsCredential>::new(&seed, ScriptedSource::new(vec![]))
                .unwrap();
        let debug = format!("{p:?}");
        assert!(!debug.contains("AKIDLEAK"), "{debug}");
        assert!(!debug.contains("secret"), "{debug}");
    }
}
