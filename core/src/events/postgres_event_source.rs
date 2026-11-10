use super::{
    ActivityWake, EventCursor, EventSource, EventWake, EventWakeMode, NewWorkflowEvent, Result,
    WakeReceiver, WorkflowEvent, WorkflowEventType, find_foreign_xids, verify_event_xids,
};
use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Report a stalled stream at most this often
const STALL_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Check that the stored transaction ids belong to this cluster at most this
/// often
const CLUSTER_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// A writing transaction open longer than this is reported as a possible stall
const STALL_TRANSACTION_AGE_SECS: f64 = 30.0;

/// PostgreSQL-based event source implementation
///
/// # Delivery
///
/// Events are read in `(tx, position)` order, where `tx` is the inserting
/// transaction's id and `position` the insert order, and only from
/// transactions below `pg_snapshot_xmin(pg_current_snapshot())`: the oldest
/// transaction still in progress anywhere on the instance. Every transaction
/// below it has committed (its rows are visible to the poll) or aborted (its
/// rows never will be). A row that becomes visible later belongs to a
/// transaction at or above that xmin, so it sorts after every row returned so
/// far. A consumer can therefore store the cursor of the last event it
/// processed and nothing is ever left behind it.
///
/// The cost is head-of-line blocking: transaction ids are instance-wide, so
/// one long-running *writing* transaction in any database on the instance
/// holds xmin down and delays every consumer until it ends.
/// [`has_pending_events`](EventSource::has_pending_events) tells a consumer
/// whose poll came back empty that events are waiting behind such a
/// transaction, so that it polls again soon; while they wait it logs the
/// offending backend once a minute.
pub struct PostgresEventSource {
    pool: PgPool,
    wake: Option<EventWake>,
    last_stall_check: Mutex<Option<Instant>>,
    last_cluster_check: Mutex<Option<Instant>>,
}

