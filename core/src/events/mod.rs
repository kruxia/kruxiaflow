pub mod error;
pub mod models;
pub mod postgres_event_source;

pub use error::{EventError, Result};
pub use models::*;
pub use postgres_event_source::PostgresEventSource;

use async_trait::async_trait;
use uuid::Uuid;

/// Event Source interface for publishing and consuming workflow events
#[async_trait]
pub trait EventSource: Send + Sync {
    /// Publish a workflow event to the event stream
    async fn publish(&self, event: NewWorkflowEvent) -> Result<()>;

    /// Poll for new events since last consumed position
    async fn poll(&self, consumer_id: &str) -> Result<Vec<WorkflowEvent>>;

    /// Update consumer position after successfully processing events
    async fn update_position(&self, consumer_id: &str, last_event_id: Uuid) -> Result<()>;
}
