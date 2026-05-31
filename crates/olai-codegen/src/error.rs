pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Build error: {0}")]
    Build(String),

    #[error("bindings config is required when python, node, or node_ts output is enabled")]
    MissingBindingsConfig,

    #[error("Missing annotation for {object}: {message}")]
    MissingAnnotation { object: String, message: String },

    #[error("Invalid annotation for {object}: {message}")]
    InvalidAnnotation { object: String, message: String },

    #[error("Invalid models_path template `{template}`: {source}")]
    InvalidModelsPathTemplate {
        template: String,
        #[source]
        source: syn::Error,
    },

    #[error("Invalid Rust path `{path}` derived from proto message `{message}`: {source}")]
    InvalidRustPath {
        path: String,
        message: String,
        #[source]
        source: syn::Error,
    },

    #[error("Invalid error_type_path `{path}`: {source}")]
    InvalidErrorTypePath {
        path: String,
        #[source]
        source: syn::Error,
    },

    #[error("Invalid {field} `{path}`: {source}")]
    InvalidConfigPath {
        field: &'static str,
        path: String,
        #[source]
        source: syn::Error,
    },

    #[error(
        "Failed to parse generated tokens as a Rust file: {source}\n--- generated tokens ---\n{tokens}"
    )]
    GeneratedParse {
        tokens: String,
        #[source]
        source: syn::Error,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
