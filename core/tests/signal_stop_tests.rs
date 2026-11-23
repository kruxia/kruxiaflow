//! A signal claims its step's wait and writes `ActivitySignaled`. Both commit
//! together, or neither does: if the engine stops partway through, the wait
//! is still open and the signal can be sent again (#56).
//!
//! The engine is stopped by dropping `deliver_signal` while the event insert
//! is in progress (a trigger holds the first one), so no code after that
//! point runs, as in an abrupt stop.
use kruxiaflow_core::subscription::{
    NewSubscription, PostgresSubscriptionService, SignalRequest, SubscriptionService,
    deliver_signal,
};
use kruxiaflow_core::workflow::OnTimeout;
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// Hold the next insert into `workflow_events` for two seconds, once. The
/// sequence isn't rolled back, so only the first insert is held.
async fn hold_next_event(pool: &PgPool) {
    for statement in [
        "CREATE SEQUENCE hold_next_event",
        "CREATE FUNCTION hold_next_event() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF nextval('hold_next_event') = 1 THEN PERFORM pg_sleep(2); END IF; \
           RETURN NEW; \
         END $$",
        "CREATE TRIGGER hold_next_event BEFORE INSERT ON workflow_events \
         FOR EACH ROW EXECUTE FUNCTION hold_next_event()",
    ] {
        sqlx::query(statement).execute(pool).await.unwrap();
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

/// A signal stopped partway through writing its event leaves the wait open:
/// the signal sent again is taken and its event is written
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_stopped_before_its_event_leaves_the_wait_open(pool: PgPool) {
    let workflow_id = waiting_workflow(&pool).await;
    let subs = PostgresSubscriptionService::new(pool.clone());
    hold_next_event(&pool).await;

    let stopped = tokio::time::timeout(
        Duration::from_millis(200),
        deliver_signal(&subs, approve(workflow_id)),
    )
    .await;
    assert!(stopped.is_err(), "the signal was stopped partway through");

    let again = deliver_signal(&subs, approve(workflow_id)).await.unwrap();
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
