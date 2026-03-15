use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonClientError {
    #[error("request failed: {0}")]
    Transport(String),
    #[error("daemon returned status {status}: {message}")]
    Api { status: u16, message: String },
    #[error("failed to parse daemon response: {0}")]
    Decode(String),
}
