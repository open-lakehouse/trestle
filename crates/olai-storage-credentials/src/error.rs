//! Error type for this crate.

/// A `Result` defaulting to this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors raised while parsing locations, credentials or wire payloads.
///
/// Messages never include secret material: a malformed credential is described by
/// which field is wrong, not by its value.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// The URL is not a storage location this crate understands.
    #[error("invalid storage location `{url}`: {reason}")]
    InvalidLocation {
        /// The offending URL.
        url: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A credential is malformed or incomplete.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),

    /// A JSON document could not be encoded or decoded.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub(crate) fn invalid_location(url: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidLocation {
            url: url.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_credential(reason: impl Into<String>) -> Self {
        Self::InvalidCredential(reason.into())
    }
}
