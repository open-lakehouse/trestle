//! Caching minted credentials, and vending through a resolver.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use olai_storage_credentials::{AccessLevel, StorageScope, VendedCredential};

use crate::error::Result;
use crate::mint::{CredentialMinter, MintRequest, minter_for};
use crate::registered::RegisteredCredential;
use crate::resolve::CredentialResolver;

/// How many credentials a [`CachingMinter`] holds unless configured otherwise.
pub const DEFAULT_CAPACITY: usize = 1024;

/// How long before expiry a cached credential stops being handed out.
pub const DEFAULT_MARGIN: Duration = Duration::from_secs(300);

/// How long a credential without an expiry (local storage) is cached.
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(3600);

/// The current time. Injectable so expiry is testable without waiting.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// What a cached credential was minted for: the whole request, structurally, so two
/// requests share an entry only if they would mint the same grant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    scope: StorageScope,
    access: AccessLevel,
    ttl: Option<Duration>,
    session_name: Option<String>,
}

impl From<&MintRequest> for Key {
    fn from(request: &MintRequest) -> Self {
        Self {
            scope: request.scope.clone(),
            access: request.access,
            ttl: request.ttl,
            session_name: request.session_name.clone(),
        }
    }
}

struct Entry {
    credential: VendedCredential,
    /// When the entry stops being served.
    stale_at: DateTime<Utc>,
}

/// A [`CredentialMinter`] that reuses credentials minted for the same request.
///
/// - Entries are keyed by the whole request (canonical scope, access, lifetime, session
///   name), never by a hash of it, so distinct requests cannot collide.
/// - An entry is served for the first half of its credential's lifetime, and never
///   within [`margin`](Self::with_margin) of its expiry, so a hit always has at least
///   half the lifetime a fresh mint would; one without an expiry is served for at most
///   [`max_age`](Self::with_max_age).
/// - The cache holds at most [`capacity`](Self::with_capacity) entries: when full, stale
///   entries go first, then those closest to going stale.
/// - One cache serves one minter, so one principal: wrap each principal's minter in its
///   own (as [`Vendor`] does).
///
/// Concurrent misses for the same request may each mint; the last one is kept.
pub struct CachingMinter {
    inner: Arc<dyn CredentialMinter>,
    capacity: usize,
    margin: TimeDelta,
    max_age: TimeDelta,
    clock: Clock,
    entries: Mutex<HashMap<Key, Entry>>,
}

impl CachingMinter {
    /// Caches what `inner` mints, with the default capacity, margin and maximum age.
    pub fn new(inner: Arc<dyn CredentialMinter>) -> Self {
        Self {
            inner,
            capacity: DEFAULT_CAPACITY,
            margin: delta(DEFAULT_MARGIN),
            max_age: delta(DEFAULT_MAX_AGE),
            clock: Arc::new(Utc::now),
            entries: Mutex::default(),
        }
    }

    /// Holds at most `capacity` credentials (at least one).
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self
    }

    /// Stops serving a credential `margin` before it expires.
    pub fn with_margin(mut self, margin: Duration) -> Self {
        self.margin = delta(margin);
        self
    }

    /// Serves a credential without an expiry for at most `max_age`.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = delta(max_age);
        self
    }

    /// Reads the time from `clock` instead of the system clock.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The number of cached credentials, stale ones included.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entries; a panic elsewhere never leaves them half-updated, so a poisoned lock
    /// is still safe to use.
    fn entries(&self) -> MutexGuard<'_, HashMap<Key, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// When a credential minted at `now` stops being served, or `None` if it is too
    /// short-lived to serve again.
    fn stale_at(&self, credential: &VendedCredential, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let stale_at = match credential.expires_at {
            Some(expiry) => {
                let half_life = now.checked_add_signed((expiry - now) / 2)?;
                expiry.checked_sub_signed(self.margin)?.min(half_life)
            }
            None => now
                .checked_add_signed(self.max_age)
                .unwrap_or(DateTime::<Utc>::MAX_UTC),
        };
        (stale_at > now).then_some(stale_at)
    }

    fn insert(&self, key: Key, credential: VendedCredential, now: DateTime<Utc>) {
        let Some(stale_at) = self.stale_at(&credential, now) else {
            return;
        };
        let mut entries = self.entries();
        if entries.len() >= self.capacity && !entries.contains_key(&key) {
            entries.retain(|_, entry| entry.stale_at > now);
            if entries.len() >= self.capacity
                && let Some(soonest) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.stale_at)
                    .map(|(key, _)| key.clone())
            {
                entries.remove(&soonest);
            }
        }
        entries.insert(
            key,
            Entry {
                credential,
                stale_at,
            },
        );
    }
}

