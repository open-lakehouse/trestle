use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::error::BoxError;

/// A user's standing with the identity provider: banned, and whether an administrator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserStatus {
    /// The user may not use the service.
    pub banned: bool,
    /// The user is an administrator.
    pub admin: bool,
}

impl UserStatus {
    /// The status of a user whose role column holds `role`.
    ///
    /// The user is an admin when `role` is `admin`, or a comma-separated list containing
    /// `admin` (surrounding whitespace is ignored).
    pub fn from_role(role: Option<&str>, banned: bool) -> Self {
        let admin = role.is_some_and(|roles| roles.split(',').any(|r| r.trim() == "admin"));
        Self { banned, admin }
    }
}

/// Looks up a user's [`UserStatus`], implemented by the host against the identity
/// provider's user records.
///
/// Every authenticated user is looked up: a token proves who the caller is, not whether
/// they are still allowed in. [`Authenticator`](crate::Authenticator) caches the results
/// for a short time.
#[async_trait]
pub trait UserStatusStore: Send + Sync + 'static {
    /// The status of user `sub` of `issuer`, or `None` if there is no such user (for
    /// example, it was deleted after the token was issued).
    async fn status(&self, issuer: &str, sub: &str) -> Result<Option<UserStatus>, BoxError>;
}

/// The most users the cache holds before it drops entries.
const MAX_ENTRIES: usize = 10_000;

type Entry = (Instant, Option<UserStatus>);

/// A [`UserStatusStore`] wrapper that remembers each answer for `ttl`.
pub(crate) struct CachedUserStatus {
    store: Box<dyn UserStatusStore>,
    ttl: Duration,
    entries: Mutex<HashMap<(String, String), Entry>>,
}

impl CachedUserStatus {
    pub(crate) fn new(store: Box<dyn UserStatusStore>, ttl: Duration) -> Self {
        Self {
            store,
            ttl,
            entries: Mutex::default(),
        }
    }

    pub(crate) async fn status(
        &self,
        issuer: &str,
        sub: &str,
    ) -> Result<Option<UserStatus>, BoxError> {
        if self.ttl.is_zero() {
            return self.store.status(issuer, sub).await;
        }
        let key = (issuer.to_owned(), sub.to_owned());
        if let Some((at, status)) = self.lock().get(&key)
            && at.elapsed() < self.ttl
        {
            return Ok(*status);
        }
        // Failures are not cached: the next request asks again.
        let status = self.store.status(issuer, sub).await?;
        let mut entries = self.lock();
        if entries.len() >= MAX_ENTRIES {
            entries.retain(|_, (at, _)| at.elapsed() < self.ttl);
            if entries.len() >= MAX_ENTRIES {
                entries.clear();
            }
        }
        entries.insert(key, (Instant::now(), status));
        Ok(status)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Answers every lookup with `status` and counts the lookups.
    #[derive(Clone)]
    pub(crate) struct Fixed {
        pub(crate) status: Option<UserStatus>,
        pub(crate) lookups: Arc<AtomicUsize>,
    }

    impl Fixed {
        pub(crate) fn new(status: Option<UserStatus>) -> Self {
            Self {
                status,
                lookups: Arc::default(),
            }
        }

        pub(crate) fn lookups(&self) -> usize {
            self.lookups.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl UserStatusStore for Fixed {
        async fn status(&self, _: &str, _: &str) -> Result<Option<UserStatus>, BoxError> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(self.status)
        }
    }

    #[test]
    fn admin_comes_from_the_role_list() {
        assert!(!UserStatus::from_role(None, false).admin);
        assert!(!UserStatus::from_role(Some("user"), false).admin);
        assert!(!UserStatus::from_role(Some("administrator"), false).admin);
        assert!(UserStatus::from_role(Some("admin"), false).admin);
        assert!(UserStatus::from_role(Some("user, admin"), false).admin);
        assert!(UserStatus::from_role(Some("admin"), true).banned);
    }

    #[tokio::test]
    async fn caches_answers_for_the_ttl() {
        let store = Fixed::new(Some(UserStatus::default()));
        let cached = CachedUserStatus::new(Box::new(store.clone()), Duration::from_secs(3600));
        cached.status("iss", "a").await.unwrap();
        cached.status("iss", "a").await.unwrap();
        assert_eq!(store.lookups(), 1);
        cached.status("iss", "b").await.unwrap();
        cached.status("other", "a").await.unwrap();
        assert_eq!(store.lookups(), 3);
    }

    #[tokio::test]
    async fn zero_ttl_always_asks_the_store() {
        let store = Fixed::new(None);
        let cached = CachedUserStatus::new(Box::new(store.clone()), Duration::ZERO);
        assert_eq!(cached.status("iss", "a").await.unwrap(), None);
        cached.status("iss", "a").await.unwrap();
        assert_eq!(store.lookups(), 2);
    }
}
