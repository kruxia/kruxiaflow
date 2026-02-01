use thiserror::Error;

#[derive(Error, Debug)]
pub enum OrchestratorError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Event error: {0}")]
    Event(#[from] crate::events::EventError),

    #[error("Queue error: {0}")]
    Queue(#[from] crate::queue::QueueError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Workflow definition not found: {0}")]
    WorkflowDefinitionNotFound(String),

    #[error("Activity not found: {0}")]
    ActivityNotFound(String),

    #[error("State deserialization error: {0}")]
    StateDeserialization(String),

    #[error("State serialization error: {0}")]
    StateSerialization(String),

    #[error("Missing activity key in event")]
    MissingActivityKey,

    #[error("Invalid condition expression: {0}")]
    InvalidCondition(String),

    #[error("Template resolution failed: {0}")]
    TemplateFailed(String),

    #[error("Invalid event: {0}")]
    InvalidEvent(String),

    #[error("Cost tracking failed: {0}")]
    CostTrackingFailed(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

pub type Result<T> = std::result::Result<T, OrchestratorError>;
