//! The crate's [`Error`].

/// What can go wrong reading or writing the [`Store`](crate::Store).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite failed.
    #[error("state store: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// An attached store's database failed, or couldn't be reached.
    #[error("shared state store: {0}")]
    Remote(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A stored column didn't hold what the store wrote there.
    #[error("state store: {0}")]
    Column(String),
    /// A shared database has no trestle state yet.
    #[error("the database holds no trestle state yet: initialize it first (`trestle state init`)")]
    Uninitialized,
    /// A shared database's schema is older than this version writes. Opening one never
    /// migrates it, so that one colleague's upgrade doesn't lock out everyone else's older
    /// version unasked.
    #[error(
        "the shared state store's schema version {found} is older than this version's \
         ({current}): migrate it (`trestle state init`) once everyone can upgrade"
    )]
    Outdated {
        /// The database's schema version.
        found: u32,
        /// The version this crate writes.
        current: u32,
    },
    /// The environment's draft was edited since the caller read it, so its edit would
    /// overwrite someone else's (see [`Store::update_draft_if`](crate::Store::update_draft_if)).
    #[error(
        "the draft of environment `{name}` changed since it was read (version {expected}, now \
         {current}): reload it and edit again"
    )]
    Conflict {
        /// The environment's name.
        name: String,
        /// The draft version the caller edited.
        expected: u64,
        /// Its version now.
        current: u64,
    },
    /// The store was written by a newer version than this one understands.
    #[error(
        "state store schema version {found} is newer than this version supports ({supported}); \
         upgrade trestle"
    )]
    UnsupportedVersion {
        /// The store's schema version.
        found: u32,
        /// The newest version this crate reads.
        supported: u32,
    },
    /// A name is not valid (see [`is_valid_name`](olai_stack_topology::workspace::is_valid_name)).
    #[error(
        "`{0}` is not a valid name: use lowercase letters, digits and dashes, starting with a letter"
    )]
    InvalidName(String),
    /// Something already exists under that name.
    #[error("{kind} `{name}` already exists")]
    Duplicate {
        /// What kind of thing (`workspace`, `environment`).
        kind: &'static str,
        /// Its name.
        name: String,
    },
    /// No such thing.
    #[error("no {kind} `{id}`")]
    NotFound {
        /// What kind of thing.
        kind: &'static str,
        /// Its id or name.
        id: String,
    },
    /// A workspace or environment can't be renamed (or its mesh changed) while it has
    /// instances: their compose projects, volumes, hostnames and mesh network are named after
    /// it, so the change would strand what Docker already holds under the old name.
    ///
    /// For an attached store, only this machine's instances are counted: a colleague's are
    /// out of sight.
    #[error(
        "{kind} `{name}` has {instances} instance{}: remove {} first",
        if *instances == 1 { "" } else { "s" },
        if *instances == 1 { "it" } else { "them" }
    )]
    HasInstances {
        /// What kind of thing (`workspace`, `environment`).
        kind: &'static str,
        /// Its name.
        name: String,
        /// How many instances it has.
        instances: usize,
    },
    /// Every slot is leased.
    #[error("no free slot: the host runs at most {} instances", olai_stack_topology::MAX_SLOT + 1)]
    SlotsExhausted,
    /// An environment manifest didn't (de)serialize.
    #[error(transparent)]
    Manifest(#[from] olai_stack_topology::ManifestError),
    /// A workspace manifest didn't (de)serialize.
    #[error(transparent)]
    Workspace(#[from] olai_stack_topology::WorkspaceError),
    /// A stored JSON column didn't (de)serialize.
    #[error("state store JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Reading or writing an import/export directory failed.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: std::path::PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// An export can't be written as asked.
    #[error("can't export: {0}")]
    Export(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<std::path::PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

/// The crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
