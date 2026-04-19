use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("signal parse failed: {0}")]
    SignalParse(String),

    #[error("pty error: {0}")]
    Pty(String),

    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("db pool error: {0}")]
    DbPool(#[from] r2d2::Error),

    #[error("io: {path} — {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("handoff invalid: {0}")]
    HandoffInvalid(String),

    #[error("project not found: {0}")]
    ProjectNotFound(uuid::Uuid),

    #[error("session not found: {0}")]
    SessionNotFound(uuid::Uuid),

    #[error("session not running: {0}")]
    SessionNotRunning(uuid::Uuid),

    #[error("ritual state error: {0}")]
    Ritual(String),

    #[error("config: {0}")]
    Config(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }
}
