//! Trusted vending for Neon object storage (feature `neon-trusted-vending`).
//!
//! Neon object storage has no STS and no bucket- or prefix-scoped keys, so a credential
//! cannot be downscoped to a table. Neon's control plane can mint **branch-scoped**,
//! **expiring** keys with `storage:read` or `storage:read` + `storage:write`, and
//! [`NeonTrustedMinter`] vends those.
//!
//! The minter is *trusted*: whoever receives a key reaches **every bucket on the branch,
//! and on its descendant branches**, at the granted access level, until the key
//! expires. The request's scope only labels the credential. This is an explicit, opt-in
//! exception to the rule that vending servers never hand out unscoped keys (see the
//! [credential vending design record]). A host that enables it must authorize every
//! caller for that reach before calling [`Vendor::vend`](crate::Vendor::vend) or
//! [`mint`](crate::CredentialMinter::mint).
//!
//! - **Expiry is the control.** Neon enforces `expires_at` to the second. Revocation is
//!   eventually consistent (S3 may accept a revoked key for some seconds), so it does
//!   not bound a key's life.
//! - **One key per access level.** Keys are not scope-limited, so the minter caches one
//!   per access level, not per scope, and reuses it until half its lifetime has
//!   passed. That leaves about two live keys per level per lifetime.
//! - **Garbage collection.** Neon keeps listing expired keys until they are revoked. After
//!   a mint, at most once per [`gc_interval`](NeonMinterOptions::with_gc_interval), the
//!   minter revokes the branch's expired `olai-vend-*` keys. This is best effort and
//!   never fails a mint.
//! - **The API key is the host's.** It is passed to [`NeonTrustedMinter::new`] from host
//!   configuration, and is never part of the [`NeonStorage`] registered credential.
//!
//! ```no_run
//! # fn run() -> olai_credential_vending::Result<()> {
//! use std::sync::Arc;
//!
//! use olai_credential_vending::neon::{NeonMinterOptions, NeonTrustedMinter};
//! use olai_credential_vending::{
//!     CredentialMinter, ExternalLocations, RegisteredCredential, Vendor, minter_for,
//! };
//!
//! let api_key = std::env::var("NEON_API_KEY").expect("host configuration");
//! let vendor = Vendor::new(ExternalLocations::new()).with_minters(move |credential| {
//!     match credential {
//!         RegisteredCredential::NeonStorage(branch) => Ok(Arc::new(NeonTrustedMinter::new(
//!             branch,
//!             api_key.clone(),
//!             NeonMinterOptions::default(),
//!         )?) as Arc<dyn CredentialMinter>),
//!         other => minter_for(other),
//!     }
//! });
//! # let _ = vendor;
//! # Ok(()) }
//! ```
//!
//! [credential vending design record]: https://github.com/open-lakehouse/trestle/blob/main/docs/credential-vending.md

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use olai_http::CloudClient;
use olai_http::neon::{
    BranchStorage, CreateCredential, DEFAULT_API_URL, NeonClient, SCOPE_STORAGE_READ,
    SCOPE_STORAGE_WRITE,
};
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageProvider, StoreHints, VendedCredential,
};
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use url::Url;

use crate::cache::Clock;
use crate::error::{Error, Result};
use crate::mint::{CredentialMinter, MintRequest};
use crate::registered::NeonStorage;

/// The lifetime of a key when the request names none.
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// The shortest lifetime the minter issues.
pub const MIN_TTL: Duration = Duration::from_secs(15 * 60);

/// The longest lifetime the minter issues.
pub const MAX_TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// How often, at most, expired keys are garbage-collected unless configured otherwise.
pub const DEFAULT_GC_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// The name prefix of every key this crate mints. Garbage collection only revokes
/// expired keys with this prefix.
pub const NAME_PREFIX: &str = "olai-vend-";

/// The most expired keys one garbage-collection pass revokes.
const GC_BATCH: usize = 50;

const KIND: &str = "neon_storage";

