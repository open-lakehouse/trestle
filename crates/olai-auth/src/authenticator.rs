use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{Extensions, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};

use crate::actor::{Actor, ActorVerifier, TrustedCompute};
use crate::api_key::{API_KEY_ISSUER, API_KEY_PREFIX, ApiKeyStore, CachedApiKeys, parse_api_key};
use crate::error::{AuthError, InvalidToken};
use crate::jwt::{JwtVerifier, peek_issuer};
use crate::principal::{AuthMethod, Principal, PrincipalKind};
use crate::user_status::{CachedUserStatus, UserStatusStore};

/// How long a user's status is cached unless configured otherwise.
const DEFAULT_USER_STATUS_TTL: Duration = Duration::from_secs(30);
/// How long a live API key is cached unless configured otherwise.
const DEFAULT_API_KEY_TTL: Duration = Duration::from_secs(10);

/// Turns a request's bearer token into a [`Principal`].
///
/// Built with [`Authenticator::builder`]. Cloning is cheap: clones share the verifiers
/// and caches.
#[derive(Clone)]
pub struct Authenticator {
    inner: Arc<Inner>,
}

struct Inner {
    jwt: HashMap<String, JwtVerifier>,
    user_status: CachedUserStatus,
    api_keys: Option<CachedApiKeys>,
    actor_verifiers: Vec<Box<dyn ActorVerifier>>,
    trusted_compute: TrustedCompute,
    /// The `WWW-Authenticate` value for a request without a token.
    challenge: HeaderValue,
    /// The `WWW-Authenticate` parameter pointing at the resource metadata, if configured.
    resource_metadata: Option<String>,
}

/// A configuration [`AuthenticatorBuilder::build`] refuses.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// Two verifiers were added for the same issuer.
    #[error("more than one verifier for issuer {0}")]
    DuplicateIssuer(String),
    /// The resource metadata URL can't be put into a `WWW-Authenticate` header.
    #[error("resource metadata URL is not a valid header parameter: {0}")]
    ResourceMetadataUrl(String),
}

/// Builds an [`Authenticator`].
pub struct AuthenticatorBuilder {
    verifiers: Vec<JwtVerifier>,
    user_status: Box<dyn UserStatusStore>,
    user_status_ttl: Duration,
    api_keys: Option<Box<dyn ApiKeyStore>>,
    api_key_ttl: Duration,
    actor_verifiers: Vec<Box<dyn ActorVerifier>>,
    trusted_compute: TrustedCompute,
    resource_metadata_url: Option<String>,
}

impl Authenticator {
    /// Starts building an authenticator that looks users up in `user_status`.
    ///
    /// The store is required: whether a user is banned or an admin always comes from it,
    /// never from token claims.
    pub fn builder(user_status: impl UserStatusStore) -> AuthenticatorBuilder {
        AuthenticatorBuilder {
            verifiers: Vec::new(),
            user_status: Box::new(user_status),
            user_status_ttl: DEFAULT_USER_STATUS_TTL,
            api_keys: None,
            api_key_ttl: DEFAULT_API_KEY_TTL,
            actor_verifiers: Vec::new(),
            trusted_compute: TrustedCompute::new(),
            resource_metadata_url: None,
        }
    }

    /// Authenticates a request: its subject from the `Authorization` header, then its
    /// actor, if it carries an actor credential (see [`actor_verifier`]).
    ///
    /// The actor is resolved against the [`trusted_compute`] rules and set as the
    /// principal's [`actor`](Principal::actor). A request whose actor credentials name
    /// different actors is rejected.
    ///
    /// # Errors
    ///
    /// As [`authenticate`](Self::authenticate) for the subject. [`AuthError::InvalidActor`]
    /// when an actor credential is invalid, and [`AuthError::Unavailable`] when it can't
    /// be checked.
    ///
    /// [`actor_verifier`]: AuthenticatorBuilder::actor_verifier
    /// [`trusted_compute`]: AuthenticatorBuilder::trusted_compute
    pub async fn authenticate_request(
        &self,
        headers: &HeaderMap,
        extensions: &Extensions,
    ) -> Result<Principal, AuthError> {
        let mut principal = self.authenticate(headers.get(AUTHORIZATION)).await?;
        principal.actor = self.actor(headers, extensions).await?;
        Ok(principal)
    }