/// True at most once per `interval` for the given timestamp slot
fn due(slot: &Mutex<Option<Instant>>, interval: Duration) -> bool {
    let Ok(mut last) = slot.lock() else {
        return false;
    };
    if last.is_some_and(|t| t.elapsed() < interval) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

impl PostgresEventSource {
    /// An event source whose consumers poll on their backoff timer alone
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            wake: None,
            last_stall_check: Mutex::new(None),
            last_cluster_check: Mutex::new(None),
        }
    }

    /// An event source whose consumers also wake on commit notifications
    pub fn with_wake(pool: PgPool, wake: EventWake) -> Self {
        Self {
            wake: Some(wake),
            ..Self::new(pool)
        }
    }

    /// Build an event source for a process that consumes events, in the
    /// given wake mode, logging which mode is active.
    ///
    /// Fails with [`EventError::ForeignCluster`](super::EventError) if the
    /// stored event transaction ids cannot belong to this database cluster
    /// (after a dump/restore or logical migration): consuming in that state
    /// silently delivers nothing. Also fails if the check itself cannot run,
    /// e.g. when migrations have not been applied.
    ///
    /// In `notify` mode a failed `LISTEN` (no permission, a transaction-mode
    /// pooler in front of the database) logs a warning and falls back to
    /// polling.
    pub async fn connect(pool: PgPool, mode: EventWakeMode) -> Result<Self> {
        verify_event_xids(&pool).await?;

        Ok(match mode {
            EventWakeMode::Poll => {
                tracing::info!("Event wake: poll (timer only)");
                Self::new(pool)
            }
            EventWakeMode::Notify => match EventWake::connect(&pool).await {
                Ok(wake) => {
                    tracing::info!("Event wake: notify (LISTEN workflow_events)");
                    Self::with_wake(pool, wake)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Event wake: LISTEN failed, falling back to poll (timer only)"
                    );
                    Self::new(pool)
                }
            },
        })
    }

    /// The activity-queue wake carried by this event source's listener
    /// connection, if it has one (see [`ActivityWake`]). `None` in poll mode
    /// or when `LISTEN` failed.
    pub fn activity_wake(&self) -> Option<ActivityWake> {
        self.wake.as_ref().map(EventWake::activity_wake)
    }

    /// While events wait behind the delivery horizon, at most once per
    /// `STALL_CHECK_INTERVAL`, log a writing transaction that has been open
    /// longer than `STALL_TRANSACTION_AGE_SECS`.
    ///
    /// It holds back the instance's xmin, so the waiting events are not
    /// delivered until it ends; without this an operator cannot tell a stall
    /// from idleness.
    async fn warn_if_stalled(&self) {
        if !due(&self.last_stall_check, STALL_CHECK_INTERVAL) {
            return;
        }

        let oldest = sqlx::query_as::<_, (i32, Option<String>, f64, i64)>(
            r#"
            SELECT a.pid, a.datname,
                   EXTRACT(EPOCH FROM now() - a.xact_start)::float8,
                   pg_snapshot_xmax(pg_current_snapshot())::text::bigint
                     - pg_snapshot_xmin(pg_current_snapshot())::text::bigint
            FROM pg_stat_activity a
            WHERE a.backend_xid IS NOT NULL
              AND a.xact_start < now() - make_interval(secs => $1)
            ORDER BY a.xact_start
            LIMIT 1
            "#,
        )
        .bind(STALL_TRANSACTION_AGE_SECS)
        .fetch_optional(&self.pool)
        .await;

        match oldest {
            Ok(Some((pid, datname, age_secs, xids_behind))) => tracing::warn!(
                pid,
                database = datname.as_deref().unwrap_or("?"),
                transaction_age_secs = age_secs.round() as i64,
                xids_behind,
                "Committed workflow events are waiting behind a long-running writing \
                 transaction and will not be delivered until it ends (events are \
                 delivered only below the oldest open transaction id on the instance)"
            ),
            Ok(None) => {}
            Err(e) => tracing::debug!(error = %e, "Event stall check failed"),
        }
    }

    /// On an empty poll, at most once per `CLUSTER_CHECK_INTERVAL`, check
    /// that the stored transaction ids still belong to this cluster.
    ///
    /// The database can be swapped for a restored or logically migrated copy
    /// under a running process (no restart, so the startup check never ran);
    /// nothing is delivered from then on.
    async fn check_cluster(&self) {
        if !due(&self.last_cluster_check, CLUSTER_CHECK_INTERVAL) {
            return;
        }

        match find_foreign_xids(&self.pool).await {
            Ok(Some(found)) => tracing::error!(
                max_stored_tx = found.max_stored_tx(),
                next_xid = found.next_xid,
                "Workflow event cursors were written by a different PostgreSQL cluster; \
                 events are not being delivered. Stop every Kruxia Flow process, then run \
                 `kruxiaflow rebase-events`"
            ),
            Ok(None) => {}
            Err(e) => tracing::debug!(error = %e, "Event cluster check failed"),
        }
    }
}

