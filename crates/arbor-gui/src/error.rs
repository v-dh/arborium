use thiserror::Error;

/// Errors from JSON file store operations (config, repository store, issue cache, etc.)
#[derive(Debug, Error)]
pub(crate) enum StoreError {
    #[error("failed to read `{path}`: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write `{path}`: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to create directory `{path}`: {source}")]
    CreateDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse `{path}`: {source}")]
    JsonParse {
        path: String,
        source: serde_json::Error,
    },
    #[error("failed to serialize data for `{path}`: {source}")]
    JsonSerialize {
        path: String,
        source: serde_json::Error,
    },
    #[allow(dead_code)]
    #[error("failed to parse `{path}` as TOML: {source}")]
    TomlParse {
        path: String,
        source: toml_edit::TomlError,
    },
    #[allow(dead_code)]
    #[error("{0}")]
    Other(String),
}
