use thiserror::Error;

pub type QueriaResult<T> = Result<T, QueriaError>;

#[derive(Debug, Error)]
pub enum QueriaError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("authentication failed")]
    Authentication,
    #[error("permission denied")]
    PermissionDenied,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("infrastructure error: {0}")]
    Infrastructure(String),
}