/// Options for a [`NeonTrustedMinter`].
#[derive(Clone)]
#[non_exhaustive]
pub struct NeonMinterOptions {
    /// The Neon API; [`DEFAULT_API_URL`] when unset.
    pub api_url: Option<Url>,
    /// The lifetime of a key when the request names none (clamped to
    /// [`MIN_TTL`]..=[`MAX_TTL`]).
    pub default_ttl: Duration,
    /// How often, at most, expired keys are garbage-collected.
    pub gc_interval: Duration,
    /// The branch's endpoint, region and addressing, to skip asking the control plane.
    pub branch_storage: Option<BranchStorage>,
    /// The current time. Injectable so expiry is testable without waiting.
    pub clock: Clock,
}

impl Default for NeonMinterOptions {
    fn default() -> Self {
        Self {
            api_url: None,
            default_ttl: DEFAULT_TTL,
            gc_interval: DEFAULT_GC_INTERVAL,
            branch_storage: None,
            clock: Arc::new(Utc::now),
        }
    }
}

impl NeonMinterOptions {
    /// Calls the Neon API at `api_url`.
    pub fn with_api_url(mut self, api_url: Url) -> Self {
        self.api_url = Some(api_url);
        self
    }

    /// Mints keys living `ttl` when the request names no lifetime.
    pub fn with_default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl;
        self
    }

    /// Garbage-collects expired keys at most once per `interval`.
    pub fn with_gc_interval(mut self, interval: Duration) -> Self {
        self.gc_interval = interval;
        self
    }

    /// Uses `storage` as the branch's endpoint, region and addressing.
    pub fn with_branch_storage(mut self, storage: BranchStorage) -> Self {
        self.branch_storage = Some(storage);
        self
    }

    /// Reads the time from `clock` instead of the system clock.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }
}

impl fmt::Debug for NeonMinterOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeonMinterOptions")
            .field("api_url", &self.api_url.as_ref().map(Url::as_str))
            .field("default_ttl", &self.default_ttl)
            .field("gc_interval", &self.gc_interval)
            .field("branch_storage", &self.branch_storage)
            .finish_non_exhaustive()
    }
}

/// A key minted for one access level.
struct Cached {
    key_id: String,
    secret: String,
    expires_at: DateTime<Utc>,
    /// When it stops being handed out: half its lifetime after it was minted.
    stale_at: DateTime<Utc>,
}

/// Mints branch-wide, expiring S3 keys for a Neon branch's object storage.
///
/// **Trusted:** a key is not limited to the request's scope. See the
/// [module documentation](self) for the threat model, and authorize callers before
/// minting.
///
/// Each mint returns the cached key for the request's access level while it has more
/// than half its lifetime left, and otherwise creates one: scopes `storage:read`
/// (read) or `storage:read` + `storage:write` (read-write), `expires_at` now plus the
/// request's lifetime (or the default) clamped to 15 minutes – 12 hours, named
/// `olai-vend-<access>-<unix seconds>`. Concurrent mints for one access level create
/// at most one key. The credential is [`StorageCredentials::Aws`] without a session
/// token, for the request's scope, with the branch's endpoint, region and path-style
/// addressing as hints.
pub struct NeonTrustedMinter {
    client: NeonClient,
    project: String,
    branch: String,
    default_ttl: Duration,
    gc_interval: TimeDelta,
    clock: Clock,
    given_storage: Option<BranchStorage>,
    hints: OnceCell<StoreHints>,
    read: AsyncMutex<Option<Cached>>,
    read_write: AsyncMutex<Option<Cached>>,
    last_gc: Mutex<Option<DateTime<Utc>>>,
}

