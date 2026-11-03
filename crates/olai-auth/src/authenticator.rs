use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderValue;
use axum::http::header::WWW_AUTHENTICATE;
use axum::response::{IntoResponse, Response};

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
            resource_metadata_url: None,
        }
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
            AuthError::InvalidToken(reason) => {
                let mut challenge =
                    format!(r#"Bearer error="invalid_token", error_description="{reason}""#);
                if let Some(metadata) = &self.inner.resource_metadata {
                    challenge.push_str(", ");
                    challenge.push_str(metadata);
                }
                let challenge = HeaderValue::try_from(challenge)
                    .expect("built from validated parts and static descriptions");
                (status, [(WWW_AUTHENTICATE, challenge)]).into_response()
            }
            _ => status.into_response(),
        }
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
