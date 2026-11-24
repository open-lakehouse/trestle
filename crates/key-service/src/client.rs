//! [`KeyServiceProvider`]: an [`olai_store::KeyProvider`] that wraps and unwraps over a key
//! server's Unix socket.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use hyper_util::rt::TokioIo;
use olai_store::{Error, KekId, KeyProvider, Result};
use tokio::net::UnixStream;
use tokio::sync::watch;
use tokio::time::Instant;
use tonic::Code;
use tonic::transport::{Channel, Endpoint};
use zeroize::Zeroizing;

use crate::v1::key_service_client::KeyServiceClient;
use crate::v1::{GetKeyRequest, KeyInfo, UnwrapRequest, WatchKeyRequest, WrapRequest};
use crate::{kek_id, parse_kek_id};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(5);

/// An [`olai_store::KeyProvider`] backed by a key server, over the consumer's Unix socket.
///
/// The provider holds no KEK. It sends each DEK to the server to be wrapped or unwrapped,
/// under the key named at construction.
///
/// - **KEK ids** are `<key>@v<version>` ([`kek_id`](crate::kek_id)), and any alias the
///   keyring maps to a version, so values sealed under an imported key's old id (`kek-1`, say)
///   keep opening.
/// - **The active version** comes from a cache that a `WatchKey` stream keeps current, so
///   [`active_kek_id`](KeyProvider::active_kek_id) does no I/O and a rotation reaches a running
///   process without a restart. While the server is away (it's being recreated, say), the
///   provider keeps the last versions it saw and reconnects with backoff.
/// - **Unwrapped DEKs are not cached** unless [`dek_cache`](KeyServiceProviderBuilder::dek_cache)
///   turns a bounded, expiring cache on. With the cache, values opened recently keep opening
///   while the server is away.
///
/// A call made while the server is unreachable retries for up to
/// [`call_timeout`](KeyServiceProviderBuilder::call_timeout), then fails. The background
/// watch stops when the provider is dropped.
///
/// ```no_run
/// # async fn example() -> olai_store::Result<()> {
/// use std::sync::Arc;
///
/// use olai_key_service::KeyServiceProvider;
/// use olai_store::EnvelopeEncryptor;
///
/// let keys = KeyServiceProvider::connect("/run/trestle-sds/sds.sock", "server-kek").await?;
/// let encryptor = EnvelopeEncryptor::new(Arc::new(keys));
/// let blob = encryptor.seal("row-id", b"value").await?;
/// # let _ = blob;
/// # Ok(())
/// # }
/// ```
pub struct KeyServiceProvider {
    inner: Arc<Inner>,
    info: watch::Receiver<Arc<KeyInfo>>,
    cache: Option<DekCache>,
}

struct Inner {
    socket: PathBuf,
    key: String,
    client: KeyServiceClient<Channel>,
    connect_timeout: Duration,
    call_timeout: Duration,
    initial_backoff: Duration,
    max_backoff: Duration,
}

/// Configures and connects a [`KeyServiceProvider`].
pub struct KeyServiceProviderBuilder {
    socket: PathBuf,
    key: String,
    connect_timeout: Duration,
    call_timeout: Duration,
    initial_backoff: Duration,
    max_backoff: Duration,
    dek_cache: Option<(Duration, usize)>,
}

