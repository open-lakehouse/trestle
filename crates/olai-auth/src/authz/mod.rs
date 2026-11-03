//! Authorization: may this principal perform this action on this resource?

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::principal::{Principal, PrincipalKind};

#[cfg(feature = "cedar")]
pub(crate) mod cedar;
#[cfg(feature = "pdp-client")]
pub(crate) mod http;
pub mod wire;

use wire::{AuthorizeRequest, AuthorizeResponse, EntityRef};

/// Decides [`AuthorizeRequest`]s, wherever the policies are evaluated.
///
/// Handlers ask through this port without knowing whether Cedar runs in a remote policy
/// decision point ([`HttpAuthorizer`](crate::HttpAuthorizer), feature `pdp-client`) or
/// in-process ([`CedarAuthorizer`](crate::CedarAuthorizer), feature `cedar`).
///
/// It cannot fail: an implementation that can't reach a decision answers
/// [`Decision::Deny`](crate::Decision::Deny) with the reason in
/// [`errors`](AuthorizeResponse::errors). It fails closed.
#[async_trait]
pub trait Authorizer: Send + Sync + 'static {
    /// Decides one request.
    async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse;

    /// Decides several requests, answering in order. By default one at a time.
    async fn authorize_batch(&self, requests: &[AuthorizeRequest]) -> Vec<AuthorizeResponse> {
        let mut responses = Vec::with_capacity(requests.len());
        for request in requests {
            responses.push(self.authorize(request).await);
        }
        responses
    }
}

#[async_trait]
impl<A: Authorizer + ?Sized> Authorizer for Arc<A> {
    async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse {
        (**self).authorize(request).await
    }

    async fn authorize_batch(&self, requests: &[AuthorizeRequest]) -> Vec<AuthorizeResponse> {
        (**self).authorize_batch(requests).await
    }
}

/// The Cedar entity types a service's principals and actions map to, such as
/// `Ns::User`, `Ns::ServiceAccount` and `Ns::Action`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTypes {
    user: String,
    service_account: String,
    action: String,
}

impl EntityTypes {
    /// Users map to `user`, service accounts to `service_account`, and action names to
    /// entities of type `action`.
    pub fn new(
        user: impl Into<String>,
        service_account: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            user: user.into(),
            service_account: service_account.into(),
            action: action.into(),
        }
    }

    /// The entity of `principal`: its subject, typed by its kind.
    pub fn principal(&self, principal: &Principal) -> EntityRef {
        let entity_type = match principal.kind {
            PrincipalKind::User { .. } => &self.user,
            PrincipalKind::ServiceAccount { .. } => &self.service_account,
        };
        EntityRef::new(entity_type.clone(), principal.subject())
    }

    /// The entity of action `name`.
    pub fn action(&self, name: impl Into<String>) -> EntityRef {
        EntityRef::new(self.action.clone(), name)
    }

    /// A request for `principal` to perform action `name` on `resource`, with the
    /// [`AuthContext::for_principal`] context. Add to it with
    /// [`with_context`](AuthorizeRequest::with_context) or extend the context first.
    pub fn request(
        &self,
        principal: &Principal,
        action: impl Into<String>,
        resource: EntityRef,
    ) -> AuthorizeRequest {
        AuthorizeRequest::new(self.principal(principal), self.action(action), resource)
            .with_context(AuthContext::for_principal(principal))
    }
}

/// Builds the standard request context.
///
/// | Key | Cedar type | From |
/// |---|---|---|
/// | `auth_method` | `String` (`jwt`, `api_key`) | [`for_principal`](Self::for_principal) |
/// | `client_id` | `String`, when known | [`for_principal`](Self::for_principal) |
/// | `scopes` | `Set<String>` | [`for_principal`](Self::for_principal) |
/// | `token_issued_at` | `Long` (seconds), when known | [`for_principal`](Self::for_principal) |
/// | `source_ip` | `ipaddr` | [`source_ip`](Self::source_ip) |
/// | `request_time` | `datetime` | [`request_time`](Self::request_time) |
/// | `branch` | `String` | [`branch`](Self::branch) |
///
/// Declare optional keys as optional attributes (`client_id?: String`) in the schema's
/// context type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthContext {
    map: Map<String, Value>,
}

