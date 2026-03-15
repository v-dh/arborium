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

/// Errors from process memory metrics collection.
#[derive(Debug, Error)]
pub(crate) enum ProcessMetricsError {
    #[error("failed to list daemon sessions: {0}")]
    DaemonListSessions(String),
}

/// Errors from git commit operations.
#[derive(Debug, Error)]
pub(crate) enum GitCommitError {
    #[error("nothing to commit")]
    NothingToCommit,
    #[error("failed to open repository at `{path}`: {source}")]
    OpenRepository { path: String, source: git2::Error },
    #[error("failed to read index: {0}")]
    ReadIndex(git2::Error),
    #[error("failed to stage changes: {0}")]
    StageChanges(git2::Error),
    #[error("failed to update index: {0}")]
    UpdateIndex(git2::Error),
    #[error("failed to write index: {0}")]
    WriteIndex(git2::Error),
    #[error("failed to write tree: {0}")]
    WriteTree(git2::Error),
    #[error("failed to find tree: {0}")]
    FindTree(git2::Error),
    #[error("failed to create signature: {0}")]
    CreateSignature(git2::Error),
    #[error("failed to create commit: {0}")]
    CreateCommit(git2::Error),
}

/// Errors from TLS certificate generation and loading.
///
/// Note: `tls.rs` is not currently compiled (`mod tls` is absent from
/// `main.rs`), so rcgen/rustls error types are stored as `String` to avoid
/// adding unused dependencies.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub(crate) enum TlsError {
    #[error("failed to create certs directory: {0}")]
    CreateCertDir(std::io::Error),
    #[error("{context}: {reason}")]
    CertGeneration { context: String, reason: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("parse certs: {0}")]
    ParseCerts(std::io::Error),
    #[error("parse private key: {0}")]
    ParsePrivateKey(std::io::Error),
    #[error("no private key found in PEM file")]
    NoPrivateKey,
    #[error("build rustls ServerConfig: {0}")]
    BuildServerConfig(String),
}