impl KeyServiceProviderBuilder {
    /// How long [`connect`](Self::connect) keeps trying to reach the server (30 s by default).
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// How long a wrap or unwrap keeps retrying an unreachable server (10 s by default).
    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// The backoff between reconnects: `initial`, doubling up to `max` (100 ms and 5 s by
    /// default).
    pub fn backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_backoff = initial;
        self.max_backoff = max.max(initial);
        self
    }

    /// Keep unwrapped DEKs in memory for `ttl`, at most `max_entries` of them (oldest evicted
    /// first). Off by default.
    ///
    /// A cached DEK opens its values without a round trip, including while the server is
    /// away. The cost is plaintext DEKs held in process memory for up to `ttl` (wiped on
    /// eviction), and a revocation that reaches this process only once the entry expires,
    /// except for a raised minimum decrypt version, which drops the affected entries as soon
    /// as the watch sees it. Wraps are never cached.
    pub fn dek_cache(mut self, ttl: Duration, max_entries: usize) -> Self {
        self.dek_cache = (max_entries > 0 && !ttl.is_zero()).then_some((ttl, max_entries));
        self
    }

    /// Fetches the key's versions (retrying while the server is unreachable), and starts the
    /// watch that keeps them current. Must be called inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// The server stayed unreachable for the connect timeout, or refused the key (it isn't
    /// listed for this consumer's socket, or doesn't exist).
    pub async fn connect(self) -> Result<KeyServiceProvider> {
        let inner = Arc::new(Inner {
            client: KeyServiceClient::new(channel(&self.socket)),
            socket: self.socket,
            key: self.key,
            connect_timeout: self.connect_timeout,
            call_timeout: self.call_timeout,
            initial_backoff: self.initial_backoff,
            max_backoff: self.max_backoff,
        });
        let first = inner.get_key().await?;
        let (tx, info) = watch::channel(Arc::new(first));
        let cache = self.dek_cache.map(|(ttl, max)| DekCache::new(ttl, max));
        let watcher = Arc::clone(&inner);
        let purge = cache.as_ref().map(DekCache::handle);
        tokio::spawn(async move { watcher.watch(tx, purge).await });
        Ok(KeyServiceProvider { inner, info, cache })
    }
}

impl KeyServiceProvider {
    /// A builder for a provider of `key` from the server listening on `socket`.
    pub fn builder(
        socket: impl Into<PathBuf>,
        key: impl Into<String>,
    ) -> KeyServiceProviderBuilder {
        KeyServiceProviderBuilder {
            socket: socket.into(),
            key: key.into(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
            dek_cache: None,
        }
    }

    /// Connects a provider of `key` from the server listening on `socket`, with the defaults.
    ///
    /// # Errors
    ///
    /// See [`KeyServiceProviderBuilder::connect`].
    pub async fn connect(socket: impl Into<PathBuf>, key: impl Into<String>) -> Result<Self> {
        Self::builder(socket, key).connect().await
    }

    /// The socket the provider talks to.
    pub fn socket(&self) -> &Path {
        &self.inner.socket
    }

    /// The key's name.
    pub fn key(&self) -> &str {
        &self.inner.key
    }

    /// The key's versions, as the provider last saw them.
    pub fn key_info(&self) -> KeyInfo {
        (**self.info.borrow()).clone()
    }

    /// Waits until the key's versions change from what the provider last saw.
    ///
    /// # Errors
    ///
    /// The background watch has stopped (the runtime is shutting down).
    pub async fn changed(&mut self) -> Result<KeyInfo> {
        self.info
            .changed()
            .await
            .map_err(|_| Error::generic("key service watch stopped"))?;
        Ok(self.key_info())
    }

    /// The version and alias to send for `kek_id`: a canonical id's version, or an alias. An
    /// alias the cache doesn't know yet goes to the server with version 0, which resolves it.
    fn target(&self, kek_id: &str) -> (u32, String) {
        if let Some(version) = parse_kek_id(&self.inner.key, kek_id) {
            return (version, String::new());
        }
        let version = self.info.borrow().aliases.get(kek_id).copied().unwrap_or(0);
        (version, kek_id.to_string())
    }
}

impl fmt::Debug for KeyServiceProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let info = self.info.borrow();
        f.debug_struct("KeyServiceProvider")
            .field("socket", &self.inner.socket)
            .field("key", &self.inner.key)
            .field("active_version", &info.active_version)
            .field("min_decrypt_version", &info.min_decrypt_version)
            .field("dek_cache", &self.cache)
            .finish()
    }
}

#[async_trait]
impl KeyProvider for KeyServiceProvider {
    fn active_kek_id(&self) -> KekId {
        kek_id(&self.inner.key, self.info.borrow().active_version)
    }

