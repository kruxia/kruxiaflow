use thiserror::Error;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, QueueError>;

#[derive(Error, Debug)]
pub enum QueueError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),

    #[error("Activity not found: {0}")]
    ActivityNotFound(Uuid),

    #[error("Activity reclaimed by another worker")]
    ActivityReclaimed,

    #[error("Invalid status: expected {expected}, actual {actual}")]
    InvalidStatus { expected: String, actual: String },

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}
