//! A signal wait ends once: by one signal or by its timeout (#33).
//!
//! The subscription row's "signaled" marker is `signaled_at`, not
//! `signal_data`: a signal sent without data leaves `signal_data` NULL, and
//! the wait must still count as signaled. These tests drive
//! `PostgresSubscriptionService` against a real database, and check the
//! `20261119000002_subscription_signaled_at` migration's backfill and down
//! migration.

use kruxiaflow_core::subscription::{
    NewSubscription, PostgresSubscriptionService, SignalRequest, SubscriptionService,
};
use kruxiaflow_core::workflow::OnTimeout;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const UP: &str = include_str!("../../migrations/20261119000002_subscription_signaled_at.up.sql");
const DOWN: &str =
    include_str!("../../migrations/20261119000002_subscription_signaled_at.down.sql");

/// A running workflow for the subscriptions' foreign key
async fn insert_test_workflow(pool: &PgPool) -> Uuid {
    let def_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workflow_definitions (name, activities) VALUES ($1, '[]'::jsonb) RETURNING id",
    )
    .bind("signal_once_test")
    .fetch_one(pool)
    .await
    .expect("insert workflow definition");

    let workflow_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data) \
         VALUES ($1, $2, $3, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)",
    )
    .bind(workflow_id)
    .bind("signal_once_test")
    .bind(def_id)
    .execute(pool)
    .await
    .expect("insert workflow");
    workflow_id
}

/// A wait for `go` on `key`; `timeout_seconds: 0` is already past its deadline
fn wait(workflow_id: Uuid, key: &str, timeout_seconds: u64) -> NewSubscription {
    NewSubscription {
        workflow_id,
        activity_key: key.to_string(),
        event_name: "go".to_string(),
        on_timeout: OnTimeout::Fail,
        timeout_seconds,
        iteration: None,
    }
}

fn signal(workflow_id: Uuid, key: &str, data: Option<Value>) -> SignalRequest {
    SignalRequest {
        workflow_id,
        activity_key: key.to_string(),
        event_name: "go".to_string(),
        data,
    }
}

/// The row's markers: (signaled_at is set, signal_data, expired_at is set)
async fn row(pool: &PgPool, workflow_id: Uuid, key: &str) -> (bool, Option<Value>, bool) {
    sqlx::query_as(
        "SELECT signaled_at IS NOT NULL, signal_data, expired_at IS NOT NULL \
         FROM activity_event_subscriptions WHERE workflow_id = $1 AND activity_key = $2",
    )
    .bind(workflow_id)
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("subscription row")
}

