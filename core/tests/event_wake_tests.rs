//! LISTEN/NOTIFY wake-up for event consumers
//! (docs/implementation/2026-11-08-commit-order-cursor-and-notify-wake.md §2.2;
//! since migration 20261123000001 the application announces each publish,
//! not a trigger).
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

/// How long to wait for a wake (it normally takes about 1 ms). A receiver
/// has no timer of its own, so only a wake (or a listener reconnect) ends the
/// wait with [`Wake::Woken`]: the bound can be generous for a busy machine and
/// costs nothing when the wake comes.
const WAKE_WITHIN: Duration = Duration::from_secs(5);
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
    let pool = common::unshared_pool(&pool).await;
    let source = wake_source(&pool).await;
    let mut rx = source.wake().expect("wake attached");

    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);

    source.publish(event(Uuid::now_v7())).await.unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_before_wait_is_not_lost(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let source = wake_source(&pool).await;
    let mut rx = source.wake().unwrap();

    source.publish(event(Uuid::now_v7())).await.unwrap();
    // The notification arrives while nobody is waiting
    tokio::time::sleep(QUIET).await;
    assert_eq!(rx.wait(Duration::from_millis(1)).await, Wake::Woken);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_every_receiver_wakes(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let source = wake_source(&pool).await;
    let mut a = source.wake().unwrap();
    let mut b = source.wake().unwrap();

    source.publish(event(Uuid::now_v7())).await.unwrap();
    assert_eq!(a.wait(WAKE_WITHIN).await, Wake::Woken);
    assert_eq!(b.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// The application announces its events; the database does not. An event
/// written by SQL alone wakes nobody, and consumers find it on their timer.
#[sqlx::test(migrations = "../migrations")]
async fn test_plain_sql_insert_does_not_wake(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
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
    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);
}

/// An event published by another process (here, an event source with a wake
/// of its own) reaches this one as a notification
#[sqlx::test(migrations = "../migrations")]
async fn test_publish_in_another_process_wakes(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let listener = wake_source(&pool).await;
    let mut rx = listener.wake().unwrap();
    let other_process = wake_source(&pool).await;

    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);
    other_process.publish(event(Uuid::now_v7())).await.unwrap();
    assert_eq!(rx.wait(WAKE_WITHIN).await, Wake::Woken);
}

/// A duplicate publish inserts nothing, so it announces nothing
#[sqlx::test(migrations = "../migrations")]
async fn test_conflicting_insert_does_not_wake(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
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
    assert_eq!(rx.wait(QUIET).await, Wake::TimedOut);
    common::settle_events(&pool).await;
    assert!(source.poll("c").await.unwrap().is_empty());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_mode_has_no_wake(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let source = PostgresEventSource::connect(pool.clone(), EventWakeMode::Poll)
        .await
        .unwrap();
    assert!(source.wake().is_none());

    let source = PostgresEventSource::connect(pool.clone(), EventWakeMode::Notify)
        .await
        .unwrap();
    assert!(source.wake().is_some());
}