    async fn actor(
        &self,
        headers: &HeaderMap,
        extensions: &Extensions,
    ) -> Result<Option<Actor>, AuthError> {
        let mut found = None;
        for verifier in &self.inner.actor_verifiers {
            let Some(identity) = verifier.verify(headers, extensions).await? else {
                continue;
            };
            match &found {
                None => found = Some(identity),
                Some(first) if *first == identity => {}
                Some(_) => return Err(AuthError::InvalidActor(InvalidToken::ActorConflict)),
            }
        }
        Ok(found.map(|identity| self.inner.trusted_compute.resolve(identity)))
    }

    /// Authenticates a request from its `Authorization` header.
    ///
    /// # Errors
    ///
    /// [`AuthError::Missing`] when there is no header or it is not a `Bearer` credential;
    /// otherwise as [`authenticate_token`](Self::authenticate_token).
    pub async fn authenticate(
        &self,
        authorization: Option<&HeaderValue>,
    ) -> Result<Principal, AuthError> {
        let token = authorization
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token)
            .ok_or(AuthError::Missing)?;
        self.authenticate_token(token).await
    }

    /// Authenticates a bearer token.
    ///
    /// An API key (`olk_…`, when an [`ApiKeyStore`] is configured) is looked up by id and
    /// its digest compared; it authenticates a service account. A JWT goes to the
    /// verifier of its (unverified) `iss`, then the user's status is looked up.
    ///
    /// # Errors
    ///
    /// [`AuthError::InvalidToken`] when the token is rejected or its subject is unknown,
    /// [`AuthError::Banned`] for a banned user, and [`AuthError::Unavailable`] when keys,
    /// the user status or the API key can't be fetched.
    pub async fn authenticate_token(&self, token: &str) -> Result<Principal, AuthError> {
        if let Some(api_keys) = &self.inner.api_keys
            && token.starts_with(API_KEY_PREFIX)
        {
            return authenticate_api_key(api_keys, token).await;
        }
        if token.bytes().filter(|&b| b == b'.').count() != 2 {
            return Err(InvalidToken::UnsupportedType.into());
        }
        let issuer = peek_issuer(token)?;
        let verifier = self.inner.jwt.get(&issuer).ok_or(InvalidToken::Issuer)?;
        let claims = verifier.verify(token).await?;
        let status = self
            .inner
            .user_status
            .status(&claims.iss, &claims.sub)
            .await
            .map_err(AuthError::Unavailable)?
            .ok_or(InvalidToken::UnknownUser)?;
        if status.banned {
            return Err(AuthError::Banned);
        }
        Ok(Principal {
            kind: PrincipalKind::User {
                sub: claims.sub,
                email: claims.email,
                email_verified: claims.email_verified,
            },
            issuer: claims.iss,
            scopes: claims.scopes,
            client_id: claims.client_id,
            auth_method: AuthMethod::Jwt,
            admin: status.admin,
            issued_at: claims.issued_at,
            actor: None,
        })
    }

    /// The response for a request that failed authentication with `error`.
    ///
    /// A 401 carries an RFC 6750 `WWW-Authenticate: Bearer` challenge, with
    /// `error="invalid_token"` when a token was presented and the configured
    /// `resource_metadata` URL (RFC 9728). A banned user gets 403, a backend failure 503.
    /// The token itself is never included.
    pub fn reject(&self, error: &AuthError) -> Response {
        let status = error.status();
        match error {
            AuthError::Missing => {
                (status, [(WWW_AUTHENTICATE, self.inner.challenge.clone())]).into_response()
            }
            AuthError::InvalidToken(reason) => self.invalid(status, &reason.to_string()),
            AuthError::InvalidActor(reason) => {
                self.invalid(status, &format!("invalid actor credential: {reason}"))
            }
            _ => status.into_response(),
        }
    }
}

