/// A convenience type for declaring Results in the resource store.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by the resource store, association store, and secret manager.
///
/// Store operations report failures through these variants; the doc comments on
/// the [`ObjectStore`], [`AssociationStore`], and [`SecretManager`] trait methods
/// note which variant a given method returns and when.
///
/// [`ObjectStore`]: crate::ObjectStore
/// [`AssociationStore`]: crate::AssociationStore
/// [`SecretManager`]: crate::SecretManager
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Entity not found.")]
    NotFound,

    #[error("Entity already exists.")]
    AlreadyExists,

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Invalid identifier: {0}")]
    InvalidIdentifier(#[from] uuid::Error),

    #[error("Generic error: {0}")]
    Generic(String),

    #[error(transparent)]
    SerDe(#[from] serde_json::Error),
}

impl Error {
    /// Constructs an [`Error::Generic`] from a message.
    ///
    /// Use this for failures that do not map onto a more specific variant.
    pub fn generic(msg: impl Into<String>) -> Self {
        Self::Generic(msg.into())
    }

    /// Constructs an [`Error::InvalidArgument`] from a message.
    ///
    /// Use this when a caller-supplied argument is malformed or out of range.
    pub fn invalid_argument(msg: impl Into<String>) -> Self {
        Self::InvalidArgument(msg.into())
    }
}
