//! A worker's report records the step's result on its queue row and
//! publishes the event the orchestrator acts on. Both commit together, or
//! neither does: if the engine stops before the event is written, the row
//! is still the worker's running claim, so the claim timeout runs the step
//! again, and a retried report still records and publishes.
//!
//! The engine is stopped by an event source that refuses the publish once.
use async_trait::async_trait;
use kruxiaflow_core::activity::ActivityWorkerService;
use kruxiaflow_core::events::{
    EventCursor, EventError, EventSource, NewWorkflowEvent, PostgresEventSource, WorkflowEvent,
};
use kruxiaflow_core::queue::{Activity, ActivityQueue, PostgresQueue, QueueConfig};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

/// An event source that refuses its first publish
struct RefuseFirst {
    inner: PostgresEventSource,
    refused: AtomicBool,
}

#[async_trait]
impl EventSource for RefuseFirst {
    async fn publish(&self, event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
        if !self.refused.swap(true, Ordering::SeqCst) {
            return Err(EventError::Invalid("engine stopped".to_string()));
        }
        self.inner.publish(event).await
    }

    async fn poll(&self, consumer_id: &str) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
        self.inner.poll(consumer_id).await
    }

    async fn update_position(
        &self,
        consumer_id: &str,
        cursor: EventCursor,
    ) -> kruxiaflow_core::events::Result<()> {
        self.inner.update_position(consumer_id, cursor).await
    }
}

/// A worker service whose first publish is refused, and the id of an
/// activity `worker_01` has claimed
async fn claimed(pool: &PgPool) -> (ActivityWorkerService, Uuid, Uuid) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let events = Arc::new(RefuseFirst {
        inner: PostgresEventSource::new(pool.clone()),
        refused: AtomicBool::new(false),
    }) as Arc<dyn EventSource>;
    let workflow_id = Uuid::now_v7();
    queue
        .schedule(
            workflow_id,
            vec![Activity {
                key: "pump".to_string(),
                worker: "std".to_string(),
                activity_name: "stroke".to_string(),
                parameters: json!({}),
                settings: None,
                scheduled_for: None,
                output_definitions: None,
                iteration: None,
                signal_data: None,
            }],
        )
        .await
        .unwrap();
    let service = ActivityWorkerService::new(queue, events);
    let activities = service
        .poll_activities("std", "worker_01", 1)
        .await
        .unwrap();
    (service, workflow_id, activities[0].id)
}

async fn row(pool: &PgPool, id: Uuid) -> (String, Option<String>) {
    sqlx::query_as("SELECT status::text, claimed_by FROM activity_queue WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn events(pool: &PgPool, workflow_id: Uuid, event_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_events WHERE workflow_id = $1 AND event_type::text = $2",
    )
    .bind(workflow_id)
    .bind(event_type)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A completion stopped before its event is written leaves the claim
/// running; the retried report completes the row and publishes the event
#[sqlx::test(migrations = "../migrations")]
async fn test_completion_stopped_before_its_event_is_not_recorded(pool: PgPool) {
    let (service, workflow_id, id) = claimed(&pool).await;

    let stopped = service
        .complete_activity(
            id,
            "worker_01".to_string(),
            json!({"strokes": 3}),
            None,
            None,
        )
        .await;
    assert!(stopped.is_err());
    assert_eq!(
        row(&pool, id).await,
        ("running".to_string(), Some("worker_01".to_string())),
        "the completion was recorded without its event"
    );
    assert_eq!(events(&pool, workflow_id, "ActivityCompleted").await, 0);

    service
        .complete_activity(
            id,
            "worker_01".to_string(),
            json!({"strokes": 3}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(row(&pool, id).await.0, "completed");
    assert_eq!(events(&pool, workflow_id, "ActivityCompleted").await, 1);
}

/// A permanent failure stopped before its event is written leaves the claim
/// running; the retried report fails the row and publishes the event
#[sqlx::test(migrations = "../migrations")]
async fn test_failure_stopped_before_its_event_is_not_recorded(pool: PgPool) {
    let (service, workflow_id, id) = claimed(&pool).await;
    let fail = || {
        service.fail_activity(
            id,
            "worker_01".to_string(),
            "jammed".to_string(),
            "the piston jammed".to_string(),
            false,
            None,
            None,
        )
    };

    assert!(fail().await.is_err());
    assert_eq!(
        row(&pool, id).await,
        ("running".to_string(), Some("worker_01".to_string())),
        "the failure was recorded without its event"
    );
    assert_eq!(events(&pool, workflow_id, "ActivityFailed").await, 0);

    assert!(!fail().await.unwrap(), "a permanent failure is not retried");
    assert_eq!(row(&pool, id).await.0, "failed");
    assert_eq!(events(&pool, workflow_id, "ActivityFailed").await, 1);
}
