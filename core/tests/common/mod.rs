//! Shared helpers for integration tests that read the event stream.
//!
//! `PostgresEventSource::poll` only returns events from transactions below
//! the instance's oldest in-progress transaction (`pg_snapshot_xmin`).
//! Transaction ids are instance-wide, and `sqlx::test` runs many tests (each
//! in its own database, each applying migrations in transactions) against one
//! Postgres instance at once, so an event a test has just committed can be
//! held back for a moment by an unrelated test's open transaction. A test
//! that publishes and then polls once would see a spurious empty batch.
//!
//! [`SettledEventSource`] waits, before each read, until every committed
//! event in the test's database is deliverable, then reads through the real
//! `PostgresEventSource`. Tests of the delivery predicate itself use
//! `PostgresEventSource` directly.
#![allow(dead_code)]

use async_trait::async_trait;
use kruxiaflow_core::events::{
    EventCursor, EventSource, NewWorkflowEvent, PostgresEventSource, Result, WakeReceiver,
    WorkflowEvent,
};
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Give up waiting after this long and read anyway (the test then fails on
/// its own assertions, with the stall visible in the elapsed time)
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Wait until no committed event in this database belongs to a transaction
/// at or above the instance's xmin, i.e. until a poll can return every
/// committed event.
pub async fn settle_events(pool: &PgPool) {
    let start = Instant::now();
    while start.elapsed() < SETTLE_TIMEOUT {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workflow_events \
             WHERE tx >= pg_snapshot_xmin(pg_current_snapshot()))",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(false);
        if !blocked {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A `PostgresEventSource` that settles before every read
pub struct SettledEventSource {
    inner: PostgresEventSource,
    pool: PgPool,
}

pub fn settled_event_source(pool: PgPool) -> SettledEventSource {
    SettledEventSource {
        inner: PostgresEventSource::new(pool.clone()),
        pool,
    }
}

/// A settling event source with a LISTEN/NOTIFY wake attached
pub async fn settled_event_source_with_wake(pool: PgPool) -> SettledEventSource {
    let wake = kruxiaflow_core::events::EventWake::connect(&pool)
        .await
        .expect("LISTEN");
    SettledEventSource {
        inner: PostgresEventSource::with_wake(pool.clone(), wake),
        pool,
    }
}

#[async_trait]
impl EventSource for SettledEventSource {
    async fn publish(&self, event: NewWorkflowEvent) -> Result<()> {
        self.inner.publish(event).await
    }

    async fn poll(&self, consumer_id: &str) -> Result<Vec<WorkflowEvent>> {
        settle_events(&self.pool).await;
        self.inner.poll(consumer_id).await
    }

    async fn update_position(&self, consumer_id: &str, cursor: EventCursor) -> Result<()> {
        self.inner.update_position(consumer_id, cursor).await
    }

    async fn get_events_from_id(
        &self,
        from_event_id: Uuid,
        workflow_ids: Option<&[Uuid]>,
        limit: i64,
    ) -> Result<Vec<WorkflowEvent>> {
        settle_events(&self.pool).await;
        self.inner
            .get_events_from_id(from_event_id, workflow_ids, limit)
            .await
    }

    async fn has_pending_events(&self, consumer_id: &str) -> Result<bool> {
        self.inner.has_pending_events(consumer_id).await
    }

    fn wake(&self) -> Option<WakeReceiver> {
        self.inner.wake()
    }
}

/// Poll `check` every 20ms until it returns true or `timeout` elapses;
/// returns whether it succeeded
pub async fn wait_until<F, Fut>(timeout: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    loop {
        if check().await {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
