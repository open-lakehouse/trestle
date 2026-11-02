use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderValue;
use axum::http::header::WWW_AUTHENTICATE;
use axum::response::{IntoResponse, Response};

use crate::error::{AuthError, InvalidToken};
use crate::jwt::{JwtVerifier, peek_issuer};
use crate::principal::{AuthMethod, Principal, PrincipalKind};
use crate::user_status::{CachedUserStatus, UserStatusStore};

/// How long a user's status is cached unless configured otherwise.
const DEFAULT_USER_STATUS_TTL: Duration = Duration::from_secs(30);

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
    /// A JWT goes to the verifier of its (unverified) `iss`, then the subject's status is
    /// looked up.
    ///
    /// # Errors
    ///
    /// [`AuthError::InvalidToken`] when the token is rejected or its subject is unknown,
    /// [`AuthError::Banned`] for a banned user, and [`AuthError::Unavailable`] when keys
    /// or the user status can't be fetched.
    pub async fn authenticate_token(&self, token: &str) -> Result<Principal, AuthError> {
        if token.bytes().filter(|&b| b == b'.').count() != 2 {
            // Opaque tokens (API keys) are not accepted yet.
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
                challenge: HeaderValue::try_from(challenge).expect("validated above"),
                resource_metadata,
            }),
        })
    }
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
