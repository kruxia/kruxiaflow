use super::{EventSource, NewWorkflowEvent, Result, WorkflowEvent, WorkflowEventType};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-based event source implementation
pub struct PostgresEventSource {
    pool: PgPool,
}

impl PostgresEventSource {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventSource for PostgresEventSource {
    /// Publish a workflow event to the event stream
    /// - Idempotent via UNIQUE(workflow_id, event_type, activity_key)
    /// - Database auto-generates id (UUIDv7) and timestamp
    #[tracing::instrument(skip(self), level = "debug")]
    async fn publish(&self, event: NewWorkflowEvent) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO workflow_events (workflow_id, event_type, activity_key, payload, iteration)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (workflow_id, event_type, activity_key, iteration) DO NOTHING
            "#,
            event.workflow_id,
            event.event_type as WorkflowEventType,
            event.activity_key,
            event.payload,
            event.iteration
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Poll for new events since last consumed position
    /// - Returns up to 100 events per poll
    /// - Uses scalar subquery for consumer position (enables Index Range Scan)
    /// - If no checkpoint exists (first poll), returns events from beginning
    ///
    /// Performance: The scalar subquery is evaluated ONCE and the result is used
    /// as a constant in the Index Condition, allowing PostgreSQL to use efficient
    /// Index Range Scan on idx_events_consumer_poll covering index.
    ///
    /// The nil UUID fallback (when no checkpoint exists) works because UUIDv7s are
    /// time-ordered and all come after the nil UUID lexicographically.
    #[tracing::instrument(skip(self), level = "debug")]
    async fn poll(&self, consumer_id: &str) -> Result<Vec<WorkflowEvent>> {
        let events = sqlx::query_as!(
            WorkflowEvent,
            r#"
            SELECT id, workflow_id, event_type as "event_type: WorkflowEventType",
                   activity_key, payload, timestamp, iteration
            FROM workflow_events
            WHERE id > COALESCE(
                (SELECT last_event_id FROM workflow_event_consumers WHERE consumer_id = $1),
                '00000000-0000-0000-0000-000000000000'::uuid
            )
            ORDER BY id ASC
            LIMIT 100
            "#,
            consumer_id
        )
        .fetch_all(&self.pool)
        .await?;

        tracing::debug!(event_count = events.len(), "Polled events");

        Ok(events)
    }

    /// Update consumer position after successfully processing events
    /// - Upserts consumer position
    /// - Only moves position forward (prevents backwards movement)
    /// - Safe for concurrent orchestrators (WHERE clause enforces forward-only)
    async fn update_position(&self, consumer_id: &str, last_event_id: Uuid) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO workflow_event_consumers (consumer_id, last_event_id, updated_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (consumer_id)
            DO UPDATE SET
                last_event_id = $2,
                updated_at = NOW()
            WHERE workflow_event_consumers.last_event_id < $2
            "#,
            consumer_id,
            last_event_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