impl NeonTrustedMinter {
    /// A minter for `branch`'s storage, calling the control plane with `api_key` (from
    /// the host's configuration). Sends no request until the first mint.
    pub fn new(
        branch: &NeonStorage,
        api_key: impl Into<String>,
        options: NeonMinterOptions,
    ) -> Result<Self> {
        let api_key = api_key.into();
        let missing = |what: &str| Error::NeedsHostConfig {
            kind: KIND,
            reason: format!("{what} is empty"),
        };
        if branch.project_id.is_empty() {
            return Err(missing("`project_id`"));
        }
        if branch.branch_id.is_empty() {
            return Err(missing("`branch_id`"));
        }
        if api_key.is_empty() {
            return Err(missing("the Neon API key"));
        }
        let api_url = match options.api_url {
            Some(url) => url,
            None => DEFAULT_API_URL
                .parse()
                .expect("the default Neon API URL parses"),
        };
        Ok(Self {
            client: NeonClient::new(CloudClient::new_with_token(api_key), api_url),
            project: branch.project_id.clone(),
            branch: branch.branch_id.clone(),
            default_ttl: options.default_ttl,
            gc_interval: TimeDelta::from_std(options.gc_interval).unwrap_or(TimeDelta::MAX),
            clock: options.clock,
            given_storage: options.branch_storage,
            hints: OnceCell::new(),
            read: AsyncMutex::default(),
            read_write: AsyncMutex::default(),
            last_gc: Mutex::default(),
        })
    }

    /// The lifetime to mint for `requested`: the default when unset, clamped.
    fn ttl(&self, requested: Option<Duration>) -> TimeDelta {
        let ttl = requested
            .unwrap_or(self.default_ttl)
            .clamp(MIN_TTL, MAX_TTL);
        TimeDelta::from_std(ttl).expect("at most 12 hours")
    }

    /// The branch's store hints, discovered once.
    async fn hints(&self) -> Result<&StoreHints> {
        self.hints
            .get_or_try_init(|| async {
                let storage = match &self.given_storage {
                    Some(storage) => storage.clone(),
                    None => self
                        .client
                        .branch_storage(&self.project, &self.branch)
                        .await
                        .map_err(Error::mint)?,
                };
                if !storage.enabled {
                    return Err(Error::unsupported(
                        self.branch_name(),
                        "object storage is not enabled",
                    ));
                }
                Ok(StoreHints {
                    endpoint: Some(storage.s3_endpoint),
                    region: Some(storage.region),
                    path_style: Some(storage.force_path_style),
                    ..StoreHints::default()
                })
            })
            .await
    }

    /// Creates a key for `access` living `ttl` from `now`.
    async fn create(
        &self,
        access: AccessLevel,
        ttl: TimeDelta,
        now: DateTime<Utc>,
    ) -> Result<Cached> {
        let (label, scopes): (&str, &[&str]) = match access {
            AccessLevel::Read => ("read", &[SCOPE_STORAGE_READ]),
            AccessLevel::ReadWrite => ("read_write", &[SCOPE_STORAGE_READ, SCOPE_STORAGE_WRITE]),
        };
        let expires_at = now + ttl;
        let request = CreateCredential::new(scopes.iter().copied())
            .with_name(format!("{NAME_PREFIX}{label}-{}", now.timestamp()))
            .with_expires_at(expires_at);
        let created = self
            .client
            .create_credential_with(&self.project, &self.branch, &request)
            .await
            .map_err(Error::mint)?;
        // Neon echoes the expiry it enforces, at second precision.
        let expires_at = created.expires_at.unwrap_or(expires_at);
        Ok(Cached {
            key_id: created.secret.token_id,
            secret: created.secret.s3_secret_access_key,
            expires_at,
            stale_at: now + (expires_at - now) / 2,
        })
    }

    /// Revokes the branch's expired `olai-vend-*` keys, at most once per GC interval.
    /// Failures are logged, never returned.
    async fn collect_garbage(&self, now: DateTime<Utc>) {
        {
            let mut last = self.last_gc.lock().unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|last| now < last + self.gc_interval) {
                return;
            }
            // Set before the pass, so a failing control plane is not asked again on
            // every mint.
            *last = Some(now);
        }
        let listed = match self
            .client
            .list_credentials(&self.project, &self.branch)
            .await
        {
            Ok(listed) => listed,
            Err(err) => {
                tracing::warn!(branch = %self.branch, error = %err, "listing Neon credentials for garbage collection failed");
                return;
            }
        };
        // Neon lists the ancestors' keys too, and keeps expired and revoked ones.
        let expired = listed.into_iter().filter(|c| {
            c.branch_id == self.branch
                && c.name
                    .as_deref()
                    .is_some_and(|n| n.starts_with(NAME_PREFIX))
                && c.expires_at.is_some_and(|t| t <= now)
                && c.revoked_at.is_none()
        });
        for credential in expired.take(GC_BATCH) {
            if let Err(err) = self
                .client
                .revoke_credential(&self.project, &self.branch, &credential.token_id)
                .await
            {
                tracing::warn!(branch = %self.branch, token_id = %credential.token_id, error = %err, "revoking an expired Neon credential failed");
            }
        }
    }

    fn branch_name(&self) -> String {
        format!(
            "neon branch `{}` of project `{}`",
            self.branch, self.project
        )
    }
}

