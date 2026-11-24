//! A worker's report records the step's result on its queue row and writes
//! the event the orchestrator acts on. Both commit together, or neither does:
//! if the engine stops before the event is written, the row is still the
//! worker's running claim, so the claim timeout runs the step again, and a
//! retried report still records the result and writes its event (#55). Once
//! they commit, the event is announced, so the orchestrator is woken for it.
//!
//! The engine is stopped by a trigger that makes the next event insert fail
//! once, as a stop just before the event is written would.
use kruxiaflow_core::activity::ActivityWorkerService;
use kruxiaflow_core::events::{Announcer, EventWake, Wake};
use kruxiaflow_core::queue::{Activity, ActivityQueue, PostgresQueue, QueueConfig};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Make the next insert into `workflow_events` fail, once. The sequence
/// isn't rolled back with the failed transaction, so the trigger fires only
/// the first time.
async fn stop_before_next_event(pool: &PgPool) {
    for statement in [
        "CREATE SEQUENCE stop_before_event",
        "CREATE FUNCTION stop_before_event() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF nextval('stop_before_event') = 1 THEN \
             RAISE EXCEPTION 'engine stopped'; \
           END IF; \
           RETURN NEW; \
         END $$",
        "CREATE TRIGGER stop_before_event BEFORE INSERT ON workflow_events \
         FOR EACH ROW EXECUTE FUNCTION stop_before_event()",
    ] {
        sqlx::query(statement).execute(pool).await.unwrap();
    }
}

/// A worker service, and the id of an activity `worker_01` has claimed
async fn claimed(pool: &PgPool) -> (ActivityWorkerService, Uuid, Uuid) {
    claimed_with(pool, Announcer::none()).await
}

/// [`claimed`], with a queue that announces through `announcer`
async fn claimed_with(pool: &PgPool, announcer: Announcer) -> (ActivityWorkerService, Uuid, Uuid) {
    let queue = Arc::new(
        PostgresQueue::new(pool.clone(), QueueConfig::default()).with_announcer(announcer),
    ) as Arc<dyn ActivityQueue>;
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
    let service = ActivityWorkerService::new(queue);
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
/// running; the retried report completes the row and writes the event
#[sqlx::test(migrations = "../migrations")]
async fn test_completion_stopped_before_its_event_is_not_recorded(pool: PgPool) {
    let (service, workflow_id, id) = claimed(&pool).await;
    stop_before_next_event(&pool).await;

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
/// running; the retried report fails the row and writes the event
#[sqlx::test(migrations = "../migrations")]
async fn test_failure_stopped_before_its_event_is_not_recorded(pool: PgPool) {
    let (service, workflow_id, id) = claimed(&pool).await;
    stop_before_next_event(&pool).await;
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

/// A retryable failure and its event commit together too: the requeued row
/// and its ActivityFailed (will_retry) event
#[sqlx::test(migrations = "../migrations")]
async fn test_retryable_failure_writes_its_event_with_the_requeue(pool: PgPool) {
    let (service, workflow_id, id) = claimed(&pool).await;
    stop_before_next_event(&pool).await;
    let fail = || {
        service.fail_activity(
            id,
            "worker_01".to_string(),
            "slipped".to_string(),
            "the belt slipped".to_string(),
            true,
            None,
            None,
        )
    };

    assert!(fail().await.is_err());
    assert_eq!(
        row(&pool, id).await.0,
        "running",
        "requeued without its event"
    );

    assert!(fail().await.unwrap(), "a retryable failure is retried");
    assert_eq!(row(&pool, id).await, ("pending".to_string(), None));
    let will_retry: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload->'will_retry' FROM workflow_events \
         WHERE workflow_id = $1 AND event_type = 'ActivityFailed'",
    )
    .bind(workflow_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(will_retry, vec![json!(true)]);
}

/// A completion's event, written in the completion's statement, wakes the
/// orchestrator once it commits
#[sqlx::test(migrations = "../migrations")]
async fn test_completion_wakes_the_orchestrator(pool: PgPool) {
    let wake = EventWake::connect(&pool).await.expect("LISTEN");
    let (service, _, id) = claimed_with(&pool, wake.announcer()).await;
    let mut rx = wake.subscribe();
    assert_eq!(rx.wait(Duration::from_millis(200)).await, Wake::TimedOut);

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
    assert_eq!(rx.wait(Duration::from_millis(500)).await, Wake::Woken);
}

/// A failure's event, written in the failure's transaction, wakes the
/// orchestrator once it commits
#[sqlx::test(migrations = "../migrations")]
async fn test_failure_wakes_the_orchestrator(pool: PgPool) {
    let wake = EventWake::connect(&pool).await.expect("LISTEN");
    let (service, _, id) = claimed_with(&pool, wake.announcer()).await;
    let mut rx = wake.subscribe();
    assert_eq!(rx.wait(Duration::from_millis(200)).await, Wake::TimedOut);

    service
        .fail_activity(
            id,
            "worker_01".to_string(),
            "jammed".to_string(),
            "the piston jammed".to_string(),
            false,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(rx.wait(Duration::from_millis(500)).await, Wake::Woken);
}