impl Authenticator {
    /// A 401 with an `invalid_token` challenge described by `description`, a static text.
    fn invalid(&self, status: axum::http::StatusCode, description: &str) -> Response {
        let mut challenge =
            format!(r#"Bearer error="invalid_token", error_description="{description}""#);
        if let Some(metadata) = &self.inner.resource_metadata {
            challenge.push_str(", ");
            challenge.push_str(metadata);
        }
        let challenge = HeaderValue::try_from(challenge)
            .expect("built from validated parts and static descriptions");
        (status, [(WWW_AUTHENTICATE, challenge)]).into_response()
    }
}

impl AuthenticatorBuilder {
    /// Accepts JWTs from the verifier's issuer.
    pub fn issuer(mut self, verifier: JwtVerifier) -> Self {
        self.verifiers.push(verifier);
        self
    }

    /// Accepts API keys (`olk_<id>_<secret>`) found in `store`. Each key authenticates
    /// as its service account, with the key's scopes.
    pub fn api_keys(mut self, store: impl ApiKeyStore) -> Self {
        self.api_keys = Some(Box::new(store));
        self
    }

    /// How long a live API key is cached (10 seconds by default). A revoked key stops
    /// working within this time. Unknown keys are never cached. [`Duration::ZERO`]
    /// disables the cache.
    pub fn api_key_cache_ttl(mut self, ttl: Duration) -> Self {
        self.api_key_ttl = ttl;
        self
    }

    /// Also accepts the actor credentials `verifier` finds, such as a
    /// [`JwtActorVerifier`](crate::JwtActorVerifier)'s workload tokens. Only
    /// [`authenticate_request`](Authenticator::authenticate_request), which
    /// [`auth_layer`](crate::auth_layer) calls, looks for actors.
    pub fn actor_verifier(mut self, verifier: impl ActorVerifier) -> Self {
        self.actor_verifiers.push(Box::new(verifier));
        self
    }

    /// The rules for which actors are trusted compute. Without them, no actor is.
    pub fn trusted_compute(mut self, trusted_compute: TrustedCompute) -> Self {
        self.trusted_compute = trusted_compute;
        self
    }

    /// How long a user's status is cached (30 seconds by default). A ban or a change of
    /// admin status takes effect within this time. [`Duration::ZERO`] disables the cache.
    pub fn user_status_ttl(mut self, ttl: Duration) -> Self {
        self.user_status_ttl = ttl;
        self
    }

    /// The URL of the resource's OAuth protected resource metadata (RFC 9728), sent in
    /// the `resource_metadata` parameter of every challenge so clients can discover the
    /// authorization server. See [`ProtectedResourceMetadata`](crate::ProtectedResourceMetadata).
    pub fn resource_metadata_url(mut self, url: impl Into<String>) -> Self {
        self.resource_metadata_url = Some(url.into());
        self
    }

