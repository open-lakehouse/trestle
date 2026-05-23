//! Error type for the trestle crate.

use std::path::PathBuf;

/// Result alias used throughout the trestle crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Top-level error for trestle operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("I/O error: {0}")]
    PlainIo(#[from] std::io::Error),

    #[error("YAML parse error in {path}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("YAML error: {0}")]
    PlainYaml(#[from] serde_yaml::Error),

    #[error("template `{name}` not found")]
    TemplateNotFound { name: String },

    #[error("component `{name}` not found")]
    ComponentNotFound { name: String },

    #[error("profile `{name}` not found in template `{template}`")]
    ProfileNotFound { template: String, name: String },

    #[error("variable `{name}` is required but was not provided")]
    MissingVariable { name: String },

    #[error("invalid value for variable `{name}`: {reason}")]
    InvalidVariable { name: String, reason: String },

    #[error("template rendering failed in {file}: {source}")]
    Render {
        file: PathBuf,
        #[source]
        source: minijinja::Error,
    },

    #[error("template engine error: {0}")]
    PlainRender(#[from] minijinja::Error),

    #[error("manifest validation failed: {0}")]
    Manifest(String),

    #[error("component dependency cycle detected involving `{0}`")]
    DependencyCycle(String),

    #[error("git operation failed: {0}")]
    Git(String),

    #[error("output directory `{0}` already exists and is not empty")]
    OutputExists(PathBuf),

    #[error("post-init hook failed: {0}")]
    Hook(String),

    #[error("codegen error: {0}")]
    Codegen(#[from] olai_codegen::Error),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }

    pub fn io_at(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn yaml_at(path: impl Into<PathBuf>, source: serde_yaml::Error) -> Self {
        Self::Yaml {
            path: path.into(),
            source,
        }
    }
}
