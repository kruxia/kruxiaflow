//! The activity queue's wake-up: which writes notify, to whom, and when
//! (docs/implementation/2026-11-08-activity-wake-sse.md §2.1, §2.2, §2.5).
//!
//! These tests listen through a real `LISTEN` connection. Notifications are
//! scoped to a database, so each `sqlx::test` database hears only its own.

use kruxiaflow_core::events::{ActivityWake, EventWake, Wake, WakeReceiver};
use kruxiaflow_core::queue::{
    Activity, ActivityQueue, ActivityResult, ActivitySettings, PostgresQueue, QueueConfig,
};
use kruxiaflow_core::workflow::{BackoffStrategy, RetryPolicy};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// Generous bound on notification latency (it is normally about 1 ms)
const WAKE_WITHIN: Duration = Duration::from_millis(1000);
/// How long to wait to be sure nothing arrives
const QUIET: Duration = Duration::from_millis(250);

async fn activity_wake(pool: &PgPool) -> ActivityWake {
    EventWake::connect(pool)
        .await
        .expect("LISTEN")
        .activity_wake()
}

fn queue(pool: &PgPool) -> PostgresQueue {
    PostgresQueue::new(pool.clone(), QueueConfig::default())
}

fn activity(worker: &str, key: &str) -> Activity {
    Activity {
        key: key.to_string(),
        worker: worker.to_string(),
        activity_name: "echo".to_string(),
        parameters: json!({}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
        iteration: None,
        signal_data: None,
    }
}

/// Up to two retries after the first attempt, 2 s apart, with the given
/// timeout
fn retrying(timeout_seconds: u64) -> ActivitySettings {
    ActivitySettings {
        timeout_seconds: Some(timeout_seconds),
        retry: Some(RetryPolicy {
            max_attempts: 3,
            strategy: BackoffStrategy::Fixed,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        ..Default::default()
    }
}

fn failure(message: &str) -> ActivityResult {
    ActivityResult {
        success: false,
        outputs: None,
        error: Some(message.to_string()),
        cost_usd: None,
        token_usage: None,
    }
}

fn success() -> ActivityResult {
    ActivityResult {
        success: true,
        outputs: None,
        error: None,
        cost_usd: None,
        token_usage: None,
    }
}

/// A time `seconds` from now by the database's clock, which is the clock the
/// queue compares `scheduled_for` against
async fn db_time_in(pool: &PgPool, seconds: i32) -> chrono::DateTime<chrono::Utc> {
    sqlx::query_scalar("SELECT now() + make_interval(secs => $1)")
        .bind(seconds as f64)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Consume wakes already delivered, so the next assertion sees only what
/// the next action causes
async fn drain(receiver: &mut WakeReceiver) {
    while receiver.wait(Duration::from_millis(100)).await == Wake::Woken {}
}

// =========================================================================
// Which writes notify
// =========================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_scheduling_wakes_that_worker_type_only(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut alpha = wake.subscribe("alpha");
    let mut beta = wake.subscribe("beta");

    queue(&pool)
        .schedule(Uuid::now_v7(), vec![activity("alpha", "a")])
        .await
        .unwrap();

    assert_eq!(alpha.wait(WAKE_WITHIN).await, Wake::Woken);
    assert_eq!(beta.wait(QUIET).await, Wake::TimedOut);
}

/// A row scheduled for later notifies when it is written, not when it comes
/// due: the wake makes the worker poll, and the poll's answer says when the
/// row is due (`next_due_in`, below)
#[sqlx::test(migrations = "../migrations")]
async fn test_scheduling_for_later_wakes_when_written(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");

    let mut delayed = activity("alpha", "a");
    delayed.scheduled_for = Some(db_time_in(&pool, 3600).await);
    queue(&pool)
        .schedule(Uuid::now_v7(), vec![delayed])
        .await
        .unwrap();

    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_claim_heartbeat_and_completion_do_not_wake(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");
    let queue = queue(&pool);

    queue
        .schedule(Uuid::now_v7(), vec![activity("alpha", "a")])
        .await
        .unwrap();
    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);
    drain(&mut receiver).await;

    let claimed = queue.claim_next("worker-1", "alpha", 1).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut, "claim");

    queue.heartbeat(claimed[0].id, "worker-1").await.unwrap();
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut, "heartbeat");

    queue
        .complete(claimed[0].id, "worker-1", success())
        .await
        .unwrap();
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut, "completion");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_retry_wakes_and_final_failure_does_not(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");
    let queue = queue(&pool);

    // A failure with retries left goes back to pending: wake
    let mut retryable = activity("alpha", "retryable");
    retryable.settings = Some(retrying(60));
    queue
        .schedule(Uuid::now_v7(), vec![retryable])
        .await
        .unwrap();
    let claimed = queue.claim_next("worker-1", "alpha", 1).await.unwrap();
    drain(&mut receiver).await;

    let will_retry = queue
        .fail(claimed[0].id, "worker-1", true, failure("try again"))
        .await
        .unwrap();
    assert!(will_retry);
    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);

    // A terminal failure leaves nothing to claim: no wake
    queue
        .schedule(Uuid::now_v7(), vec![activity("alpha", "terminal")])
        .await
        .unwrap();
    // The retry above is due in 2 s, so this claims the new activity
    let claimed = queue.claim_next("worker-1", "alpha", 1).await.unwrap();
    assert_eq!(claimed[0].activity_key, "terminal");
    drain(&mut receiver).await;

    let will_retry = queue
        .fail(claimed[0].id, "worker-1", false, failure("give up"))
        .await
        .unwrap();
    assert!(!will_retry);
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_stale_reclaim_wakes(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");
    let queue = queue(&pool);

    let mut short_timeout = activity("alpha", "a");
    short_timeout.settings = Some(retrying(1));
    queue
        .schedule(Uuid::now_v7(), vec![short_timeout])
        .await
        .unwrap();
    queue.claim_next("worker-1", "alpha", 1).await.unwrap();

    // Let the 1 s timeout pass with no heartbeat
    tokio::time::sleep(Duration::from_millis(1200)).await;
    drain(&mut receiver).await;

    let reclaimed = queue.reclaim_stale_activities(100).await.unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// A second schedule of the same activity inserts nothing, so it announces
/// nothing
#[sqlx::test(migrations = "../migrations")]
async fn test_idempotent_reschedule_does_not_wake(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");
    let queue = queue(&pool);
    let workflow_id = Uuid::now_v7();

    queue
        .schedule(workflow_id, vec![activity("alpha", "a")])
        .await
        .unwrap();
    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);
    drain(&mut receiver).await;

    queue
        .schedule(workflow_id, vec![activity("alpha", "a")])
        .await
        .unwrap();
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut);
}