/// After one signal, every further signal for the wait is rejected, whatever
/// either one carries, and the row keeps the first signal's data
async fn second_signal_rejected(pool: PgPool, first: Option<Value>, second: Option<Value>) {
    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());
    service
        .create_subscription(wait(workflow_id, "approve", 300))
        .await
        .unwrap();

    let r1 = service
        .signal_activity(signal(workflow_id, "approve", first.clone()))
        .await
        .unwrap();
    assert!(r1.is_some(), "the first signal must be accepted");
    assert_eq!(
        row(&pool, workflow_id, "approve").await,
        (true, first.clone(), false)
    );

    let r2 = service
        .signal_activity(signal(workflow_id, "approve", second))
        .await
        .unwrap();
    assert!(
        r2.is_none(),
        "a second signal for a wait already signaled was accepted: {r2:?}"
    );
    assert_eq!(
        row(&pool, workflow_id, "approve").await,
        (true, first, false),
        "the second signal changed the row"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_second_signal_rejected_without_data(pool: PgPool) {
    second_signal_rejected(pool, None, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_second_signal_rejected_without_then_with_data(pool: PgPool) {
    second_signal_rejected(pool, None, Some(json!({"approved": false}))).await;
}

/// Both signals carry data
#[sqlx::test(migrations = "../migrations")]
async fn test_second_signal_rejected_with_data(pool: PgPool) {
    second_signal_rejected(pool, Some(json!({"n": 1})), Some(json!({"n": 2}))).await;
}

/// Concurrent signals without data: exactly one is accepted
#[sqlx::test(migrations = "../migrations")]
async fn test_concurrent_signals_without_data_one_accepted(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());
    service
        .create_subscription(wait(workflow_id, "approve", 300))
        .await
        .unwrap();

    let results = futures::future::join_all(
        (0..8).map(|_| service.signal_activity(signal(workflow_id, "approve", None))),
    )
    .await;
    let accepted = results
        .into_iter()
        .map(|r| r.unwrap())
        .filter(Option::is_some)
        .count();
    assert_eq!(accepted, 1, "{accepted} of 8 concurrent signals accepted");
}

/// A wait signaled before its deadline is not expired by the checker, even
/// when the orchestrator has not yet taken the signal (the row still exists)
async fn signaled_wait_not_expired(pool: PgPool, data: Option<Value>) {
    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());
    service
        .create_subscription(wait(workflow_id, "approve", 0))
        .await
        .unwrap();
    let r = service
        .signal_activity(signal(workflow_id, "approve", data.clone()))
        .await
        .unwrap();
    assert!(r.is_some());

    let expired = service.expire_subscriptions(100).await.unwrap();
    assert!(
        expired.is_empty(),
        "a signaled wait was expired: {expired:?}"
    );
    assert_eq!(
        row(&pool, workflow_id, "approve").await,
        (true, data, false)
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signaled_wait_not_expired_without_data(pool: PgPool) {
    signaled_wait_not_expired(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signaled_wait_not_expired_with_data(pool: PgPool) {
    signaled_wait_not_expired(pool, Some(json!({"n": 1}))).await;
}

/// A wait the checker has expired takes no signal: the timeout ended it
async fn expired_wait_rejects_signal(pool: PgPool, data: Option<Value>) {
    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());
    service
        .create_subscription(wait(workflow_id, "approve", 0))
        .await
        .unwrap();
    let expired = service.expire_subscriptions(100).await.unwrap();
    assert_eq!(expired.len(), 1);

    let r = service
        .signal_activity(signal(workflow_id, "approve", data))
        .await
        .unwrap();
    assert!(r.is_none(), "a signal was accepted after expiry: {r:?}");
    assert_eq!(
        row(&pool, workflow_id, "approve").await,
        (false, None, true)
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_expired_wait_rejects_signal_without_data(pool: PgPool) {
    expired_wait_rejects_signal(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_expired_wait_rejects_signal_with_data(pool: PgPool) {
    expired_wait_rejects_signal(pool, Some(json!({"n": 1}))).await;
}

/// A signal without data still reads back as no data
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_without_data_has_no_signal_data(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());
    service
        .create_subscription(wait(workflow_id, "approve", 300))
        .await
        .unwrap();
    let r = service
        .signal_activity(signal(workflow_id, "approve", None))
        .await
        .unwrap()
        .expect("signal accepted");
    assert_eq!(r.signal_data, None);
    assert_eq!(
        service
            .get_signal_data(workflow_id, "approve")
            .await
            .unwrap(),
        None
    );
}

/// The migration, from the schema before it: a row signaled with data counts
/// as signaled; a waiting row, and one signaled without data (which cannot be
/// told apart from it), stay waiting. The down migration keeps a wait
/// signaled without data counted as signaled under the old marker.
#[sqlx::test(migrations = "../migrations")]
async fn test_migration_backfill_and_down(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    sqlx::raw_sql(DOWN)
        .execute(&pool)
        .await
        .expect("down migration");
    for (key, data) in [
        ("with_data", Some(json!({"approved": true}))),
        ("waiting", None),
    ] {
        sqlx::query(
            "INSERT INTO activity_event_subscriptions \
                 (workflow_id, activity_key, event_name, on_timeout, timeout_at, signal_data) \
             VALUES ($1, $2, 'go', 'fail', NOW() + INTERVAL '1 hour', $3)",
        )
        .bind(workflow_id)
        .bind(key)
        .bind(data)
        .execute(&pool)
        .await
        .unwrap();
    }

    sqlx::raw_sql(UP)
        .execute(&pool)
        .await
        .expect("up migration");
    assert_eq!(
        row(&pool, workflow_id, "with_data").await,
        (true, Some(json!({"approved": true})), false)
    );
    assert_eq!(
        row(&pool, workflow_id, "waiting").await,
        (false, None, false)
    );

    // The backfilled row takes no further signal; the waiting one takes one
    let service = PostgresSubscriptionService::new(pool.clone());
    let again = service
        .signal_activity(signal(workflow_id, "with_data", None))
        .await
        .unwrap();
    assert!(again.is_none());
    let first = service
        .signal_activity(signal(workflow_id, "waiting", None))
        .await
        .unwrap();
    assert!(first.is_some());

    // Down: the wait signaled without data keeps a non-NULL signal_data
    sqlx::raw_sql(DOWN)
        .execute(&pool)
        .await
        .expect("down migration");
    let old_marker: Vec<(String, Option<Value>)> = sqlx::query_as(
        "SELECT activity_key, signal_data FROM activity_event_subscriptions \
         WHERE workflow_id = $1 ORDER BY activity_key",
    )
    .bind(workflow_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        old_marker,
        vec![
            ("waiting".to_string(), Some(Value::Null)),
            ("with_data".to_string(), Some(json!({"approved": true}))),
        ]
    );
    let column: Option<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_name = 'activity_event_subscriptions' AND column_name = 'signaled_at'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(column, None, "down migration left signaled_at");

    // And up again, so the database matches its migration history
    sqlx::raw_sql(UP)
        .execute(&pool)
        .await
        .expect("up migration");
}

/// A signal that commits while the expiry checker's UPDATE waits on the row
/// wins: the checker skips the row instead of marking a signaled wait expired
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_committed_during_expiry_wins(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());
    service
        .create_subscription(wait(workflow_id, "step", 0))
        .await
        .unwrap();

    // The signal's UPDATE, holding the row lock until it commits
    let mut signal_tx = pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE activity_event_subscriptions SET signaled_at = NOW() \
         WHERE workflow_id = $1 AND signaled_at IS NULL AND expired_at IS NULL",
    )
    .bind(workflow_id)
    .execute(&mut *signal_tx)
    .await
    .unwrap();

    let checker = tokio::spawn(async move { service.expire_subscriptions(100).await });

    // Wait until the checker's UPDATE is blocked on the row
    let start = std::time::Instant::now();
    loop {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock' \
               AND query LIKE '%SET expired_at%')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if blocked {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the checker never blocked on the signaled row"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    signal_tx.commit().await.unwrap();

    let expired = checker.await.unwrap().unwrap();
    assert!(
        expired.is_empty(),
        "a signaled wait was expired: {expired:?}"
    );
    assert_eq!(row(&pool, workflow_id, "step").await, (true, None, false));
}
