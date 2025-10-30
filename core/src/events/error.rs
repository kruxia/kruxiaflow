use thiserror::Error;

#[derive(Error, Debug)]
pub enum EventError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Event not found: {0}")]
    NotFound(String),

    #[error("Invalid event: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, EventError>;