#[async_trait]
impl EventSource for PostgresEventSource {
    /// Publish a workflow event to the event stream
    /// - Idempotent via the workflow_events_dedup_idx unique index
    ///   (workflow_id, event_type, activity_key, iteration, payload->>'attempt'):
    ///   per-attempt publishers (worker failures, retry scheduling, timeout
    ///   failures) set 'attempt' in the payload so each attempt's events are
    ///   distinct; every other event type carries no attempt and keeps
    ///   one-slot idempotency (NULLS NOT DISTINCT)
    /// - Database assigns id (UUIDv7), timestamp, position and tx
    #[tracing::instrument(skip(self), level = "debug")]
    async fn publish(&self, event: NewWorkflowEvent) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO workflow_events (workflow_id, event_type, activity_key, payload, iteration)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (workflow_id, event_type, activity_key, iteration, ((payload->>'attempt'))) DO NOTHING
            "#,
            event.workflow_id,
            event.event_type as WorkflowEventType,
            event.activity_key,
            event.payload,
            event.iteration
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Poll for new events past the consumer's stored cursor
    /// - Returns up to 1000 events per poll, in (tx, position) order
    /// - Only events from finished transactions (see the type docs)
    /// - If no cursor exists (first poll), returns events from the beginning
    ///
    /// Performance: the scalar subqueries are evaluated once and used as
    /// constants in the index condition, giving a range scan on the
    /// idx_events_consumer_poll covering index (tx, position).
    #[tracing::instrument(skip(self), level = "trace")]
    async fn poll(&self, consumer_id: &str) -> Result<Vec<WorkflowEvent>> {
        let events = sqlx::query_as!(
            WorkflowEvent,
            r#"
            SELECT id, position, tx::text::bigint AS "tx!", workflow_id,
                   event_type as "event_type: WorkflowEventType",
                   activity_key, payload, timestamp, iteration
            FROM workflow_events
            WHERE (tx, position) > (
                    COALESCE((SELECT last_tx FROM workflow_event_consumers WHERE consumer_id = $1),
                             '0'::xid8),
                    COALESCE((SELECT last_position FROM workflow_event_consumers WHERE consumer_id = $1),
                             0))
              AND tx < pg_snapshot_xmin(pg_current_snapshot())
            ORDER BY tx, position
            LIMIT 1000
            "#,
            consumer_id
        )
        .fetch_all(&self.pool)
        .await?;

        tracing::trace!(event_count = events.len(), "Polled events");

        if events.is_empty() {
            self.check_cluster().await;
        }

        Ok(events)
    }

