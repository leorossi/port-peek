/// Errors surfaced by the library and the command-line interface.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this platform is not supported yet (supported: macOS, Linux)")]
    UnsupportedPlatform,
    #[error("{0}")]
    Backend(String),
    #[error("failed to serialize JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to write output: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