    async fn wrap_dek(&self, dek: &[u8], kek_id: &str) -> Result<Vec<u8>> {
        let (version, alias) = self.target(kek_id);
        let mut response = self
            .inner
            .call(|mut client| {
                let request = WrapRequest {
                    key: self.inner.key.clone(),
                    dek: dek.to_vec(),
                    version,
                    alias: alias.clone(),
                };
                async move { client.wrap(request).await }
            })
            .await?;
        // The envelope records `kek_id`: the wrap must be under that version and no other.
        if version != 0 && response.version != version {
            return Err(Error::generic(format!(
                "key service wrapped under version {} of `{}`, not the requested {version}",
                response.version, self.inner.key
            )));
        }
        Ok(std::mem::take(&mut response.wrapped))
    }

    async fn unwrap_dek(&self, wrapped: &[u8], kek_id: &str) -> Result<Zeroizing<Vec<u8>>> {
        if let Some(dek) = self.cache.as_ref().and_then(|c| c.get(kek_id, wrapped)) {
            return Ok(dek);
        }
        let (version, alias) = self.target(kek_id);
        let mut response = self
            .inner
            .call(|mut client| {
                let request = UnwrapRequest {
                    key: self.inner.key.clone(),
                    version,
                    wrapped: wrapped.to_vec(),
                    alias: alias.clone(),
                };
                async move { client.unwrap(request).await }
            })
            .await?;
        let dek = Zeroizing::new(std::mem::take(&mut response.dek));
        if let Some(cache) = &self.cache {
            let version = if version == 0 {
                self.info.borrow().aliases.get(kek_id).copied().unwrap_or(0)
            } else {
                version
            };
            cache.insert(kek_id, wrapped, version, &dek);
        }
        Ok(dek)
    }
}

/// Connects lazily to the Unix socket at `path`, and again after the connection breaks.
fn channel(path: &Path) -> Channel {
    let path = path.to_path_buf();
    // The URI is required but unused: the connector ignores it and dials the socket.
    Endpoint::from_static("http://keys.invalid").connect_with_connector_lazy(tower::service_fn(
        move |_| {
            let path = path.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        },
    ))
}

/// Whether a status means the server is away (worth retrying), not a final answer.
fn unreachable(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        Code::Unavailable | Code::Unknown | Code::Cancelled
    )
}

struct Backoff {
    next: Duration,
    max: Duration,
}

impl Backoff {
    fn new(inner: &Inner) -> Self {
        Self {
            next: inner.initial_backoff,
            max: inner.max_backoff,
        }
    }

    fn next(&mut self) -> Duration {
        let wait = self.next;
        self.next = (self.next * 2).min(self.max);
        wait
    }
}

impl Inner {
    /// Runs `call`, retrying with backoff while the server is unreachable, for up to `within`.
    async fn retry<T, F, Fut>(&self, within: Duration, mut call: F) -> Result<T>
    where
        F: FnMut(KeyServiceClient<Channel>) -> Fut,
        Fut: Future<Output = std::result::Result<tonic::Response<T>, tonic::Status>>,
    {
        let deadline = Instant::now() + within;
        let mut backoff = Backoff::new(self);
        loop {
            match call(self.client.clone()).await {
                Ok(response) => return Ok(response.into_inner()),
                Err(status) if unreachable(&status) => {
                    let wait = backoff.next();
                    if Instant::now() + wait > deadline {
                        return Err(Error::generic(format!(
                            "key service at {} is unreachable: {}",
                            self.socket.display(),
                            status.message()
                        )));
                    }
                    tracing::debug!(
                        socket = %self.socket.display(),
                        key = %self.key,
                        reason = %status.message(),
                        "key service unreachable; retrying",
                    );
                    tokio::time::sleep(wait).await;
                }
                Err(status) => return Err(self.refused(&status)),
            }
        }
    }