// =========================================================================
// When the notification arrives
// =========================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_waits_for_commit_and_never_comes_after_rollback(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");

    let insert = "INSERT INTO activity_queue \
                  (workflow_id, activity_key, worker, name, parameters, timeout_duration) \
                  VALUES ($1, 'a', 'alpha', 'echo', '{}', interval '60 seconds')";

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(insert)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut, "uncommitted");
    tx.rollback().await.unwrap();
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut, "rolled back");

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(insert)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(receiver.wait(QUIET).await, Wake::TimedOut, "uncommitted");
    tx.commit().await.unwrap();
    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// The row is claimable by the time the wake arrives: nothing stands between
/// the commit and the claim (unlike the event cursor, which waits for older
/// transactions)
#[sqlx::test(migrations = "../migrations")]
async fn test_activity_is_claimable_when_the_wake_arrives(pool: PgPool) {
    let wake = activity_wake(&pool).await;
    let mut receiver = wake.subscribe("alpha");
    let queue = queue(&pool);

    // An unrelated transaction that has written and stays open must not
    // hold the claim back
    let mut unrelated = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO workflow_definitions (name, activities) VALUES ('open', '[]')")
        .execute(&mut *unrelated)
        .await
        .unwrap();

    queue
        .schedule(Uuid::now_v7(), vec![activity("alpha", "a")])
        .await
        .unwrap();
    assert_eq!(receiver.wait(WAKE_WITHIN).await, Wake::Woken);

    let claimed = queue.claim_next("worker-1", "alpha", 1).await.unwrap();
    assert_eq!(claimed.len(), 1);

    unrelated.rollback().await.unwrap();
}

// =========================================================================
// next_due_in: what a worker is told after an empty claim
// =========================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_next_due_in_reports_the_earliest_future_activity(pool: PgPool) {
    let queue = queue(&pool);

    // Nothing pending
    assert_eq!(queue.next_due_in("alpha").await.unwrap(), None);

    // Something due now is claimable, not "due later"
    queue
        .schedule(Uuid::now_v7(), vec![activity("alpha", "now")])
        .await
        .unwrap();
    assert_eq!(queue.next_due_in("alpha").await.unwrap(), None);

    // Two delayed activities: the earlier one counts
    for (key, seconds) in [("in-60", 60), ("in-30", 30)] {
        let mut delayed = activity("alpha", key);
        delayed.scheduled_for = Some(db_time_in(&pool, seconds).await);
        queue.schedule(Uuid::now_v7(), vec![delayed]).await.unwrap();
    }
    let due = queue
        .next_due_in("alpha")
        .await
        .unwrap()
        .expect("a delayed activity is pending");
    assert!(
        due > Duration::from_secs(25) && due <= Duration::from_secs(30),
        "expected just under 30 s, got {due:?}"
    );

    // Another worker type has nothing
    assert_eq!(queue.next_due_in("beta").await.unwrap(), None);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_next_due_in_covers_a_retry_backoff(pool: PgPool) {
    let queue = queue(&pool);

    let mut retryable = activity("alpha", "a");
    retryable.settings = Some(retrying(60));
    queue
        .schedule(Uuid::now_v7(), vec![retryable])
        .await
        .unwrap();
    let claimed = queue.claim_next("worker-1", "alpha", 1).await.unwrap();

    // Running: nothing is pending
    assert_eq!(queue.next_due_in("alpha").await.unwrap(), None);

    assert!(
        queue
            .fail(claimed[0].id, "worker-1", true, failure("try again"))
            .await
            .unwrap()
    );

    // Back to pending, due after the 2 s backoff
    let due = queue
        .next_due_in("alpha")
        .await
        .unwrap()
        .expect("the retry is pending");
    assert!(
        due > Duration::from_millis(500) && due <= Duration::from_secs(2),
        "expected just under 2 s, got {due:?}"
    );

    // And it is not claimable before then
    assert!(
        queue
            .claim_next("worker-1", "alpha", 1)
            .await
            .unwrap()
            .is_empty()
    );
}
