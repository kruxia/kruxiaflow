//! A signal claims its step's wait and publishes `ActivitySignaled`. Both
//! commit together, or neither does: if the engine stops partway through,
//! the wait is still open and the signal can be sent again.
//!
//! The engine is stopped by dropping `deliver_signal` while its publish is
//! in progress, so no code after that point runs, as in an abrupt stop.
use async_trait::async_trait;
use kruxiaflow_core::events::{
    EventCursor, EventSource, NewWorkflowEvent, PostgresEventSource, WorkflowEvent,
};
use kruxiaflow_core::subscription::{
    NewSubscription, PostgresSubscriptionService, SignalRequest, SubscriptionService,
    deliver_signal,
};
use kruxiaflow_core::workflow::OnTimeout;
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// An event source whose publish never finishes
struct Stalls;

#[async_trait]
impl EventSource for Stalls {
    async fn publish(&self, _event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
        std::future::pending().await
    }

    async fn poll(
        &self,
        _consumer_id: &str,
    ) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
        Ok(Vec::new())
    }

    async fn update_position(
        &self,
        _consumer_id: &str,
        _cursor: EventCursor,
    ) -> kruxiaflow_core::events::Result<()> {
        Ok(())
    }
}

/// A running workflow whose `approve` step waits for `go`
async fn waiting_workflow(pool: &PgPool) -> Uuid {
    let def_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workflow_definitions (name, activities) VALUES ('w', '[]') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let workflow_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data) \
         VALUES ($1, 'w', $2, '{}', 'running', '{}', '{}')",
    )
    .bind(workflow_id)
    .bind(def_id)
    .execute(pool)
    .await
    .unwrap();
    PostgresSubscriptionService::new(pool.clone())
        .create_subscription(NewSubscription {
            workflow_id,
            activity_key: "approve".to_string(),
            event_name: "go".to_string(),
            on_timeout: OnTimeout::Fail,
            timeout_seconds: 300,
            iteration: None,
        })
        .await
        .unwrap();
    workflow_id
}

fn approve(workflow_id: Uuid) -> SignalRequest {
    SignalRequest {
        workflow_id,
        activity_key: "approve".to_string(),
        event_name: "go".to_string(),
        data: Some(json!({"approved": true})),
    }
}

/// A signal stopped partway through its publish leaves the wait open: the
/// signal sent again is accepted and its event is written
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_stopped_before_its_event_leaves_the_wait_open(pool: PgPool) {
    let workflow_id = waiting_workflow(&pool).await;
    let subs = PostgresSubscriptionService::new(pool.clone());

    let stopped = tokio::time::timeout(
        Duration::from_millis(200),
        deliver_signal(&subs, &Stalls, approve(workflow_id)),
    )
    .await;
    assert!(stopped.is_err(), "the signal was stopped partway through");

    let again = deliver_signal(
        &subs,
        &PostgresEventSource::new(pool.clone()),
        approve(workflow_id),
    )
    .await
    .unwrap();
    assert!(
        again.is_signaled(),
        "the signal sent again was refused: the stopped one claimed the wait"
    );
    let signaled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_events \
         WHERE workflow_id = $1 AND event_type = 'ActivitySignaled'",
    )
    .bind(workflow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(signaled, 1);
}