    async fn call<T, F, Fut>(&self, call: F) -> Result<T>
    where
        F: FnMut(KeyServiceClient<Channel>) -> Fut,
        Fut: Future<Output = std::result::Result<tonic::Response<T>, tonic::Status>>,
    {
        self.retry(self.call_timeout, call).await
    }

    async fn get_key(&self) -> Result<KeyInfo> {
        let key = self.key.clone();
        self.retry(self.connect_timeout, |mut client| {
            let request = GetKeyRequest { key: key.clone() };
            async move { client.get_key(request).await }
        })
        .await
    }

    /// A final answer from the server. Its messages name keys and versions, never material.
    fn refused(&self, status: &tonic::Status) -> Error {
        let message = format!(
            "key service at {} refused key `{}`: {}",
            self.socket.display(),
            self.key,
            status.message()
        );
        match status.code() {
            Code::InvalidArgument | Code::FailedPrecondition => Error::invalid_argument(message),
            _ => Error::generic(message),
        }
    }

    /// Keeps `tx` current until every receiver is gone: one `WatchKey` stream at a time,
    /// reconnecting with backoff whenever a stream ends. `tx` always holds the last versions
    /// seen.
    async fn watch(&self, tx: watch::Sender<Arc<KeyInfo>>, purge: Option<CacheHandle>) {
        let mut backoff = Backoff::new(self);
        loop {
            let mut healthy = false;
            let ended = tokio::select! {
                () = tx.closed() => return,
                ended = self.stream(&tx, purge.as_ref(), &mut healthy) => ended,
            };
            if healthy {
                backoff = Backoff::new(self);
            }
            tracing::debug!(
                socket = %self.socket.display(),
                key = %self.key,
                reason = %ended,
                "key watch ended; keeping the last versions and reconnecting",
            );
            tokio::select! {
                () = tx.closed() => return,
                () = tokio::time::sleep(backoff.next()) => {}
            }
        }
    }

    /// One `WatchKey` stream, until it ends. Returns why it ended.
    async fn stream(
        &self,
        tx: &watch::Sender<Arc<KeyInfo>>,
        purge: Option<&CacheHandle>,
        healthy: &mut bool,
    ) -> String {
        let request = WatchKeyRequest {
            key: self.key.clone(),
        };
        let mut inbound = match self.client.clone().watch_key(request).await {
            Ok(response) => response.into_inner(),
            Err(status) => return status.message().to_string(),
        };
        while let Some(message) = inbound.next().await {
            let info = match message {
                Ok(info) => info,
                Err(status) => return status.message().to_string(),
            };
            *healthy = true;
            if info != **tx.borrow() {
                tracing::info!(
                    key = %self.key,
                    active_version = info.active_version,
                    min_decrypt_version = info.min_decrypt_version,
                    "key versions changed",
                );
                if let Some(purge) = purge {
                    drop_below(purge, info.min_decrypt_version);
                }
                tx.send_replace(Arc::new(info));
            }
        }
        "the server closed the stream".into()
    }
}

/// Unwrapped DEKs by KEK id and wrapped bytes, each kept for a while.
struct DekCache {
    ttl: Duration,
    max_entries: usize,
    entries: CacheHandle,
}

type CacheHandle = Arc<Mutex<CacheEntries>>;

#[derive(Default)]
struct CacheEntries {
    map: HashMap<(String, Vec<u8>), CachedDek>,
}

struct CachedDek {
    dek: Zeroizing<Vec<u8>>,
    /// The version it was wrapped under; 0 for an alias the cache didn't know yet.
    version: u32,
    inserted: Instant,
}