fn delta(duration: Duration) -> TimeDelta {
    TimeDelta::from_std(duration).unwrap_or(TimeDelta::MAX)
}

#[async_trait]
impl CredentialMinter for CachingMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        let key = Key::from(request);
        let now = (self.clock)();
        if let Some(entry) = self.entries().get(&key)
            && entry.stale_at > now
        {
            return Ok(entry.credential.clone());
        }
        let credential = self.inner.mint(request).await?;
        self.insert(key, credential.clone(), now);
        Ok(credential)
    }
}

impl fmt::Debug for CachingMinter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachingMinter")
            .field("inner", &self.inner)
            .field("capacity", &self.capacity)
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

/// Builds the minter for a registered credential.
type MakeMinter = dyn Fn(&RegisteredCredential) -> Result<Arc<dyn CredentialMinter>> + Send + Sync;

/// Vends for any registered location: resolves the governing credential, and mints
/// through a [`CachingMinter`] per registered credential.
///
/// This is the whole server side of a `temporary-*-credentials` endpoint once the
/// caller is authorized: a location in, a downscoped credential out.
///
/// Minters are kept per registered credential, secrets included: when a credential is
/// rotated or removed, [`forget`](Self::forget) the old one so its secrets and cached
/// credentials are dropped.
pub struct Vendor<R> {
    resolver: R,
    minters: Mutex<HashMap<RegisteredCredential, Arc<CachingMinter>>>,
    make: Box<MakeMinter>,
    capacity: usize,
}

impl<R: CredentialResolver> Vendor<R> {
    /// Vends through `resolver`, minting with [`minter_for`].
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            minters: Mutex::default(),
            make: Box::new(minter_for),
            capacity: DEFAULT_CAPACITY,
        }
    }

    /// Builds each registered credential's minter with `make` instead, for example to
    /// set endpoints or audit fields.
    pub fn with_minters(
        mut self,
        make: impl Fn(&RegisteredCredential) -> Result<Arc<dyn CredentialMinter>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.make = Box::new(make);
        self
    }

    /// The capacity of each credential's cache.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Mints a credential for `request`, from the credential governing its scope.
    pub async fn vend(&self, request: &MintRequest) -> Result<VendedCredential> {
        let credential = self.resolver.resolve(request.scope.location()).await?;
        let minter = self.minter(&credential)?;
        minter.mint(request).await
    }

    /// Drops the minter for `credential` and everything it cached. Returns whether there
    /// was one.
    pub fn forget(&self, credential: &RegisteredCredential) -> bool {
        self.minters().remove(credential).is_some()
    }

    /// Drops every minter and everything they cached.
    pub fn clear(&self) {
        self.minters().clear();
    }

    fn minter(&self, credential: &RegisteredCredential) -> Result<Arc<CachingMinter>> {
        if let Some(minter) = self.minters().get(credential) {
            return Ok(Arc::clone(minter));
        }
        // Built outside the lock: building may read files or fail, and must not hold up
        // (or, by panicking, poison) vending for other credentials. Of concurrent builds
        // for the same credential, the first inserted is kept.
        let minter =
            Arc::new(CachingMinter::new((self.make)(credential)?).with_capacity(self.capacity));
        Ok(Arc::clone(
            self.minters().entry(credential.clone()).or_insert(minter),
        ))
    }
}

