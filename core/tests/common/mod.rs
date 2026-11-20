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
//!
//! Waiting on other tests' transactions is only safe if those transactions
//! always end, and with the pool `sqlx::test` hands out they need not: every
//! test that uses this module first replaces it with [`unshared_pool`].
#![allow(dead_code)]

use async_trait::async_trait;
use kruxiaflow_core::events::{
    EventCursor, EventSource, NewWorkflowEvent, PostgresEventSource, Result, WakeReceiver,
    WorkflowEvent,
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Give up waiting after this long and read anyway (the test then fails on
/// its own assertions, with the stall visible in the elapsed time)
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// A pool for this test alone, to the same database and of the same size as
/// the one `sqlx::test` gave it, but outside the test binary's shared
/// connection budget. Tests that read events, run orchestrator handlers, or
/// hold transactions call it first, as `let pool = unshared_pool(&pool).await`.
///
/// **Why:** every `sqlx::test` pool (5 connections) draws its connections from
/// one parent pool of 20 per test binary (`MASTER_POOL` in sqlx-postgres), and
/// keeps each slot it has drawn until the test ends. An orchestrator handler
/// holds its transaction, with an xid once it has written, while it takes a
/// second connection to publish (`OrchestratorConfig::effective_partitions`
/// sizes the partitions for this, but only against the test's own pool). With
/// enough tests running at once, the binary deadlocks until a timeout:
///
/// - test A's handler has written (`INSERT INTO workflows`) and waits for a
///   connection, while other tests hold all 20 slots;
/// - those tests are in [`settle_events`], waiting for A's xid to leave the
///   instance's horizon, and keep their slots while they wait;
/// - nothing moves until a [`SETTLE_TIMEOUT`] expires and its test fails on an
///   empty poll (seen in the loop tests: 4 handlers idle in transaction for
///   10 s, 5 tests timing out).
///
/// A pool of its own has no parent, so a test's connections wait only on that
/// test. The waits cannot be narrowed to the test's own database instead,
/// because the production poll's horizon is instance-wide (see
/// [`settle_events`]). Reserving a test's 5 slots up front does not work
/// either: `sqlx::test` has already drawn one slot per test before the test
/// body runs, so tests queued for a reservation hold the slots the reserving
/// test waits for.
///
/// The cost is connections: up to 5 per running test on top of the idle one
/// `sqlx::test` keeps, so the server needs `max_connections` of at least
/// 6 x test threads plus 20.
pub async fn unshared_pool(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(pool.options().get_max_connections())
        .idle_timeout(Some(Duration::from_secs(1)))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("connect the test's own pool")
}

/// Wait until no committed event in this database belongs to a transaction
/// at or above the instance's xmin, i.e. until a poll can return every
/// committed event.
///
/// The horizon is deliberately the instance-wide one, the predicate the poll
/// that follows uses (`tx < pg_snapshot_xmin(pg_current_snapshot())`).
/// **Why not only this database's transactions:** waiting less than the poll
/// does cannot make the poll return more. Settling would return while another
/// database's transaction still holds the horizon, and the real poll would
/// then come back empty: the same failure as a timeout, only sooner. What keeps
/// this wait short is that no test holds a writing transaction for long, which
/// [`unshared_pool`] makes sure of.
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

    async fn poll_after(
        &self,
        consumer_id: &str,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<Vec<WorkflowEvent>> {
        settle_events(&self.pool).await;
        self.inner.poll_after(consumer_id, after, limit).await
    }

    async fn has_pending_events_after(
        &self,
        consumer_id: &str,
        after: Option<EventCursor>,
    ) -> Result<bool> {
        self.inner
            .has_pending_events_after(consumer_id, after)
            .await
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
