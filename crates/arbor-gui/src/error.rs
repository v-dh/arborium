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
    #[error("{0}")]
    Other(String),
}

/// Errors from embedded terminal PTY operations.
#[derive(Debug, Error)]
pub(crate) enum TerminalError {
    #[error("{0}")]
    Pty(String),
    #[error("{0}")]
    LockPoisoned(&'static str),
}

/// Errors from connection address parsing and tunnel setup.
#[derive(Debug, Error)]
pub(crate) enum ConnectionError {
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Io(String),
}

/// Errors from GitHub API and OAuth operations.
#[derive(Debug, Error)]
pub(crate) enum GitHubError {
    #[error("{0}")]
    Api(String),
    #[error("{0}")]
    Auth(String),
}

/// Errors from local git repository operations.
#[derive(Debug, Error)]
pub(crate) enum GitError {
    #[error("{0}")]
    Operation(String),
}

/// Errors from prompt/agent command execution.
#[derive(Debug, Error)]
pub(crate) enum PromptError {
    #[error("{0}")]
    Execution(String),
}

/// Errors from external process launching.
#[derive(Debug, Error)]
pub(crate) enum LaunchError {
    #[error("{0}")]
    Failed(String),
}

/// Errors from home-directory and path resolution.
#[derive(Debug, Error)]
pub(crate) enum PathError {
    #[error("repository path cannot be empty")]
    EmptyPath,
    #[error("HOME environment variable is not set")]
    NoHomeDir,
}

/// Errors from config value parsing (theme, terminal backend).
#[derive(Debug, Error, PartialEq)]
pub(crate) enum ConfigParseError {
    #[error("{0}")]
    InvalidValue(String),
}

/// Errors from CLI argument parsing.
#[derive(Debug, Error)]
pub(crate) enum CliError {
    #[error("{0}")]
    InvalidArg(String),
}

/// Errors from running external daemon processes.
#[derive(Debug, Error)]
pub(crate) enum DaemonLaunchError {
    #[error("{0}")]
    Failed(String),
}

/// Errors from remote outpost provisioning over SSH.
#[derive(Debug, Error)]
pub(crate) enum OutpostError {
    #[error("SSH connection failed: {0}")]
    Connection(String),
    #[error("{0}")]
    Provisioning(String),
}

/// Errors from worktree creation, cloning, and lifecycle operations.
#[derive(Debug, Error)]
pub(crate) enum WorktreeError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    GitOperation(String),
    #[error("{0}")]
    Io(String),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error("{0}")]
    CommandFailed(String),
    #[error("{0}")]
    GitHub(String),
    #[error("{0}")]
    Script(String),
    #[error("{message}; rollback also failed: {rollback}")]
    ScriptWithRollbackFailure { message: String, rollback: String },
}