impl<R> Vendor<R> {
    fn minters(&self) -> MutexGuard<'_, HashMap<RegisteredCredential, Arc<CachingMinter>>> {
        self.minters.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<R> fmt::Debug for Vendor<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vendor")
            .field("minters", &self.minters().len())
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use olai_storage_credentials::StorageCredentials;

    use super::*;
    use crate::registered::{AwsIamRole, AzureStorageKey};
    use crate::resolve::ExternalLocations;

    /// Mints a fresh credential per call, expiring `lifetime` after `now`.
    #[derive(Debug)]
    struct Counting {
        mints: AtomicUsize,
        lifetime: Option<TimeDelta>,
        now: DateTime<Utc>,
    }

    impl Counting {
        fn new(lifetime: Option<i64>) -> Arc<Self> {
            Arc::new(Self {
                mints: AtomicUsize::new(0),
                lifetime: lifetime.map(TimeDelta::seconds),
                now: t(0),
            })
        }
        fn mints(&self) -> usize {
            self.mints.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl CredentialMinter for Counting {
        async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
            let n = self.mints.fetch_add(1, Ordering::SeqCst);
            Ok(VendedCredential::new(
                StorageCredentials::aws(format!("key-{n}"), "secret", None),
                request.scope.clone(),
                request.access,
            )
            // Each credential lives 10 s longer than the last, so they go stale in order.
            .with_expiry(
                self.lifetime
                    .map(|l| self.now + l + TimeDelta::seconds(10 * n as i64)),
            ))
        }
    }

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap()
    }

    /// A clock tests move by hand.
    fn clock() -> (Arc<Mutex<DateTime<Utc>>>, Clock) {
        let now = Arc::new(Mutex::new(t(0)));
        let read = Arc::clone(&now);
        (now, Arc::new(move || *read.lock().unwrap()))
    }

    fn request(url: &str) -> MintRequest {
        MintRequest::new(url.parse().unwrap(), AccessLevel::Read)
    }