    /// Builds the authenticator.
    ///
    /// # Errors
    ///
    /// When two verifiers share an issuer, or the resource metadata URL contains
    /// characters a quoted header parameter can't hold.
    pub fn build(self) -> Result<Authenticator, ConfigError> {
        let mut jwt = HashMap::with_capacity(self.verifiers.len());
        for verifier in self.verifiers {
            let issuer = verifier.issuer().to_owned();
            if jwt.insert(issuer.clone(), verifier).is_some() {
                return Err(ConfigError::DuplicateIssuer(issuer));
            }
        }
        let resource_metadata = match self.resource_metadata_url {
            Some(url) if url.contains(['"', '\\']) || HeaderValue::try_from(&url).is_err() => {
                return Err(ConfigError::ResourceMetadataUrl(url));
            }
            Some(url) => Some(format!(r#"resource_metadata="{url}""#)),
            None => None,
        };
        let challenge = match &resource_metadata {
            Some(metadata) => format!("Bearer {metadata}"),
            None => "Bearer".to_owned(),
        };
        Ok(Authenticator {
            inner: Arc::new(Inner {
                jwt,
                user_status: CachedUserStatus::new(self.user_status, self.user_status_ttl),
                api_keys: self
                    .api_keys
                    .map(|store| CachedApiKeys::new(store, self.api_key_ttl)),
                actor_verifiers: self.actor_verifiers,
                trusted_compute: self.trusted_compute,
                challenge: HeaderValue::try_from(challenge).expect("validated above"),
                resource_metadata,
            }),
        })
    }
}

async fn authenticate_api_key(api_keys: &CachedApiKeys, key: &str) -> Result<Principal, AuthError> {
    let id = parse_api_key(key).ok_or(InvalidToken::Malformed)?;
    let record = api_keys
        .lookup(id)
        .await
        .map_err(AuthError::Unavailable)?
        .filter(|record| record.matches(key))
        .ok_or(InvalidToken::ApiKey)?;
    Ok(Principal {
        kind: PrincipalKind::ServiceAccount {
            id: record.account_id,
            name: record.account_name,
        },
        issuer: API_KEY_ISSUER.to_owned(),
        scopes: record.scopes,
        client_id: None,
        auth_method: AuthMethod::ApiKey,
        admin: false,
        issued_at: None,
        actor: None,
    })
}

/// The token of a `Bearer` credential. The scheme is case-insensitive (RFC 9110).
fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim_matches(' ');
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::api_key::tests::{KEY, Keys};
    use crate::key_source::PushKeySource;
    use crate::test_support::{AUDIENCE, ISSUER, NOW, claims, jwks, sign, signing_key, token};
    use crate::user_status::UserStatus;
    use crate::user_status::tests::Fixed;

    fn verifier(issuer: &str, kid_seed: (&str, u8)) -> JwtVerifier {
        let keys = PushKeySource::new();
        keys.set_jwks(&jwks(&[kid_seed])).unwrap();
        JwtVerifier::new(issuer, keys)
            .with_audience(AUDIENCE)
            .with_clock(|| NOW)
    }

    fn authenticator(store: Fixed) -> Authenticator {
        Authenticator::builder(store)
            .issuer(verifier(ISSUER, ("k1", 1)))
            .issuer(verifier("https://other.example.com", ("o1", 7)))
            .resource_metadata_url("https://api.example.com/.well-known/oauth-protected-resource")
            .build()
            .unwrap()
    }

    fn header(value: &str) -> HeaderValue {
        HeaderValue::from_str(value).unwrap()
    }

    #[tokio::test]
    async fn builds_a_user_principal() {
        let store = Fixed::new(Some(UserStatus::from_role(Some("admin"), false)));
        let t = token(&claims());
        let principal = authenticator(store)
            .authenticate(Some(&header(&format!("Bearer {t}"))))
            .await
            .unwrap();
        assert_eq!(principal.issuer, ISSUER);
        assert_eq!(principal.subject(), "8c1d0f5e-3b7a-4c55-9f40-2d6f1e9b7a10");
        assert_eq!(principal.auth_method, AuthMethod::Jwt);
        assert_eq!(principal.issued_at, Some(NOW - 60));
        assert!(principal.is_admin());
        assert!(matches!(
            principal.kind,
            PrincipalKind::User { email: Some(ref e), email_verified: true, .. } if e == "user@example.com"
        ));
    }

    #[tokio::test]
    async fn ignores_the_role_claim() {
        let store = Fixed::new(Some(UserStatus::default()));
        let mut claims = claims();
        claims["role"] = json!("admin");
        let principal = authenticator(store)
            .authenticate_token(&token(&claims))
            .await
            .unwrap();
        assert!(!principal.is_admin());
    }

    #[tokio::test]
    async fn dispatches_by_issuer() {
        let store = Fixed::new(Some(UserStatus::default()));
        let mut other = claims();
        other["iss"] = json!("https://other.example.com");
        let t = sign(
            &json!({ "alg": "EdDSA", "kid": "o1" }),
            &other,
            &signing_key(7),
        );
        let principal = authenticator(store).authenticate_token(&t).await.unwrap();
        assert_eq!(principal.issuer, "https://other.example.com");

        // Signed with the first issuer's key, but claiming the second issuer.
        let t = sign(
            &json!({ "alg": "EdDSA", "kid": "o1" }),
            &other,
            &signing_key(1),
        );
        let auth = authenticator(Fixed::new(Some(UserStatus::default())));
        assert!(matches!(
            auth.authenticate_token(&t).await,
            Err(AuthError::InvalidToken(InvalidToken::Signature))
        ));

        let mut unknown = claims();
        unknown["iss"] = json!("https://evil.example.com");
        assert!(matches!(
            auth.authenticate_token(&token(&unknown)).await,
            Err(AuthError::InvalidToken(InvalidToken::Issuer))
        ));
    }

    #[tokio::test]
    async fn rejects_banned_and_unknown_users() {
        let banned = Fixed::new(Some(UserStatus::from_role(Some("admin"), true)));
        assert!(matches!(
            authenticator(banned)
                .authenticate_token(&token(&claims()))
                .await,
            Err(AuthError::Banned)
        ));
        let deleted = Fixed::new(None);
        assert!(matches!(
            authenticator(deleted)
                .authenticate_token(&token(&claims()))
                .await,
            Err(AuthError::InvalidToken(InvalidToken::UnknownUser))
        ));
    }

    #[tokio::test]
    async fn does_not_look_up_users_for_invalid_tokens() {
        let store = Fixed::new(Some(UserStatus::default()));
        let auth = authenticator(store.clone());
        let mut expired = claims();
        expired["exp"] = json!(NOW - 3600);
        assert!(auth.authenticate_token(&token(&expired)).await.is_err());
        assert_eq!(store.lookups(), 0);
    }

    #[tokio::test]
    async fn requires_a_bearer_credential() {
        let auth = authenticator(Fixed::new(Some(UserStatus::default())));
        let t = token(&claims());
        for value in ["", "Bearer", "Bearer ", "Basic dXNlcjpwdw==", "Token abc"] {
            assert!(
                matches!(
                    auth.authenticate(Some(&header(value))).await,
                    Err(AuthError::Missing)
                ),
                "{value}"
            );
        }
        assert!(matches!(
            auth.authenticate(None).await,
            Err(AuthError::Missing)
        ));
        assert!(
            auth.authenticate(Some(&header(&format!("bearer {t}"))))
                .await
                .is_ok()
        );
        assert!(matches!(
            auth.authenticate(Some(&header("Bearer opaque-api-key")))
                .await,
            Err(AuthError::InvalidToken(InvalidToken::UnsupportedType))
        ));
    }

    fn with_keys(keys: Keys) -> Authenticator {
        Authenticator::builder(Fixed::new(Some(UserStatus::from_role(
            Some("admin"),
            false,
        ))))
        .issuer(verifier(ISSUER, ("k1", 1)))
        .api_keys(keys)
        .build()
        .unwrap()
    }

    #[tokio::test]
    async fn api_keys_authenticate_service_accounts() {
        let auth = with_keys(Keys::with(KEY));
        let principal = auth
            .authenticate(Some(&header(&format!("Bearer {KEY}"))))
            .await
            .unwrap();
        assert_eq!(principal.auth_method, AuthMethod::ApiKey);
        assert_eq!(principal.issuer, API_KEY_ISSUER);
        assert_eq!(principal.subject(), "7d4c1a52-8f0e-4b8e-9a51-1f2b3c4d5e6f");
        assert!(matches!(
            principal.kind,
            PrincipalKind::ServiceAccount { ref name, .. } if name == "ingest-bot"
        ));
        assert!(principal.has_scope("ingest"));
        // Admin comes from the user status store, which service accounts never reach.
        assert!(!principal.is_admin());
        // JWTs still work alongside.
        assert!(auth.authenticate_token(&token(&claims())).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_wrong_unknown_and_revoked_keys() {
        let keys = Keys::with(KEY);
        let auth = with_keys(keys.clone());
        let invalid = |result: Result<Principal, AuthError>| {
            matches!(result, Err(AuthError::InvalidToken(InvalidToken::ApiKey)))
        };

        let wrong_secret = format!("{}0", &KEY[..KEY.len() - 1]);
        assert_ne!(wrong_secret, KEY);
        assert!(invalid(auth.authenticate_token(&wrong_secret).await));
        // The cached record is compared again: a wrong secret never rides on a hit.
        assert!(auth.authenticate_token(KEY).await.is_ok());
        assert!(invalid(auth.authenticate_token(&wrong_secret).await));

        let unknown = KEY.replace("00112233aabbccdd", "ffffffffffffffff");
        assert!(invalid(auth.authenticate_token(&unknown).await));

        let auth = Authenticator::builder(Fixed::new(None))
            .api_keys(keys.clone())
            .api_key_cache_ttl(Duration::ZERO)
            .build()
            .unwrap();
        assert!(auth.authenticate_token(KEY).await.is_ok());
        keys.revoke(KEY);
        assert!(invalid(auth.authenticate_token(KEY).await));
    }

    #[tokio::test]
    async fn malformed_keys_are_not_looked_up() {
        let keys = Keys::with(KEY);
        let auth = with_keys(keys.clone());
        for bad in [
            "olk_",
            "olk_abc_def",
            &KEY.to_uppercase().replacen("OLK_", "olk_", 1),
        ] {
            assert!(
                matches!(
                    auth.authenticate_token(bad).await,
                    Err(AuthError::InvalidToken(InvalidToken::Malformed))
                ),
                "{bad}"
            );
        }
        assert_eq!(keys.lookups(), 0);
    }

    #[tokio::test]
    async fn api_keys_are_unsupported_without_a_store() {
        let auth = authenticator(Fixed::new(Some(UserStatus::default())));
        assert!(matches!(
            auth.authenticate_token(KEY).await,
            Err(AuthError::InvalidToken(InvalidToken::UnsupportedType))
        ));
    }

    #[tokio::test]
    async fn api_key_store_failures_are_unavailable() {
        struct Down;
        #[async_trait::async_trait]
        impl ApiKeyStore for Down {
            async fn lookup(
                &self,
                _: &str,
            ) -> Result<Option<crate::api_key::ApiKeyRecord>, crate::BoxError> {
                Err("connection refused".into())
            }
        }
        let auth = Authenticator::builder(Fixed::new(None))
            .api_keys(Down)
            .build()
            .unwrap();
        assert!(matches!(
            auth.authenticate_token(KEY).await,
            Err(AuthError::Unavailable(_))
        ));
    }

    mod actors {
        use axum::http::{Extensions, HeaderMap, StatusCode};

        use super::*;
        use crate::actor::{
            ACTOR_TOKEN_HEADER, ActorIdentity, ActorMatch, ActorMethod, ActorVerifier, ComputeRule,
            JwtActorVerifier, TrustedCompute,
        };

        const WORKLOAD_ISSUER: &str = "https://oidc.eks.example.com/id/ABC";
        const SPARK: &str = "spiffe://prod.example/ns/etl/sa/spark";

        fn workload_token(sub: &str) -> String {
            sign(
                &json!({ "alg": "EdDSA", "kid": "w1" }),
                &json!({
                    "iss": WORKLOAD_ISSUER,
                    "aud": AUDIENCE,
                    "sub": sub,
                    "iat": NOW - 60,
                    "exp": NOW + 240,
                }),
                &signing_key(9),
            )
        }

        fn authenticator() -> Authenticator {
            let workload = verifier(WORKLOAD_ISSUER, ("w1", 9));
            Authenticator::builder(Fixed::new(Some(UserStatus::default())))
                .issuer(verifier(ISSUER, ("k1", 1)))
                .actor_verifier(JwtActorVerifier::new([workload]).unwrap())
                .trusted_compute(TrustedCompute::new().with(ComputeRule::new(
                    "spark-prod",
                    ActorMatch::Prefix("spiffe://prod.example/ns/etl".into()),
                )))
                .build()
                .unwrap()
        }

        fn request(subject: &str, actor: Option<&str>) -> HeaderMap {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, header(&format!("Bearer {subject}")));
            if let Some(actor) = actor {
                headers.insert(ACTOR_TOKEN_HEADER, header(actor));
            }
            headers
        }

        async fn authenticate(headers: &HeaderMap) -> Result<Principal, AuthError> {
            authenticator()
                .authenticate_request(headers, &Extensions::new())
                .await
        }

        #[tokio::test]
        async fn trusted_compute_acts_for_the_subject() {
            let user = token(&claims());
            let principal = authenticate(&request(&user, Some(&workload_token(SPARK))))
                .await
                .unwrap();
            // Still the user: the actor adds who the request is made from, not who for.
            assert_eq!(principal.subject(), "8c1d0f5e-3b7a-4c55-9f40-2d6f1e9b7a10");
            let actor = principal.actor.as_ref().unwrap();
            assert_eq!(actor.id, SPARK);
            assert_eq!(actor.issuer, WORKLOAD_ISSUER);
            assert_eq!(actor.compute.as_deref(), Some("spark-prod"));
            assert!(principal.is_trusted_compute());
        }

        #[tokio::test]
        async fn other_actors_are_untrusted() {
            let user = token(&claims());
            let laptop = workload_token("laptop");
            let principal = authenticate(&request(&user, Some(&laptop))).await.unwrap();
            let actor = principal.actor.as_ref().unwrap();
            assert_eq!(actor.id, format!("{WORKLOAD_ISSUER}#laptop"));
            assert!(!principal.is_trusted_compute());

            let principal = authenticate(&request(&user, None)).await.unwrap();
            assert_eq!(principal.actor, None);
            assert!(!principal.is_trusted_compute());
        }

        #[tokio::test]
        async fn the_subject_token_cannot_claim_an_actor() {
            let mut claims = claims();
            claims["act"] = json!({ "sub": SPARK });
            claims["trusted"] = json!(true);
            let principal = authenticate(&request(&token(&claims), None)).await.unwrap();
            assert_eq!(principal.actor, None);
            // Nor is the user's own token accepted as an actor token.
            let user = token(&self::claims());
            assert!(matches!(
                authenticate(&request(&user, Some(&user))).await,
                Err(AuthError::InvalidActor(InvalidToken::Issuer))
            ));
        }

        #[tokio::test]
        async fn invalid_actor_credentials_are_rejected() {
            let auth = authenticator();
            let user = token(&claims());
            let error = auth
                .authenticate_request(&request(&user, Some("garbage")), &Extensions::new())
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                AuthError::InvalidActor(InvalidToken::Malformed)
            ));
            let response = auth.reject(&error);
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let challenge = response.headers()[WWW_AUTHENTICATE].to_str().unwrap();
            assert!(
                challenge.contains("invalid actor credential: malformed token"),
                "{challenge}"
            );
        }

        #[tokio::test]
        async fn the_subject_is_still_required() {
            let mut headers = HeaderMap::new();
            headers.insert(ACTOR_TOKEN_HEADER, header(&workload_token(SPARK)));
            assert!(matches!(
                authenticate(&headers).await,
                Err(AuthError::Missing)
            ));
        }

        /// Finds the same evidence every time, as a stand-in for a second method.
        struct Evidence(&'static str);

        #[async_trait::async_trait]
        impl ActorVerifier for Evidence {
            async fn verify(
                &self,
                _: &HeaderMap,
                _: &Extensions,
            ) -> Result<Option<ActorIdentity>, AuthError> {
                Ok(Some(ActorIdentity::new(
                    self.0,
                    WORKLOAD_ISSUER,
                    ActorMethod::Mtls,
                )))
            }
        }

        #[tokio::test]
        async fn actor_credentials_must_agree() {
            let with = |id| {
                Authenticator::builder(Fixed::new(Some(UserStatus::default())))
                    .issuer(verifier(ISSUER, ("k1", 1)))
                    .actor_verifier(Evidence(SPARK))
                    .actor_verifier(Evidence(id))
                    .build()
                    .unwrap()
            };
            let headers = request(&token(&claims()), None);
            let principal = with(SPARK)
                .authenticate_request(&headers, &Extensions::new())
                .await
                .unwrap();
            assert_eq!(principal.actor.unwrap().method, ActorMethod::Mtls);
            assert!(matches!(
                with("spiffe://prod.example/other")
                    .authenticate_request(&headers, &Extensions::new())
                    .await,
                Err(AuthError::InvalidActor(InvalidToken::ActorConflict))
            ));
        }
    }

    #[test]
    fn rejects_bad_configuration() {
        let store = Fixed::new(None);
        let dup = Authenticator::builder(store.clone())
            .issuer(verifier(ISSUER, ("k1", 1)))
            .issuer(verifier(ISSUER, ("k2", 2)))
            .build();
        assert!(matches!(dup, Err(ConfigError::DuplicateIssuer(_))));
        let quote = Authenticator::builder(store)
            .resource_metadata_url("https://api.example.com/\"x")
            .build();
        assert!(matches!(quote, Err(ConfigError::ResourceMetadataUrl(_))));
    }
}
