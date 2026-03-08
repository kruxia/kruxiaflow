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

    /// Get events after a specific event ID, optionally filtered by workflow IDs.
    ///
    /// Used for WebSocket reconnection replay: clients track the last received event
    /// `id` (UUIDv7) and pass it on reconnect to resume from that exact checkpoint.
    /// Uses event ID (not timestamp) because UUIDv7 IDs are monotonically ordered on
    /// the PK index, giving an exact resume point with no duplicates or gaps — unlike
    /// timestamps where multiple events can share the same value.
    ///
    /// This is not intended for general historical queries (e.g., "last hour of events").
    /// That use case would be a separate REST endpoint with pagination.
    async fn get_events_from_id(
        &self,
        from_event_id: Uuid,
        workflow_ids: Option<&[Uuid]>,
        limit: i64,
    ) -> Result<Vec<WorkflowEvent>> {
        let _ = (from_event_id, workflow_ids, limit);
        Err(EventError::Invalid(
            "get_events_from_id not implemented".to_string(),
        ))
    }
}
