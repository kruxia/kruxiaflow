pub mod error;
pub mod models;
pub mod postgres_event_source;
pub mod rebase;
pub mod wake;

pub use error::{EventError, Result};
pub use models::*;
pub use postgres_event_source::PostgresEventSource;
pub use rebase::{
    ForeignXids, RebaseReport, find_foreign_xids, rebase_event_xids, verify_event_xids,
};
pub use wake::{ActivityWake, EventWake, EventWakeMode, Wake, WakeReceiver};

use async_trait::async_trait;
use uuid::Uuid;

/// Event Source interface for publishing and consuming workflow events
#[async_trait]
pub trait EventSource: Send + Sync {
    /// Publish a workflow event to the event stream
    async fn publish(&self, event: NewWorkflowEvent) -> Result<()>;

    /// Poll for new events since the consumer's last stored cursor.
    ///
    /// Returns events in cursor order. Every event returned is final: no
    /// event can later become visible behind the last one returned, so a
    /// consumer may store that event's cursor once it has processed the batch.
    async fn poll(&self, consumer_id: &str) -> Result<Vec<WorkflowEvent>>;

    /// Store the consumer's cursor after successfully processing events
    /// (forward-only: a cursor behind the stored one is ignored)
    async fn update_position(&self, consumer_id: &str, cursor: EventCursor) -> Result<()>;

    /// Get events after a specific event ID, optionally filtered by workflow IDs.
    ///
    /// Used for WebSocket reconnection replay: clients track the last received event
    /// `id` and pass it on reconnect to resume from that exact checkpoint. The id is
    /// resolved to the event's cursor, so the replay has the same no-gap guarantee
    /// as `poll`. An unknown id yields an empty replay.
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

    /// A wake-up signal that fires when new events may have committed.
    ///
    /// `None` (the default) means consumers poll on their backoff timer alone.
    /// A wake is only a hint: consumers must poll on the timer as well and be
    /// correct without it.
    fn wake(&self) -> Option<WakeReceiver> {
        None
    }
}