impl AuthContext {
    /// An empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// The context describing how `principal` authenticated.
    pub fn for_principal(principal: &Principal) -> Self {
        let mut context = Self::new()
            .with("auth_method", principal.auth_method.as_str())
            .with("scopes", principal.scopes.clone());
        if let Some(client_id) = &principal.client_id {
            context = context.with("client_id", client_id.clone());
        }
        if let Some(iat) = principal.issued_at {
            context = context.with("token_issued_at", iat);
        }
        context
    }

    /// The caller's address, as a Cedar `ipaddr`.
    pub fn source_ip(self, ip: IpAddr) -> Self {
        self.with(
            "source_ip",
            json!({ "__extn": { "fn": "ip", "arg": ip.to_string() } }),
        )
    }

    /// When the request was made, as a Cedar `datetime` (whole seconds, UTC).
    pub fn request_time(self, at: SystemTime) -> Self {
        self.with(
            "request_time",
            json!({ "__extn": { "fn": "datetime", "arg": rfc3339(at) } }),
        )
    }

    /// The deployment branch or environment serving the request.
    pub fn branch(self, branch: impl Into<String>) -> Self {
        self.with("branch", branch.into())
    }

    /// Sets any other context key.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.map.insert(key.into(), value.into());
        self
    }

    /// The context as a JSON object.
    pub fn into_map(self) -> Map<String, Value> {
        self.map
    }
}

impl From<AuthContext> for Map<String, Value> {
    fn from(context: AuthContext) -> Self {
        context.map
    }
}

/// `at` as `YYYY-MM-DDThh:mm:ssZ`, the form Cedar's `datetime` parses.
fn rfc3339(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// How long [`CachedAuthorizer`] keeps a decision unless configured otherwise.
const DEFAULT_DECISION_TTL: Duration = Duration::from_secs(5);
/// The most decisions the cache holds before it drops entries.
const MAX_DECISIONS: usize = 10_000;

/// An [`Authorizer`] wrapper that remembers decisions for a few seconds.
///
/// A decision is reused for the same principal, action, resource, resource entity and
/// context, except that `request_time` is left out of the comparison (it changes on
/// every request). So a grant or revocation takes effect within the TTL, and a policy
/// conditioned on `request_time` can be off by up to the TTL. Answers that carry
/// errors, including every fail-closed deny, are not cached.
pub struct CachedAuthorizer<A> {
    inner: A,
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, AuthorizeResponse)>>,
}

impl<A: Authorizer> CachedAuthorizer<A> {
    /// Caches `inner`'s decisions for 5 seconds.
    pub fn new(inner: A) -> Self {
        Self {
            inner,
            ttl: DEFAULT_DECISION_TTL,
            entries: Mutex::default(),
        }
    }

    /// How long a decision is reused.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    fn cached(&self, key: &str) -> Option<AuthorizeResponse> {
        let entries = self.lock();
        let (at, response) = entries.get(key)?;
        (at.elapsed() < self.ttl).then(|| response.clone())
    }

    fn remember(&self, key: String, response: &AuthorizeResponse) {
        if self.ttl.is_zero() || !response.errors.is_empty() {
            return;
        }
        let mut entries = self.lock();
        if entries.len() >= MAX_DECISIONS {
            entries.retain(|_, (at, _)| at.elapsed() < self.ttl);
            if entries.len() >= MAX_DECISIONS {
                entries.clear();
            }
        }
        entries.insert(key, (Instant::now(), response.clone()));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, AuthorizeResponse)>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What makes two requests the same decision: everything but `request_time`.
fn cache_key(request: &AuthorizeRequest) -> String {
    let mut context = request.context.clone();
    context.remove("request_time");
    json!([
        request.principal,
        request.action,
        request.resource,
        context,
        request.resource_entity,
    ])
    .to_string()
}