    /// Whether committed events lie past the consumer's stored cursor
    ///
    /// Unlike `poll` this has no transaction-id bound, so after an empty
    /// poll it is true exactly when events are waiting behind the delivery
    /// horizon (or crossed it since that poll). One index probe on
    /// idx_events_consumer_poll.
    #[tracing::instrument(skip(self), level = "trace")]
    async fn has_pending_events(&self, consumer_id: &str) -> Result<bool> {
        let pending = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM workflow_events
                WHERE (tx, position) > (
                        COALESCE((SELECT last_tx FROM workflow_event_consumers WHERE consumer_id = $1),
                                 '0'::xid8),
                        COALESCE((SELECT last_position FROM workflow_event_consumers WHERE consumer_id = $1),
                                 0))
            ) AS "pending!"
            "#,
            consumer_id
        )
        .fetch_one(&self.pool)
        .await?;

        if pending {
            self.warn_if_stalled().await;
        }

        Ok(pending)
    }

    /// Events after an explicit cursor, under the same horizon as `poll`
    #[tracing::instrument(skip(self), level = "trace")]
    async fn poll_after(
        &self,
        consumer_id: &str,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<Vec<WorkflowEvent>> {
        let Some(after) = after else {
            let mut events = self.poll(consumer_id).await?;
            events.truncate(limit);
            return Ok(events);
        };

        let events = sqlx::query_as!(
            WorkflowEvent,
            r#"
            SELECT id, position, tx::text::bigint AS "tx!", workflow_id,
                   event_type as "event_type: WorkflowEventType",
                   activity_key, payload, timestamp, iteration
            FROM workflow_events
            WHERE (tx, position) > (($1::bigint)::text::xid8, $2)
              AND tx < pg_snapshot_xmin(pg_current_snapshot())
            ORDER BY tx, position
            LIMIT $3
            "#,
            after.tx,
            after.position,
            limit.min(1000) as i64
        )
        .fetch_all(&self.pool)
        .await?;

        tracing::trace!(event_count = events.len(), "Polled events after cursor");

        if events.is_empty() {
            self.check_cluster().await;
        }

        Ok(events)
    }

    /// Whether committed events lie past an explicit cursor
    #[tracing::instrument(skip(self), level = "trace")]
    async fn has_pending_events_after(
        &self,
        consumer_id: &str,
        after: Option<EventCursor>,
    ) -> Result<bool> {
        let Some(after) = after else {
            return self.has_pending_events(consumer_id).await;
        };

        let pending = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM workflow_events
                WHERE (tx, position) > (($1::bigint)::text::xid8, $2)
            ) AS "pending!"
            "#,
            after.tx,
            after.position
        )
        .fetch_one(&self.pool)
        .await?;

        if pending {
            self.warn_if_stalled().await;
        }

        Ok(pending)
    }

    /// Get events after a specific event ID, optionally filtered by workflow IDs.
    ///
    /// Used for WebSocket reconnection replay. Resolves `from_event_id` to its
    /// (tx, position) and returns the events after it in cursor order, from
    /// finished transactions only, limited to `limit` rows. An unknown id
    /// makes the comparison NULL and the replay empty.
    ///
    /// - When workflow_ids is Some and non-empty, filters to those workflows
    /// - When workflow_ids is None or empty, returns all events
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_events_from_id(
        &self,
        from_event_id: Uuid,
        workflow_ids: Option<&[Uuid]>,
        limit: i64,
    ) -> Result<Vec<WorkflowEvent>> {
        let events = match workflow_ids {
            Some(ids) if !ids.is_empty() => {
                sqlx::query_as!(
                    WorkflowEvent,
                    r#"
                    SELECT id, position, tx::text::bigint AS "tx!", workflow_id,
                           event_type as "event_type: WorkflowEventType",
                           activity_key, payload, timestamp, iteration
                    FROM workflow_events
                    WHERE (tx, position) > (SELECT f.tx, f.position FROM workflow_events f WHERE f.id = $1)
                      AND tx < pg_snapshot_xmin(pg_current_snapshot())
                      AND workflow_id = ANY($2)
                    ORDER BY tx, position
                    LIMIT $3
                    "#,
                    from_event_id,
                    ids,
                    limit
                )
                .fetch_all(&self.pool)
                .await?
            }
            _ => {
                sqlx::query_as!(
                    WorkflowEvent,
                    r#"
                    SELECT id, position, tx::text::bigint AS "tx!", workflow_id,
                           event_type as "event_type: WorkflowEventType",
                           activity_key, payload, timestamp, iteration
                    FROM workflow_events
                    WHERE (tx, position) > (SELECT f.tx, f.position FROM workflow_events f WHERE f.id = $1)
                      AND tx < pg_snapshot_xmin(pg_current_snapshot())
                    ORDER BY tx, position
                    LIMIT $2
                    "#,
                    from_event_id,
                    limit
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        tracing::debug!(
            event_count = events.len(),
            "Fetched events from checkpoint ID"
        );

        Ok(events)
    }

    /// Store the consumer's cursor after successfully processing events
    /// - Upserts the consumer row
    /// - Only moves the cursor forward (safe for concurrent consumers sharing
    ///   an id: the WHERE clause rejects a cursor behind the stored one)
    async fn update_position(&self, consumer_id: &str, cursor: EventCursor) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO workflow_event_consumers (consumer_id, last_tx, last_position, updated_at)
            VALUES ($1, ($2::bigint)::text::xid8, $3, NOW())
            ON CONFLICT (consumer_id)
            DO UPDATE SET
                last_tx = EXCLUDED.last_tx,
                last_position = EXCLUDED.last_position,
                updated_at = NOW()
            WHERE (workflow_event_consumers.last_tx, workflow_event_consumers.last_position)
                < (EXCLUDED.last_tx, EXCLUDED.last_position)
            "#,
            consumer_id,
            cursor.tx,
            cursor.position
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    fn wake(&self) -> Option<WakeReceiver> {
        self.wake.as_ref().map(EventWake::subscribe)
    }
}
