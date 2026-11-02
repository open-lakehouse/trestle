//! Bearer-token authentication for axum services.
//!
//! The crate verifies the `Authorization: Bearer` header of each request and hands
//! handlers a [`Principal`]:
//!
//! - [`JwtVerifier`] checks one issuer's tokens: EdDSA (Ed25519) signatures only, an
//!   exact `iss`, an `aud` allow-list and `exp`/`nbf` with a small leeway.
//! - Signing keys come from a [`KeySource`], cached by `kid`. An unknown `kid` triggers a
//!   rate-limited refresh, which picks up rotated keys. [`PushKeySource`] takes keys the
//!   host fetched itself; [`HttpKeySource`] (feature `http`) fetches a JWKS URL.
//! - A [`UserStatusStore`] supplies whether a user is banned or an admin. The lookup is
//!   required and cached for a short TTL. Token claims are never used for either.
//! - [`auth_layer`] runs an [`Authenticator`] in front of a router. Rejected requests get
//!   a 401 with an RFC 6750 `WWW-Authenticate` challenge whose `resource_metadata`
//!   parameter (RFC 9728) points clients at the authorization server.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use axum::{Router, routing::get};
//! use olai_auth::{
//!     Authenticator, BoxError, JwtVerifier, Principal, ProtectedResourceMetadata,
//!     PushKeySource, UserStatus, UserStatusStore, auth_layer,
//! };
//!
//! struct Users;
//!
//! #[olai_auth::async_trait]
//! impl UserStatusStore for Users {
//!     async fn status(&self, _issuer: &str, _sub: &str) -> Result<Option<UserStatus>, BoxError> {
//!         // Look the user up in the identity provider's user table.
//!         Ok(Some(UserStatus::from_role(Some("user"), false)))
//!     }
//! }
//!
//! async fn whoami(principal: Principal) -> String {
//!     principal.subject().to_string()
//! }
//!
//! # fn build(jwks: &[u8]) -> Result<Router, Box<dyn std::error::Error>> {
//! let keys = PushKeySource::new();
//! keys.set_jwks(jwks)?;
//!
//! let issuer = "https://auth.example.com";
//! let verifier = JwtVerifier::new(issuer, keys)
//!     .with_audience(issuer)
//!     .with_audience("https://api.example.com")
//!     // Pushed keys are local, so a refresh costs nothing.
//!     .with_min_refresh_interval(Duration::ZERO);
//! let authenticator = Authenticator::builder(Users)
//!     .issuer(verifier)
//!     .resource_metadata_url("https://api.example.com/.well-known/oauth-protected-resource")
//!     .build()?;
//!
//! let protected = auth_layer(Router::new().route("/whoami", get(whoami)), authenticator);
//! // Public routes are merged outside the layer.
//! let app = Router::new()
//!     .route("/health", get(|| async { "ok" }))
//!     .merge(ProtectedResourceMetadata::new("https://api.example.com", issuer).router())
//!     .merge(protected);
//! # Ok(app)
//! # }
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

mod authenticator;
mod error;
mod jwk;
mod jwt;
mod key_source;
mod layer;
mod metadata;
mod principal;
mod user_status;

#[cfg(test)]
mod test_support;

/// Re-exported for implementing [`KeySource`] and [`UserStatusStore`].
pub use async_trait::async_trait;
pub use authenticator::{Authenticator, AuthenticatorBuilder, ConfigError};
pub use error::{AuthError, BoxError, InvalidToken};
pub use jwk::{JwkError, JwkSet};
pub use jwt::{JwtClaims, JwtVerifier};
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub use key_source::HttpKeySource;
pub use key_source::{KeySource, PushKeySource};
pub use layer::auth_layer;
pub use metadata::{ProtectedResourceMetadata, WELL_KNOWN_PATH};
pub use principal::{AuthMethod, MissingPrincipal, Principal, PrincipalKind};
pub use user_status::{UserStatus, UserStatusStore};