#[async_trait]
impl<A: Authorizer> Authorizer for CachedAuthorizer<A> {
    async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse {
        let key = cache_key(request);
        if let Some(response) = self.cached(&key) {
            return response;
        }
        let response = self.inner.authorize(request).await;
        self.remember(key, &response);
        response
    }

    async fn authorize_batch(&self, requests: &[AuthorizeRequest]) -> Vec<AuthorizeResponse> {
        let keys: Vec<String> = requests.iter().map(cache_key).collect();
        let mut responses: Vec<Option<AuthorizeResponse>> =
            keys.iter().map(|key| self.cached(key)).collect();
        let misses: Vec<usize> = (0..requests.len())
            .filter(|&i| responses[i].is_none())
            .collect();
        if !misses.is_empty() {
            let pending: Vec<AuthorizeRequest> =
                misses.iter().map(|&i| requests[i].clone()).collect();
            let answers = self.inner.authorize_batch(&pending).await;
            for (n, &i) in misses.iter().enumerate() {
                let answer = answers.get(n).cloned().unwrap_or_else(|| {
                    AuthorizeResponse::deny("authorizer answered fewer requests than asked")
                });
                self.remember(keys[i].clone(), &answer);
                responses[i] = Some(answer);
            }
        }
        responses
            .into_iter()
            .map(|response| response.unwrap_or_else(|| AuthorizeResponse::deny("no decision")))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::principal::AuthMethod;
    use crate::wire::Decision;

    pub(crate) fn user() -> Principal {
        Principal {
            kind: PrincipalKind::User {
                sub: "u1".into(),
                email: None,
                email_verified: false,
            },
            issuer: "https://auth.example.com".into(),
            scopes: vec!["read".into()],
            client_id: Some("cli".into()),
            auth_method: AuthMethod::Jwt,
            admin: false,
            issued_at: Some(1_800_000_000),
        }
    }

    /// Allows `read` and denies anything else; `boom` fails. Counts the requests.
    #[derive(Default, Clone)]
    struct Counting(Arc<AtomicUsize>);

    #[async_trait]
    impl Authorizer for Counting {
        async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse {
            self.0.fetch_add(1, Ordering::SeqCst);
            match request.action.id.as_str() {
                "read" => AuthorizeResponse {
                    decision: Decision::Allow,
                    policies: vec!["p0".into()],
                    errors: vec![],
                },
                "boom" => AuthorizeResponse::deny("backend down"),
                _ => AuthorizeResponse {
                    decision: Decision::Deny,
                    policies: vec![],
                    errors: vec![],
                },
            }
        }
    }

    fn types() -> EntityTypes {
        EntityTypes::new("Ns::User", "Ns::ServiceAccount", "Ns::Action")
    }

    #[test]
    fn maps_principals_and_actions_to_entities() {
        let types = types();
        assert_eq!(types.principal(&user()), EntityRef::new("Ns::User", "u1"));
        let mut machine = user();
        machine.kind = PrincipalKind::ServiceAccount {
            id: "sa1".into(),
            name: "bot".into(),
        };
        assert_eq!(
            types.principal(&machine),
            EntityRef::new("Ns::ServiceAccount", "sa1")
        );
        assert_eq!(types.action("read"), EntityRef::new("Ns::Action", "read"));
    }

    #[test]
    fn builds_the_standard_context() {
        let at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let context = AuthContext::for_principal(&user())
            .source_ip("192.0.2.7".parse().unwrap())
            .request_time(at)
            .branch("preview-42")
            .into_map();
        assert_eq!(
            Value::Object(context),
            json!({
                "auth_method": "jwt",
                "scopes": ["read"],
                "client_id": "cli",
                "token_issued_at": 1_800_000_000,
                "source_ip": { "__extn": { "fn": "ip", "arg": "192.0.2.7" } },
                "request_time": { "__extn": { "fn": "datetime", "arg": "2027-01-15T08:00:00Z" } },
                "branch": "preview-42",
            })
        );
        let mut key = user();
        key.auth_method = AuthMethod::ApiKey;
        key.client_id = None;
        key.issued_at = None;
        let context = AuthContext::for_principal(&key).into_map();
        assert_eq!(context["auth_method"], "api_key");
        assert!(!context.contains_key("client_id"));
        assert!(!context.contains_key("token_issued_at"));
    }

    #[test]
    fn formats_datetimes() {
        let at = |secs| rfc3339(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(at(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    fn request(action: &str) -> AuthorizeRequest {
        types().request(&user(), action, EntityRef::new("Ns::Doc", "d1"))
    }

    #[tokio::test]
    async fn caches_decisions_for_the_ttl() {
        let inner = Counting::default();
        let cached = CachedAuthorizer::new(inner.clone()).with_ttl(Duration::from_secs(3600));
        assert!(cached.authorize(&request("read")).await.is_allowed());
        assert!(cached.authorize(&request("read")).await.is_allowed());
        assert!(!cached.authorize(&request("write")).await.is_allowed());
        assert!(!cached.authorize(&request("write")).await.is_allowed());
        assert_eq!(inner.0.load(Ordering::SeqCst), 2);

        // Another resource or context is another decision; the request time is not.
        let other = types().request(&user(), "read", EntityRef::new("Ns::Doc", "d2"));
        cached.authorize(&other).await;
        let from_elsewhere = request("read").with_context(
            AuthContext::for_principal(&user()).source_ip("198.51.100.1".parse().unwrap()),
        );
        cached.authorize(&from_elsewhere).await;
        assert_eq!(inner.0.load(Ordering::SeqCst), 4);
        let later = request("read")
            .with_context(AuthContext::for_principal(&user()).request_time(SystemTime::now()));
        cached.authorize(&later).await;
        assert_eq!(inner.0.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn expires_decisions() {
        let inner = Counting::default();
        let ttl = Duration::from_millis(30);
        let cached = CachedAuthorizer::new(inner.clone()).with_ttl(ttl);
        cached.authorize(&request("read")).await;
        tokio::time::sleep(ttl * 2).await;
        cached.authorize(&request("read")).await;
        assert_eq!(inner.0.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn does_not_cache_errors() {
        let inner = Counting::default();
        let cached = CachedAuthorizer::new(inner.clone()).with_ttl(Duration::from_secs(3600));
        for _ in 0..2 {
            let response = cached.authorize(&request("boom")).await;
            assert_eq!(response.decision, Decision::Deny);
            assert_eq!(response.errors, ["backend down"]);
        }
        assert_eq!(inner.0.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn batches_only_the_misses() {
        let inner = Counting::default();
        let cached = CachedAuthorizer::new(inner.clone()).with_ttl(Duration::from_secs(3600));
        cached.authorize(&request("read")).await;
        let answers = cached
            .authorize_batch(&[request("write"), request("read"), request("boom")])
            .await;
        let decisions: Vec<Decision> = answers.iter().map(|a| a.decision).collect();
        assert_eq!(decisions, [Decision::Deny, Decision::Allow, Decision::Deny]);
        assert_eq!(answers[1].policies, ["p0"]);
        assert_eq!(inner.0.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_short_batch_answer_fails_closed() {
        struct Short;
        #[async_trait]
        impl Authorizer for Short {
            async fn authorize(&self, _: &AuthorizeRequest) -> AuthorizeResponse {
                AuthorizeResponse {
                    decision: Decision::Allow,
                    policies: vec![],
                    errors: vec![],
                }
            }
            async fn authorize_batch(&self, _: &[AuthorizeRequest]) -> Vec<AuthorizeResponse> {
                vec![]
            }
        }
        let answers = CachedAuthorizer::new(Short)
            .authorize_batch(&[request("read")])
            .await;
        assert_eq!(answers[0].decision, Decision::Deny);
    }
}
