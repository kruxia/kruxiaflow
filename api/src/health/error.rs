use thiserror::Error;

/// Errors that can occur during health checks
#[derive(Debug, Error)]
pub enum HealthCheckError {
    /// Database connectivity error
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    /// Event source connectivity error
    #[error("Event source error: {0}")]
    EventSourceError(String),

    /// Activity queue connectivity error
    #[error("Activity queue error: {0}")]
    QueueError(String),

    /// Health check timed out
    #[error("Health check timeout")]
    Timeout,

    /// Unexpected result from health check
    #[error("Unexpected result from health check")]
    UnexpectedResult,
}

/// Result type for health check operations
pub type Result<T> = std::result::Result<T, HealthCheckError>;