#[async_trait]
impl CredentialMinter for NeonTrustedMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        if request.scope.location().provider() != StorageProvider::Aws {
            return Err(Error::unsupported(
                &request.scope,
                "Neon storage is addressed as `s3://bucket[/prefix]`",
            ));
        }
        let hints = self.hints().await?.clone();
        let slot = match request.access {
            AccessLevel::Read => &self.read,
            AccessLevel::ReadWrite => &self.read_write,
        };
        // Held across the create, so concurrent misses create one key.
        let mut cached = slot.lock().await;
        let now = (self.clock)();
        let mut created = false;
        if !cached.as_ref().is_some_and(|c| c.stale_at > now) {
            *cached = Some(
                self.create(request.access, self.ttl(request.ttl), now)
                    .await?,
            );
            created = true;
        }
        let key = cached.as_ref().expect("set above");
        let vended = VendedCredential::new(
            StorageCredentials::aws(key.key_id.clone(), key.secret.clone(), None),
            request.scope.clone(),
            request.access,
        )
        .with_expiry(Some(key.expires_at))
        .with_hints(hints);
        drop(cached);
        if created {
            self.collect_garbage(now).await;
        }
        Ok(vended)
    }
}

impl fmt::Debug for NeonTrustedMinter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeonTrustedMinter")
            .field("project", &self.project)
            .field("branch", &self.branch)
            .field("api_url", &self.client.api_url().as_str())
            .field("default_ttl", &self.default_ttl)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Mock, Server, ServerGuard};
    use serde_json::json;

    use super::*;

    const BRANCH: &str = "/projects/proj/branches/br-1";

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap()
    }

    fn rfc3339(time: DateTime<Utc>) -> String {
        time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    /// A clock tests move by hand.
    fn clock() -> (Arc<Mutex<DateTime<Utc>>>, Clock) {
        let now = Arc::new(Mutex::new(t(0)));
        let read = Arc::clone(&now);
        (now, Arc::new(move || *read.lock().unwrap()))
    }

    fn storage() -> BranchStorage {
        serde_json::from_value(json!({
            "enabled": true,
            "s3_endpoint": "https://br-1.storage.c-7.eu-central-1.aws.neon.tech",
            "region": "eu-central-1",
            "force_path_style": true,
        }))
        .unwrap()
    }

    fn minter(server: &ServerGuard, clock: Clock) -> NeonTrustedMinter {
        NeonTrustedMinter::new(
            &NeonStorage {
                project_id: "proj".into(),
                branch_id: "br-1".into(),
            },
            "SECRET-napi",
            NeonMinterOptions::default()
                .with_api_url(server.url().parse().unwrap())
                .with_branch_storage(storage())
                .with_clock(clock),
        )
        .unwrap()
    }

    /// Expects `n` creates for `scopes` at `now` living `ttl` seconds, answering with
    /// key `key`.
    async fn create(
        server: &mut ServerGuard,
        scopes: &[&str],
        label: &str,
        now: DateTime<Utc>,
        ttl: i64,
        key: &str,
        n: usize,
    ) -> Mock {
        let expires_at = rfc3339(now + TimeDelta::seconds(ttl));
        server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .match_header("authorization", "Bearer SECRET-napi")
            .match_body(Matcher::Json(json!({
                "scopes": scopes,
                "principal_type": "user",
                "name": format!("olai-vend-{label}-{}", now.timestamp()),
                "expires_at": expires_at,
            })))
            .with_status(201)
            .with_body(
                json!({
                    "token_id": key,
                    "s3_secret_access_key": format!("SECRET-{key}"),
                    "scopes": scopes,
                    "branch_id": "br-1",
                    "expires_at": expires_at,
                })
                .to_string(),
            )
            .expect(n)
            .create_async()
            .await
    }

    /// Answers every garbage-collection listing with no credentials.
    async fn empty_listing(server: &mut ServerGuard) -> Mock {
        server
            .mock("GET", format!("{BRANCH}/credentials").as_str())
            .with_body(json!({ "credentials": [] }).to_string())
            .create_async()
            .await
    }

    fn request(url: &str, access: AccessLevel) -> MintRequest {
        MintRequest::new(url.parse().unwrap(), access)
    }

    fn key_id(vended: &VendedCredential) -> &str {
        match &vended.credentials {
            StorageCredentials::Aws { key_id, token, .. } => {
                assert_eq!(token, &None);
                key_id
            }
            other => panic!("expected AWS credentials, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mints_per_access_level_with_the_branch_hints() {
        let mut server = Server::new_async().await;
        empty_listing(&mut server).await;
        let read = create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            3600,
            "nak_r",
            1,
        )
        .await;
        let write = create(
            &mut server,
            &["storage:read", "storage:write"],
            "read_write",
            t(0),
            3600,
            "nak_rw",
            1,
        )
        .await;
        let (_, clock) = clock();
        let neon = minter(&server, clock);

        let vended = neon
            .mint(&request("s3://one/tab", AccessLevel::Read))
            .await
            .unwrap();
        assert_eq!(key_id(&vended), "nak_r");
        assert_eq!(vended.scope, "s3://one/tab".parse().unwrap());
        assert_eq!(vended.access, AccessLevel::Read);
        assert_eq!(vended.expires_at, Some(t(3600)));
        assert_eq!(
            vended.hints,
            StoreHints {
                endpoint: Some("https://br-1.storage.c-7.eu-central-1.aws.neon.tech".into()),
                region: Some("eu-central-1".into()),
                path_style: Some(true),
                ..StoreHints::default()
            }
        );
        // Another scope at the same level reuses the key: it is not scope-limited.
        let other = neon
            .mint(&request("s3://two/other", AccessLevel::Read))
            .await
            .unwrap();
        assert_eq!(key_id(&other), "nak_r");
        assert_eq!(other.scope, "s3://two/other".parse().unwrap());

        let rw = neon
            .mint(&request("s3://one/tab", AccessLevel::ReadWrite))
            .await
            .unwrap();
        assert_eq!(key_id(&rw), "nak_rw");
        assert_eq!(rw.access, AccessLevel::ReadWrite);
        read.assert_async().await;
        write.assert_async().await;
    }

    #[tokio::test]
    async fn keys_are_minted_again_after_half_their_lifetime() {
        let mut server = Server::new_async().await;
        empty_listing(&mut server).await;
        let first = create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            3600,
            "nak_1",
            1,
        )
        .await;
        let second = create(
            &mut server,
            &["storage:read"],
            "read",
            t(1800),
            3600,
            "nak_2",
            1,
        )
        .await;
        let (now, clock) = clock();
        let neon = minter(&server, clock);
        let tab = request("s3://one/tab", AccessLevel::Read);

        assert_eq!(key_id(&neon.mint(&tab).await.unwrap()), "nak_1");
        *now.lock().unwrap() = t(1799);
        assert_eq!(key_id(&neon.mint(&tab).await.unwrap()), "nak_1");
        *now.lock().unwrap() = t(1800);
        let fresh = neon.mint(&tab).await.unwrap();
        assert_eq!(key_id(&fresh), "nak_2");
        assert_eq!(fresh.expires_at, Some(t(5400)));
        first.assert_async().await;
        second.assert_async().await;
    }

    #[tokio::test]
    async fn lifetimes_are_clamped() {
        let mut server = Server::new_async().await;
        empty_listing(&mut server).await;
        // A minute is raised to 15 minutes; a day is cut to 12 hours.
        let short = create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            900,
            "nak_s",
            1,
        )
        .await;
        let long = create(
            &mut server,
            &["storage:read", "storage:write"],
            "read_write",
            t(0),
            12 * 3600,
            "nak_l",
            1,
        )
        .await;
        let (_, clock) = clock();
        let neon = minter(&server, clock);
        neon.mint(&request("s3://one", AccessLevel::Read).with_ttl(Duration::from_secs(60)))
            .await
            .unwrap();
        neon.mint(
            &request("s3://one", AccessLevel::ReadWrite).with_ttl(Duration::from_secs(86_400)),
        )
        .await
        .unwrap();
        short.assert_async().await;
        long.assert_async().await;
    }

    #[tokio::test]
    async fn concurrent_misses_create_one_key() {
        let mut server = Server::new_async().await;
        empty_listing(&mut server).await;
        let create = create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            3600,
            "nak_1",
            1,
        )
        .await;
        let (_, clock) = clock();
        let neon = Arc::new(minter(&server, clock));
        let mints = (0..8).map(|n| {
            let neon = Arc::clone(&neon);
            tokio::spawn(async move {
                neon.mint(&request(&format!("s3://b/{n}"), AccessLevel::Read))
                    .await
                    .unwrap()
            })
        });
        for mint in mints {
            assert_eq!(key_id(&mint.await.unwrap()), "nak_1");
        }
        create.assert_async().await;
    }

    #[tokio::test]
    async fn garbage_collection_revokes_only_expired_own_keys() {
        let mut server = Server::new_async().await;
        let credential =
            |id: &str, name: &str, branch: &str, expires: Option<i64>, revoked: bool| {
                let mut c = json!({
                    "token_id": id, "name": name, "scopes": ["storage:read"],
                    "branch_id": branch, "principal_type": "user",
                    "created_at": rfc3339(t(-7200)),
                });
                if let Some(expires) = expires {
                    c["expires_at"] = json!(rfc3339(t(expires)));
                }
                if revoked {
                    c["revoked_at"] = json!(rfc3339(t(-60)));
                }
                c
            };
        let listing = server
            .mock("GET", format!("{BRANCH}/credentials").as_str())
            .with_body(
                json!({ "credentials": [
                    credential("nak_expired", "olai-vend-read-1", "br-1", Some(-10), false),
                    credential("nak_live", "olai-vend-read-2", "br-1", Some(3600), false),
                    credential("nak_revoked", "olai-vend-read-3", "br-1", Some(-10), true),
                    credential("nak_parent", "olai-vend-read-4", "br-0", Some(-10), false),
                    credential("nak_theirs", "etl", "br-1", Some(-10), false),
                    credential("nak_forever", "olai-vend-read-5", "br-1", None, false),
                ]})
                .to_string(),
            )
            .expect(2)
            .create_async()
            .await;
        let revoke = server
            .mock(
                "DELETE",
                format!("{BRANCH}/credentials/nak_expired").as_str(),
            )
            .with_status(204)
            .expect(2)
            .create_async()
            .await;
        let others = server
            .mock(
                "DELETE",
                Matcher::Regex(format!(
                    "^{BRANCH}/credentials/nak_(live|revoked|parent|theirs|forever)$"
                )),
            )
            .expect(0)
            .create_async()
            .await;
        create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            900,
            "nak_1",
            1,
        )
        .await;
        create(
            &mut server,
            &["storage:read"],
            "read",
            t(450),
            900,
            "nak_2",
            1,
        )
        .await;
        create(
            &mut server,
            &["storage:read"],
            "read",
            t(900),
            900,
            "nak_3",
            1,
        )
        .await;
        let (now, clock) = clock();
        let neon = NeonTrustedMinter::new(
            &NeonStorage {
                project_id: "proj".into(),
                branch_id: "br-1".into(),
            },
            "SECRET-napi",
            NeonMinterOptions::default()
                .with_api_url(server.url().parse().unwrap())
                .with_branch_storage(storage())
                .with_default_ttl(Duration::from_secs(900))
                .with_gc_interval(Duration::from_secs(600))
                .with_clock(clock),
        )
        .unwrap();
        let tab = request("s3://one", AccessLevel::Read);

        // The first mint collects; one within the interval does not; one after does.
        neon.mint(&tab).await.unwrap();
        *now.lock().unwrap() = t(450);
        neon.mint(&tab).await.unwrap();
        *now.lock().unwrap() = t(900);
        neon.mint(&tab).await.unwrap();
        listing.assert_async().await;
        revoke.assert_async().await;
        others.assert_async().await;
    }

    #[tokio::test]
    async fn garbage_collection_failures_do_not_fail_mints() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", format!("{BRANCH}/credentials").as_str())
            .with_status(500)
            .create_async()
            .await;
        create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            3600,
            "nak_1",
            1,
        )
        .await;
        let (_, clock) = clock();
        let vended = minter(&server, clock)
            .mint(&request("s3://one", AccessLevel::Read))
            .await
            .unwrap();
        assert_eq!(key_id(&vended), "nak_1");
    }

    #[tokio::test]
    async fn failed_creates_are_mint_errors() {
        let mut server = Server::new_async().await;
        let create = server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .with_status(502)
            .expect(1)
            .create_async()
            .await;
        let (_, clock) = clock();
        let err = minter(&server, clock)
            .mint(&request("s3://one", AccessLevel::Read))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Mint { .. }), "{err}");
        assert!(err.to_string().contains("502"), "{err}");
        create.assert_async().await;
    }

    #[tokio::test]
    async fn discovers_the_branch_storage_once() {
        let mut server = Server::new_async().await;
        let discovered = server
            .mock("GET", format!("{BRANCH}/storage").as_str())
            .with_body(
                json!({
                    "enabled": true,
                    "s3_endpoint": "https://br-1.storage.example",
                    "region": "eu-central-1",
                    "force_path_style": true,
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        empty_listing(&mut server).await;
        create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            3600,
            "nak_1",
            1,
        )
        .await;
        let (_, clock) = clock();
        let neon = NeonTrustedMinter::new(
            &NeonStorage {
                project_id: "proj".into(),
                branch_id: "br-1".into(),
            },
            "SECRET-napi",
            NeonMinterOptions::default()
                .with_api_url(server.url().parse().unwrap())
                .with_clock(clock),
        )
        .unwrap();
        for _ in 0..2 {
            let vended = neon
                .mint(&request("s3://one", AccessLevel::Read))
                .await
                .unwrap();
            assert_eq!(
                vended.hints.endpoint.as_deref(),
                Some("https://br-1.storage.example")
            );
        }
        discovered.assert_async().await;
    }

    #[tokio::test]
    async fn refuses_other_storage_and_bad_config() {
        let server = Server::new_async().await;
        let (_, clock) = clock();
        let err = minter(&server, clock)
            .mint(&request("gs://bucket/tab", AccessLevel::Read))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");

        for (project, branch, key) in [("", "br-1", "k"), ("p", "", "k"), ("p", "br-1", "")] {
            let err = NeonTrustedMinter::new(
                &NeonStorage {
                    project_id: project.into(),
                    branch_id: branch.into(),
                },
                key,
                NeonMinterOptions::default(),
            )
            .unwrap_err();
            assert!(matches!(err, Error::NeedsHostConfig { .. }), "{err}");
        }
    }

    #[tokio::test]
    async fn debug_shows_no_secrets() {
        let mut server = Server::new_async().await;
        empty_listing(&mut server).await;
        create(
            &mut server,
            &["storage:read"],
            "read",
            t(0),
            3600,
            "nak_1",
            1,
        )
        .await;
        let (_, clock) = clock();
        let neon = minter(&server, clock);
        let vended = neon
            .mint(&request("s3://one", AccessLevel::Read))
            .await
            .unwrap();
        for debug in [
            format!("{neon:?}"),
            format!("{vended:?}"),
            format!("{:?}", NeonMinterOptions::default()),
        ] {
            assert!(!debug.contains("SECRET"), "{debug}");
        }
        assert!(format!("{neon:?}").contains("br-1"));
    }
}
