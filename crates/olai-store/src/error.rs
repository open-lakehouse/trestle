/// A convenience type for declaring Results in the resource store.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by the resource store and association store.
///
/// Store operations report failures through these variants; the doc comments on
/// the [`ObjectStore`] and [`AssociationStore`] trait methods note which variant
/// a given method returns and when.
///
/// [`ObjectStore`]: crate::ObjectStore
/// [`AssociationStore`]: crate::AssociationStore
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Entity not found.")]
    NotFound,

    #[error("Entity already exists.")]
    AlreadyExists,

    #[error("Precondition failed: the object was modified concurrently.")]
    Conflict,

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

    /// A stable, low-cardinality kind string for use as a `tracing` / OpenTelemetry
    /// `error.type` field.
    ///
    /// Returns a `&'static str` and never includes an inner message, so recording it on a
    /// span cannot leak the caller-supplied text carried by [`InvalidArgument`](Self::InvalidArgument)
    /// or [`Generic`](Self::Generic).
    pub(crate) fn kind_str(&self) -> &'static str {
        match self {
            Error::NotFound => "not_found",
            Error::AlreadyExists => "already_exists",
            Error::Conflict => "conflict",
            Error::InvalidArgument(_) => "invalid_argument",
            Error::InvalidIdentifier(_) => "invalid_identifier",
            Error::Generic(_) => "generic",
            Error::SerDe(_) => "serde",
        }
    }
}
