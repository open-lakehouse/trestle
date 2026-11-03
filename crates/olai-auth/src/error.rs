use std::fmt;

/// The error type host-implemented traits ([`KeySource`], [`UserStatusStore`],
/// [`ApiKeyStore`]) return.
///
/// [`KeySource`]: crate::KeySource
/// [`UserStatusStore`]: crate::UserStatusStore
/// [`ApiKeyStore`]: crate::ApiKeyStore
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Why a request was not authenticated.
///
/// Each variant maps to one response from [`auth_layer`](crate::auth_layer): see
/// [`AuthError::status`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// The request carries no `Authorization: Bearer` header.
    #[error("missing bearer token")]
    Missing,
    /// A bearer token was presented but is not acceptable.
    #[error("invalid token: {0}")]
    InvalidToken(InvalidToken),
    /// The token is valid, but the user is banned.
    #[error("user is banned")]
    Banned,
    /// A key source, the user status store or the API key store failed, so the token
    /// could not be checked.
    #[error("authentication backend unavailable: {0}")]
    Unavailable(#[source] BoxError),
}

impl AuthError {
    /// The HTTP status [`auth_layer`](crate::auth_layer) responds with: 401 for a missing
    /// or invalid token, 403 for a banned user, 503 when a backend is unavailable.
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::Missing | Self::InvalidToken(_) => axum::http::StatusCode::UNAUTHORIZED,
            Self::Banned => axum::http::StatusCode::FORBIDDEN,
            Self::Unavailable(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<InvalidToken> for AuthError {
    fn from(reason: InvalidToken) -> Self {
        Self::InvalidToken(reason)
    }
}

/// The specific check a presented token failed.
///
/// The description is safe to return to the caller: it never contains the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidToken {
    /// Not a well-formed token: wrong segment count, bad base64 or JSON, too long, or
    /// an `olk_` API key of the wrong shape.
    Malformed,
    /// The token is neither a JWT nor, when API keys are accepted, an API key.
    UnsupportedType,
    /// The header's `alg` is not `EdDSA`.
    Algorithm,
    /// The header's `typ` is not the type the verifier requires.
    Type,
    /// The header has no `kid`.
    MissingKeyId,
    /// No signing key with the token's `kid`, even after a refresh.
    UnknownKey,
    /// The signature does not verify.
    Signature,
    /// The `iss` claim is not a configured issuer.
    Issuer,
    /// No `aud` value is in the issuer's allow-list.
    Audience,
    /// The token has expired, or has no `exp`.
    Expired,
    /// The token's `nbf` is in the future.
    NotYetValid,
    /// The user status store does not know the token's subject.
    UnknownUser,
    /// An API key that is unknown, no longer live, or whose secret does not match.
    ApiKey,
}

impl InvalidToken {
    fn description(self) -> &'static str {
        match self {
            Self::Malformed => "malformed token",
            Self::UnsupportedType => "unsupported token type",
            Self::Algorithm => "unsupported signing algorithm",
            Self::Type => "unexpected JWT type",
            Self::MissingKeyId => "token has no key id",
            Self::UnknownKey => "unknown signing key",
            Self::Signature => "invalid signature",
            Self::Issuer => "untrusted issuer",
            Self::Audience => "token is not for this audience",
            Self::Expired => "token expired",
            Self::NotYetValid => "token not yet valid",
            Self::UnknownUser => "unknown user",
            Self::ApiKey => "invalid API key",
        }
    }
}

impl fmt::Display for InvalidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}
