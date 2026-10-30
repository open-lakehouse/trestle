//! Error type for this crate.

/// A `Result` defaulting to this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors raised while resolving principals or minting credentials.
///
/// Messages never include secret material.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// A location, scope or credential was rejected by the model.
    #[error(transparent)]
    Credentials(#[from] olai_storage_credentials::Error),

    /// No registered location covers the requested one.
    #[error("no registered location covers `{location}`")]
    NotFound {
        /// The requested location.
        location: String,
    },

    /// The requested location contains a more specific registered location that
    /// another credential governs, so no one credential may mint for all of it.
    #[error("`{location}` contains `{nested}`, which another credential governs")]
    Overlapping {
        /// The requested location.
        location: String,
        /// The registered location inside it.
        nested: String,
    },

    /// The minter cannot mint for this request, for example a location in another cloud.
    #[error("cannot mint for `{location}`: {reason}")]
    Unsupported {
        /// The requested location.
        location: String,
        /// Why not.
        reason: String,
    },

    /// The cloud call that mints the credential failed.
    #[error("minting failed: {source}")]
    Mint {
        /// The underlying failure.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl Error {
    /// Wraps a failure of the cloud call that mints a credential.
    pub fn mint(source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Mint {
            source: source.into(),
        }
    }

    pub(crate) fn unsupported(location: impl ToString, reason: impl Into<String>) -> Self {
        Self::Unsupported {
            location: location.to_string(),
            reason: reason.into(),
        }
    }
}

impl From<olai_http::Error> for Error {
    fn from(err: olai_http::Error) -> Self {
        Self::mint(err)
    }
}
