use thiserror::Error;

/// Errors from process lifecycle management.
#[derive(Debug, Error)]
pub(crate) enum ProcessError {
    #[error("process `{0}` is not defined")]
    NotDefined(String),
    #[error("process `{0}` is not tracked")]
    NotTracked(String),
    #[error("process `{0}` not found")]
    NotFound(String),
    #[error("process name `{0}` is ambiguous, use a process id instead")]
    AmbiguousName(String),
    #[error("{0}")]
    Daemon(String),
}

/// Errors from task scheduling and execution.
#[derive(Debug, Error)]
pub(crate) enum TaskError {
    #[error("task `{0}` not found")]
    NotFound(String),
    #[error("task `{0}` is already running")]
    AlreadyRunning(String),
    #[error("failed to spawn agent `{name}`: {reason}")]
    SpawnFailed { name: String, reason: String },
}

/// Errors from GitHub/GitLab issue provider operations.
#[derive(Debug, Error)]
pub(crate) enum IssueProviderError {
    #[error("invalid repository slug `{0}`")]
    InvalidSlug(String),
    #[error("{context}: {reason}")]
    ApiRequest { context: String, reason: String },
    #[error("{provider} returned {status}: {body}")]
    ApiStatus {
        provider: String,
        status: u16,
        body: String,
    },
    #[error("{0}")]
    Other(String),
}

/// Errors from managed worktree path resolution.
#[derive(Debug, Error)]
pub(crate) enum ManagedWorktreeError {
    #[error("repository root has no terminal directory name")]
    NoTerminalDir,
    #[error("worktree name contains no usable characters")]
    EmptyName,
    #[error("user home directory environment variables are not set")]
    NoHomeDir,
}

/// Errors from repository store operations.
#[derive(Debug, Error)]
pub(crate) enum RepositoryStoreError {
    #[error("failed to read repository store `{path}`: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse repository store `{path}` as JSON: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
}

/// Errors from WebSocket client message processing.
#[derive(Debug, Error)]
pub(crate) enum WsClientError {
    #[error("invalid websocket payload: {0}")]
    InvalidPayload(String),
    #[error("invalid signal, expected interrupt|terminate|kill")]
    InvalidSignal,
    #[error("{0}")]
    Daemon(String),
}

/// Errors from route-level operations.
#[derive(Debug, Error)]
pub(crate) enum RouteError {
    #[error("{0}")]
    Internal(String),
}
