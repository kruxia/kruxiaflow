//! LISTEN/NOTIFY wake-up for event consumers
//! (docs/implementation/2026-11-08-commit-order-cursor-and-notify-wake.md §2.2).
//!
//! Notifications are scoped to a database, so each `sqlx::test` database
//! sees only its own.

mod common;

use kruxiaflow_core::events::{
    EventSource, EventWake, EventWakeMode, NewWorkflowEvent, PostgresEventSource, Wake,
    WorkflowEventType,
};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// Generous bound on notification latency (it is normally ~1ms)
const WAKE_WITHIN: Duration = Duration::from_millis(500);
/// How long to wait to be sure nothing arrives
const QUIET: Duration = Duration::from_millis(200);

fn event(workflow_id: Uuid) -> NewWorkflowEvent {
    NewWorkflowEvent {
        workflow_id,
        event_type: WorkflowEventType::WorkflowCreated,
        activity_key: None,
        payload: json!({}),
        iteration: None,
    }
}

async fn wake_source(pool: &PgPool) -> PostgresEventSource {
    let wake = EventWake::connect(pool).await.expect("LISTEN");
    PostgresEventSource::with_wake(pool.clone(), wake)
}

#[sqlx::test(migrations = "../migrations")]
async fn test_publish_wakes_receiver(pool: PgPool) {
    let source = wake_source(&pool).await;
    let mut rx = source.wake().expect("wake attached");

    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);

    let started = std::time::Instant::now();
    source.publish(event(Uuid::now_v7())).await.unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
    assert!(started.elapsed() < WAKE_WITHIN);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_before_wait_is_not_lost(pool: PgPool) {
    let source = wake_source(&pool).await;
    let mut rx = source.wake().unwrap();

    source.publish(event(Uuid::now_v7())).await.unwrap();
    // The notification arrives while nobody is waiting
    tokio::time::sleep(QUIET).await;
    assert_eq!(rx.wait(Duration::from_millis(1)).await, Wake::Woken);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_every_receiver_wakes(pool: PgPool) {
    let source = wake_source(&pool).await;
    let mut a = source.wake().unwrap();
    let mut b = source.wake().unwrap();

    source.publish(event(Uuid::now_v7())).await.unwrap();
    assert_eq!(a.wait(WAKE_WITHIN).await, Wake::Woken);
    assert_eq!(b.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// The trigger, not the client, is the source of the notification
#[sqlx::test(migrations = "../migrations")]
async fn test_plain_sql_insert_wakes(pool: PgPool) {
    let source = wake_source(&pool).await;
    let mut rx = source.wake().unwrap();

    sqlx::query(
        "INSERT INTO workflow_events (workflow_id, event_type, payload) \
         VALUES ($1, 'WorkflowCreated', '{}')",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// Notifications are delivered at commit; a rolled-back insert never wakes
#[sqlx::test(migrations = "../migrations")]
async fn test_notification_waits_for_commit(pool: PgPool) {
    let source = wake_source(&pool).await;
    let mut rx = source.wake().unwrap();

    // (A rolled-back first uuidv7() call on a connection is also the case
    // migration 20261108000003 fixes for PostgreSQL < 18.)
    let insert = "INSERT INTO workflow_events (workflow_id, event_type, payload) \
                  VALUES ($1, 'WorkflowCreated', '{}')";

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(insert)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);
    tx.rollback().await.unwrap();
    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(insert)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);
    tx.commit().await.unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// A duplicate publish inserts nothing but still fires the statement
/// trigger: the consumer wakes, polls, and finds nothing new
#[sqlx::test(migrations = "../migrations")]
async fn test_conflicting_insert_wakes_and_poll_is_empty(pool: PgPool) {
    let source = wake_source(&pool).await;
    let mut rx = source.wake().unwrap();
    let workflow_id = Uuid::now_v7();

    source.publish(event(workflow_id)).await.unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
    common::settle_events(&pool).await;
    let events = source.poll("c").await.unwrap();
    assert_eq!(events.len(), 1);
    source
        .update_position("c", events[0].cursor())
        .await
        .unwrap();

    source.publish(event(workflow_id)).await.unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
    common::settle_events(&pool).await;
    assert!(source.poll("c").await.unwrap().is_empty());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_mode_has_no_wake(pool: PgPool) {
    let source = PostgresEventSource::connect(pool.clone(), EventWakeMode::Poll)
        .await
        .unwrap();
    assert!(source.wake().is_none());

    let source = PostgresEventSource::connect(pool.clone(), EventWakeMode::Notify)
        .await
        .unwrap();
    assert!(source.wake().is_some());
}
