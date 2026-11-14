//! Bearer-token authentication and an authorization port for axum services.
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
//! - API keys (`olk_<id>_<secret>`) authenticate service accounts. An [`ApiKeyStore`]
//!   looks a key up by id, and the SHA-256 of the key is compared in constant time.
//!   `PgApiKeyStore` (feature `postgres`) reads them from Postgres.
//! - An [`Authorizer`] decides whether a principal may perform an action on a resource,
//!   wherever the Cedar policies are evaluated: `HttpAuthorizer` (feature `pdp-client`)
//!   asks a remote policy decision point, `CedarAuthorizer` (feature `cedar`) evaluates
//!   them in-process. Both fail closed. [`CachedAuthorizer`] reuses decisions for a few
//!   seconds, and [`AuthContext`] builds the standard request context.
//! - [`grants`]: `(subject, privilege, object)` tuples and the groups subjects are in,
//!   behind a [`GrantStore`] ([`MemoryGrantStore`]; `PgGrantStore` with feature
//!   `postgres`, over the tables `migrate` creates).
//!   `GrantsEntityLoader` (feature `cedar`) loads them as the entities a
//!   `CedarAuthorizer` decides on: the principal with its groups, and the privileges it
//!   holds on each entity of the request.
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

mod api_key;
mod authenticator;
mod authz;
mod error;
pub mod grants;
mod jwk;
mod jwt;
mod key_source;
mod layer;
mod metadata;
mod principal;
mod user_status;

#[cfg(feature = "postgres")]
mod migrate;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(test)]
mod test_support;

pub use api_key::{
    API_KEY_ISSUER, API_KEY_PREFIX, ApiKeyRecord, ApiKeyStore, api_key_digest, parse_api_key,
};
/// Re-exported for implementing [`KeySource`], [`UserStatusStore`] and [`ApiKeyStore`].
pub use async_trait::async_trait;
pub use authenticator::{Authenticator, AuthenticatorBuilder, ConfigError};
#[cfg(feature = "cedar")]
#[cfg_attr(docsrs, doc(cfg(feature = "cedar")))]
pub use authz::cedar::{CedarAuthorizer, CedarConfigError, DEFAULT_LOADER_ATTRS, EntityLoader};
#[cfg(feature = "pdp-client")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdp-client")))]
pub use authz::http::HttpAuthorizer;
pub use authz::wire;
pub use authz::wire::{AuthorizeRequest, AuthorizeResponse, Decision, EntityRef};
pub use authz::{AuthContext, Authorizer, CachedAuthorizer, EntityTypes};
pub use error::{AuthError, BoxError, InvalidToken};
#[cfg(feature = "cedar")]
#[cfg_attr(docsrs, doc(cfg(feature = "cedar")))]
pub use grants::loader::{GrantsEntityLoader, PrincipalAttrs, PrincipalFacts, SubjectTypes};
#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
pub use grants::postgres::PgGrantStore;
pub use grants::{
    Grant, GrantChange, GrantError, GrantStore, GroupMembership, MemoryGrantStore, ObjectRef,
    Subject, SubjectFacts, SubjectKind,
};
pub use jwk::{JwkError, JwkSet};
pub use jwt::{JwtClaims, JwtVerifier};
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub use key_source::HttpKeySource;
pub use key_source::{KeySource, PushKeySource};
pub use layer::auth_layer;
pub use metadata::{ProtectedResourceMetadata, WELL_KNOWN_PATH};
#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
pub use migrate::{MIGRATOR, migrate};
#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
pub use postgres::PgApiKeyStore;
pub use principal::{AuthMethod, MissingPrincipal, Principal, PrincipalKind};
pub use user_status::{UserStatus, UserStatusStore};