impl DekCache {
    fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            entries: Arc::default(),
        }
    }

    fn handle(&self) -> CacheHandle {
        Arc::clone(&self.entries)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheEntries> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn get(&self, kek_id: &str, wrapped: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        let mut entries = self.lock();
        let key = (kek_id.to_string(), wrapped.to_vec());
        let entry = entries.map.get(&key)?;
        if entry.inserted.elapsed() < self.ttl {
            return Some(entry.dek.clone());
        }
        entries.map.remove(&key);
        None
    }

    fn insert(&self, kek_id: &str, wrapped: &[u8], version: u32, dek: &Zeroizing<Vec<u8>>) {
        let mut entries = self.lock();
        if entries.map.len() >= self.max_entries {
            let ttl = self.ttl;
            entries.map.retain(|_, e| e.inserted.elapsed() < ttl);
        }
        while entries.map.len() >= self.max_entries {
            let oldest = entries
                .map
                .iter()
                .min_by_key(|(_, e)| e.inserted)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => entries.map.remove(&key),
                None => break,
            };
        }
        entries.map.insert(
            (kek_id.to_string(), wrapped.to_vec()),
            CachedDek {
                dek: dek.clone(),
                version,
                inserted: Instant::now(),
            },
        );
    }
}

impl CacheEntries {
    fn len(&self) -> usize {
        self.map.len()
    }
}

/// Drops every entry for a version the server no longer unwraps, and every entry of an alias
/// whose version wasn't known when it was cached.
fn drop_below(entries: &CacheHandle, min_decrypt: u32) {
    entries
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .map
        .retain(|_, e| e.version != 0 && e.version >= min_decrypt);
}

impl fmt::Debug for DekCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DekCache")
            .field("ttl", &self.ttl)
            .field("max_entries", &self.max_entries)
            .field("entries", &self.lock().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dek(byte: u8) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(vec![byte; 32])
    }

    #[tokio::test(start_paused = true)]
    async fn cache_entries_expire() {
        let cache = DekCache::new(Duration::from_secs(60), 8);
        cache.insert("k@v1", b"w", 1, &dek(1));
        assert_eq!(
            cache.get("k@v1", b"w").unwrap().as_slice(),
            dek(1).as_slice()
        );
        assert!(cache.get("k@v2", b"w").is_none());
        assert!(cache.get("k@v1", b"other").is_none());
        tokio::time::advance(Duration::from_secs(61)).await;
        assert!(cache.get("k@v1", b"w").is_none());
        assert_eq!(cache.lock().len(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn cache_evicts_the_oldest_at_the_cap() {
        let cache = DekCache::new(Duration::from_secs(60), 2);
        cache.insert("k@v1", b"a", 1, &dek(1));
        tokio::time::advance(Duration::from_secs(1)).await;
        cache.insert("k@v1", b"b", 1, &dek(2));
        tokio::time::advance(Duration::from_secs(1)).await;
        cache.insert("k@v1", b"c", 1, &dek(3));
        assert_eq!(cache.lock().len(), 2);
        assert!(cache.get("k@v1", b"a").is_none());
        assert!(cache.get("k@v1", b"b").is_some());
        assert!(cache.get("k@v1", b"c").is_some());
    }

    #[tokio::test]
    async fn raising_min_decrypt_drops_older_entries() {
        let cache = DekCache::new(Duration::from_secs(60), 8);
        cache.insert("k@v1", b"a", 1, &dek(1));
        cache.insert("k@v2", b"b", 2, &dek(2));
        cache.insert("kek-1", b"c", 0, &dek(3));
        drop_below(&cache.handle(), 2);
        assert!(cache.get("k@v1", b"a").is_none());
        assert!(cache.get("kek-1", b"c").is_none());
        assert!(cache.get("k@v2", b"b").is_some());
    }

    #[test]
    fn debug_shows_no_deks() {
        let cache = DekCache::new(Duration::from_secs(60), 8);
        cache.insert("k@v1", b"a", 1, &Zeroizing::new(b"hunter2-dek".to_vec()));
        let rendered = format!("{cache:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("entries: 1"), "{rendered}");
    }

    #[test]
    fn builder_disables_a_degenerate_cache() {
        let builder = KeyServiceProvider::builder("/s", "k").dek_cache(Duration::ZERO, 8);
        assert!(builder.dek_cache.is_none());
        let builder = KeyServiceProvider::builder("/s", "k").dek_cache(Duration::from_secs(1), 0);
        assert!(builder.dek_cache.is_none());
    }
}