    fn key(credential: &VendedCredential) -> String {
        match &credential.credentials {
            StorageCredentials::Aws { key_id, .. } => key_id.clone(),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_same_request_is_served_from_the_cache() {
        let inner = Counting::new(Some(3600));
        let (_, clock) = clock();
        let cache = CachingMinter::new(inner.clone()).with_clock(clock);
        let a = cache.mint(&request("s3://b/tab")).await.unwrap();
        let b = cache.mint(&request("s3://b/tab")).await.unwrap();
        assert_eq!(key(&a), key(&b));
        assert_eq!(inner.mints(), 1);
    }

    #[tokio::test]
    async fn distinct_requests_do_not_share_entries() {
        let inner = Counting::new(Some(3600));
        let cache = CachingMinter::new(inner.clone());
        let base = request("s3://b/tab");
        for request in [
            base.clone(),
            request("s3://b/table2"),
            MintRequest::new(base.scope.clone(), AccessLevel::ReadWrite),
            base.clone().with_ttl(Duration::from_secs(900)),
            base.clone().with_session_name("alice"),
        ] {
            cache.mint(&request).await.unwrap();
        }
        assert_eq!(inner.mints(), 5);
        // Another spelling of the same location is the same request.
        cache
            .mint(&request("https://b.s3.us-east-1.amazonaws.com/tab"))
            .await
            .unwrap();
        assert_eq!(inner.mints(), 5);
    }

    #[tokio::test]
    async fn credentials_are_served_for_half_their_lifetime() {
        let inner = Counting::new(Some(3600));
        let (now, clock) = clock();
        let cache = CachingMinter::new(inner.clone()).with_clock(clock);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        *now.lock().unwrap() = t(1799);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        assert_eq!(inner.mints(), 1);
        // A caller asking for an hour never gets a credential with under half an hour left.
        *now.lock().unwrap() = t(1800);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        assert_eq!(inner.mints(), 2);
    }

    #[tokio::test]
    async fn credentials_near_expiry_are_minted_again() {
        let inner = Counting::new(Some(600));
        let (now, clock) = clock();
        let cache = CachingMinter::new(inner.clone())
            .with_clock(clock)
            .with_margin(Duration::from_secs(400));
        cache.mint(&request("s3://b/tab")).await.unwrap();
        *now.lock().unwrap() = t(199);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        assert_eq!(inner.mints(), 1);
        *now.lock().unwrap() = t(200);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        assert_eq!(inner.mints(), 2);
    }

    #[tokio::test]
    async fn extreme_margins_and_ages_do_not_panic() {
        let inner = Counting::new(Some(3600));
        let cache = CachingMinter::new(inner.clone()).with_margin(Duration::MAX);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        assert!(cache.is_empty());

        // Cached forever.
        let inner = Counting::new(None);
        let (now, clock) = clock();
        let cache = CachingMinter::new(inner.clone())
            .with_clock(clock)
            .with_max_age(Duration::MAX);
        cache.mint(&request("file:///data/tab")).await.unwrap();
        *now.lock().unwrap() = t(10 * 365 * 24 * 3600);
        cache.mint(&request("file:///data/tab")).await.unwrap();
        assert_eq!(inner.mints(), 1);
    }

    #[tokio::test]
    async fn credentials_without_expiry_age_out() {
        let inner = Counting::new(None);
        let (now, clock) = clock();
        let cache = CachingMinter::new(inner.clone())
            .with_clock(clock)
            .with_max_age(Duration::from_secs(60));
        cache.mint(&request("file:///data/tab")).await.unwrap();
        *now.lock().unwrap() = t(59);
        cache.mint(&request("file:///data/tab")).await.unwrap();
        assert_eq!(inner.mints(), 1);
        *now.lock().unwrap() = t(60);
        cache.mint(&request("file:///data/tab")).await.unwrap();
        assert_eq!(inner.mints(), 2);
    }

    #[tokio::test]
    async fn short_lived_credentials_are_not_cached() {
        let inner = Counting::new(Some(60));
        let (_, clock) = clock();
        let cache = CachingMinter::new(inner.clone()).with_clock(clock);
        cache.mint(&request("s3://b/tab")).await.unwrap();
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn capacity_evicts_the_soonest_stale_entry() {
        let inner = Counting::new(Some(3600));
        let (_, clock) = clock();
        let cache = CachingMinter::new(inner.clone())
            .with_clock(clock)
            .with_capacity(2);
        for n in 1..=3 {
            cache.mint(&request(&format!("s3://b/{n}"))).await.unwrap();
        }
        assert_eq!(cache.len(), 2);
        assert_eq!(inner.mints(), 3);
        // The first goes stale soonest, so it was evicted; the others are still cached.
        cache.mint(&request("s3://b/3")).await.unwrap();
        cache.mint(&request("s3://b/2")).await.unwrap();
        assert_eq!(inner.mints(), 3);
        cache.mint(&request("s3://b/1")).await.unwrap();
        assert_eq!(inner.mints(), 4);
    }

    #[tokio::test]
    async fn vendor_mints_with_the_governing_credential() {
        let locations = ExternalLocations::new()
            .with_location(
                "s3://bucket".parse().unwrap(),
                RegisteredCredential::AwsIamRole(AwsIamRole {
                    role_arn: "arn:aws:iam::1:role/bucket".into(),
                    ..AwsIamRole::default()
                }),
            )
            .with_location(
                "abfss://ctr@acct.dfs.core.windows.net".parse().unwrap(),
                RegisteredCredential::AzureStorageKey(AzureStorageKey {
                    account_name: "acct".into(),
                    account_key: "a2V5".into(),
                }),
            );
        let built = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&built);
        let inner = Counting::new(Some(3600));
        let minter: Arc<dyn CredentialMinter> = inner.clone();
        let vendor = Vendor::new(locations).with_minters(move |credential| {
            seen.lock().unwrap().push(credential.kind());
            Ok(Arc::clone(&minter))
        });

        vendor.vend(&request("s3://bucket/tab")).await.unwrap();
        vendor.vend(&request("s3://bucket/tab")).await.unwrap();
        vendor
            .vend(&request("abfss://ctr@acct.dfs.core.windows.net/t"))
            .await
            .unwrap();
        // One minter per registered credential, each with its own cache.
        assert_eq!(
            *built.lock().unwrap(),
            ["aws_iam_role", "azure_storage_key"]
        );
        assert_eq!(inner.mints(), 2);

        let err = vendor.vend(&request("s3://other/t")).await.unwrap_err();
        assert!(matches!(err, crate::Error::NotFound { .. }), "{err}");
    }

    fn account_key() -> RegisteredCredential {
        RegisteredCredential::AzureStorageKey(AzureStorageKey {
            account_name: "acct".into(),
            account_key: "a2V5".into(),
        })
    }

    #[tokio::test]
    async fn vendor_mints_with_the_default_minters() {
        let vendor = Vendor::new(ExternalLocations::new().with_location(
            "abfss://ctr@acct.dfs.core.windows.net".parse().unwrap(),
            account_key(),
        ));
        let vended = vendor
            .vend(&request("abfss://ctr@acct.dfs.core.windows.net/t"))
            .await
            .unwrap();
        match &vended.credentials {
            StorageCredentials::AzureSas(pairs) => {
                assert!(
                    pairs.iter().any(|(k, v)| k == "sr" && v == "d"),
                    "{pairs:?}"
                )
            }
            other => panic!("expected a SAS, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forgotten_credentials_drop_their_minters() {
        let vendor = Vendor::new(ExternalLocations::new().with_location(
            "abfss://ctr@acct.dfs.core.windows.net".parse().unwrap(),
            account_key(),
        ));
        vendor
            .vend(&request("abfss://ctr@acct.dfs.core.windows.net/t"))
            .await
            .unwrap();
        assert_eq!(vendor.minters().len(), 1);
        assert!(vendor.forget(&account_key()));
        assert!(!vendor.forget(&account_key()));
        assert!(vendor.minters().is_empty());

        vendor
            .vend(&request("abfss://ctr@acct.dfs.core.windows.net/t"))
            .await
            .unwrap();
        vendor.clear();
        assert!(vendor.minters().is_empty());
    }

    #[tokio::test]
    async fn a_panicking_minter_build_does_not_break_the_vendor() {
        let locations = ExternalLocations::new()
            .with_location(
                "s3://bucket".parse().unwrap(),
                RegisteredCredential::AwsIamRole(AwsIamRole {
                    role_arn: "arn:aws:iam::1:role/bucket".into(),
                    ..AwsIamRole::default()
                }),
            )
            .with_location(
                "abfss://ctr@acct.dfs.core.windows.net".parse().unwrap(),
                account_key(),
            );
        let vendor = Arc::new(
            Vendor::new(locations).with_minters(|credential| match credential {
                RegisteredCredential::AwsIamRole(_) => panic!("cannot build"),
                other => minter_for(other),
            }),
        );
        let panicking = Arc::clone(&vendor);
        let handle = tokio::spawn(async move { panicking.vend(&request("s3://bucket/t")).await });
        assert!(handle.await.unwrap_err().is_panic());
        vendor
            .vend(&request("abfss://ctr@acct.dfs.core.windows.net/t"))
            .await
            .unwrap();
        assert!(format!("{vendor:?}").contains("minters: 1"));
    }
}
