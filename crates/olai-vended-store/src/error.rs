//! Error type for this crate.

use olai_storage_credentials::AccessLevel;

/// A `Result` defaulting to this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors raised while vending credentials or building stores from them.
///
/// Messages never include secret material.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// A credential, location or wire payload was rejected by the model.
    #[error(transparent)]
    Credentials(#[from] olai_storage_credentials::Error),

    /// The vending server, or the transport to it, failed.
    #[error("credential vending failed: {source}")]
    Vend {
        /// The underlying failure.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// The credential is of a kind the requested store cannot use, for example an
    /// Azure SAS for an S3 store.
    #[error("expected {expected} credentials, got {actual}")]
    CredentialKind {
        /// The kind the store needs.
        expected: &'static str,
        /// The kind the credential carries.
        actual: &'static str,
    },

    /// A refreshed credential no longer covers what the store was built for.
    ///
    /// A source replays the request it was bound to, so this means the server
    /// narrowed its grant. Using the credential would fail requests the store accepts.
    #[error(
        "refreshed credential for `{scope}` does not cover `{expected}` with {access:?} access"
    )]
    RefreshNarrowed {
        /// The scope the store was built for.
        expected: String,
        /// The scope of the refreshed credential.
        scope: String,
        /// The access level the store was built for.
        access: AccessLevel,
    },
}

impl Error {
    /// Wrap a failure from a vending server or its transport.
    pub fn vend(source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Vend {
            source: source.into(),
        }
    }
}

impl From<Error> for object_store::Error {
    fn from(err: Error) -> Self {
        object_store::Error::Generic {
            store: "VendedStore",
            source: Box::new(err),
        }
    }
}
