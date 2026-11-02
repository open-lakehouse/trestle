use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

/// The authenticated caller of a request.
///
/// [`auth_layer`](crate::auth_layer) puts one into the request extensions. Handlers take
/// it as an extractor:
///
/// ```
/// use olai_auth::Principal;
///
/// async fn handler(principal: Principal) -> String {
///     format!("hello {}", principal.subject())
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Who the caller is.
    pub kind: PrincipalKind,
    /// The issuer that vouched for the caller.
    pub issuer: String,
    /// The scopes the credential grants. Empty when it carries none.
    pub scopes: Vec<String>,
    /// The OAuth client the token was issued to, if any.
    pub client_id: Option<String>,
    /// How the caller authenticated.
    pub auth_method: AuthMethod,
    /// Whether the caller is an administrator, according to the
    /// [`UserStatusStore`](crate::UserStatusStore).
    pub admin: bool,
}

/// The kind of caller behind a [`Principal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalKind {
    /// A human user.
    User {
        /// The issuer's stable user id (`sub`).
        sub: String,
        /// The user's email address, if the token has one.
        email: Option<String>,
        /// Whether the issuer has verified `email`.
        email_verified: bool,
    },
    /// A non-human caller, such as a CI job or another service.
    ServiceAccount {
        /// The service account's stable id.
        id: String,
        /// A display name.
        name: String,
    },
}

/// How a [`Principal`] authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthMethod {
    /// A JWT verified by a [`JwtVerifier`](crate::JwtVerifier).
    Jwt,
}

impl Principal {
    /// The caller's stable id: the user's `sub`, or the service account's id.
    pub fn subject(&self) -> &str {
        match &self.kind {
            PrincipalKind::User { sub, .. } => sub,
            PrincipalKind::ServiceAccount { id, .. } => id,
        }
    }

    /// Whether the caller is an administrator.
    pub fn is_admin(&self) -> bool {
        self.admin
    }

    /// Whether the credential grants `scope`.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// The rejection of the [`Principal`] extractor: the route is not behind
/// [`auth_layer`](crate::auth_layer).
///
/// It is a server misconfiguration rather than a client error, so it responds with 500
/// instead of letting the request through unauthenticated.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("no authenticated principal: the route is not behind auth_layer")]
pub struct MissingPrincipal;

impl IntoResponse for MissingPrincipal {
    fn into_response(self) -> Response {
        tracing::error!("{self}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Principal {
    type Rejection = MissingPrincipal;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Principal>()
            .cloned()
            .ok_or(MissingPrincipal)
    }
}
