//! Guard and repair for event cursors that outlive their database cluster.
//!
//! The event cursor orders by `tx`, the inserting transaction's id, and
//! delivers only below the cluster's oldest in-progress transaction id (see
//! [`PostgresEventSource`](super::PostgresEventSource)). `tx` values and
//! consumer cursors are stored as data, but the transaction-id counter belongs
//! to the cluster. `pg_upgrade`, streaming-replica promotion and physical
//! point-in-time recovery carry the counter along. A `pg_dump`/restore or a
//! logical-replication migration (including the blue/green upgrades of some
//! managed services) does not: the data arrives in a cluster whose counter
//! starts over. Then:
//!
//! - restored events have a `tx` far above the new cluster's xmin and are
//!   held back, and
//! - new events get small `tx` values that sort behind every restored
//!   cursor and are never delivered.
//!
//! Both are silent. [`find_foreign_xids`] detects the state (a committed row
//! or a cursor can never be at or above the cluster's next transaction id, so
//! there are no false positives), [`verify_event_xids`] turns it into a
//! startup error, and [`rebase_event_xids`] repairs it.

use sqlx::PgPool;

use super::{EventError, Result};

/// Stored transaction ids that this cluster cannot have assigned
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignXids {
    /// Highest `tx` of any event, if at or above `next_xid`
    pub max_event_tx: Option<i64>,
    /// Highest `last_tx` of any consumer cursor, if at or above `next_xid`
    pub max_cursor_tx: Option<i64>,
    /// The cluster's next transaction id (one past the highest completed)
    pub next_xid: i64,
}

impl ForeignXids {
    /// The highest foreign transaction id found
    pub fn max_stored_tx(&self) -> i64 {
        self.max_event_tx
            .unwrap_or(0)
            .max(self.max_cursor_tx.unwrap_or(0))
    }
}

/// What [`rebase_event_xids`] rewrote
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebaseReport {
    pub events: u64,
    pub consumers: u64,
}

/// Look for events or consumer cursors whose transaction id is at or above
/// the cluster's next transaction id. `None` means the stored ids are
/// consistent with this cluster.
pub async fn find_foreign_xids(pool: &PgPool) -> Result<Option<ForeignXids>> {
    // One statement, so the rows and the snapshot bound are consistent: a row
    // visible to a snapshot always has tx below that snapshot's xmax
    let (next_xid, max_event_tx, max_cursor_tx) =
        sqlx::query_as::<_, (i64, Option<i64>, Option<i64>)>(
            r#"
            SELECT pg_snapshot_xmax(pg_current_snapshot())::text::bigint,
                   (SELECT tx::text::bigint FROM workflow_events
                     ORDER BY tx DESC, position DESC LIMIT 1),
                   (SELECT last_tx::text::bigint FROM workflow_event_consumers
                     ORDER BY last_tx DESC LIMIT 1)
            "#,
        )
        .fetch_one(pool)
        .await?;

    let foreign = |tx: Option<i64>| tx.filter(|&tx| tx >= next_xid);
    let (max_event_tx, max_cursor_tx) = (foreign(max_event_tx), foreign(max_cursor_tx));

    if max_event_tx.is_none() && max_cursor_tx.is_none() {
        return Ok(None);
    }
    Ok(Some(ForeignXids {
        max_event_tx,
        max_cursor_tx,
        next_xid,
    }))
}

/// Fail with [`EventError::ForeignCluster`] if the stored transaction ids
/// cannot belong to this cluster. Called before a process starts consuming
/// events.
pub async fn verify_event_xids(pool: &PgPool) -> Result<()> {
    match find_foreign_xids(pool).await? {
        None => Ok(()),
        Some(found) => Err(EventError::ForeignCluster {
            max_stored_tx: found.max_stored_tx(),
            next_xid: found.next_xid,
        }),
    }
}

/// Renumber every event and consumer cursor so that they are consistent with
/// this cluster, keeping the stream order and every consumer's place in it.
///
/// All existing events get `tx = 0` (below every transaction id the cluster
/// will ever assign, the same value the commit-order migration gives
/// pre-existing rows) and `position` = their rank in the stream. Each cursor
/// becomes `(0, rank of the last event at or before it)`.
///
/// The stream order used for the ranks is, in three groups:
///
/// 1. events already at `tx = 0` (history from the migration or an earlier
///    rebase), by position;
/// 2. events with a foreign `tx` (at or above the cluster's next transaction
///    id), by `(tx, position)`: the restored stream;
/// 3. events with a `tx` this cluster could have assigned, by
///    `(tx, position)`: events published after the restore, which sorted
///    *behind* the restored cursors and were never delivered. Placing them
///    last delivers them.
///
/// A restored event whose old `tx` happens to be below the new cluster's
/// counter cannot be told apart from group 3 and lands there, so a consumer
/// may see it again. Delivery is at-least-once and handlers are
/// replay-idempotent, so that is slow rather than wrong.
///
/// Run with every Kruxia Flow process stopped: a consumer that polled before
/// the rebase and stores its cursor after it would write a stale cursor back.
/// The tables are locked for the duration, which waits for open writers.
/// Requires ownership of `workflow_events` (the role that runs migrations).
pub async fn rebase_event_xids(pool: &PgPool) -> Result<RebaseReport> {
    /// 0 = already rebased history, 1 = foreign, 2 = assigned by this cluster
    const GROUP: &str = "CASE WHEN {tx} = '0'::xid8 THEN 0 \
                         WHEN {tx} >= ($1::bigint)::text::xid8 THEN 1 ELSE 2 END";
    let group = |column: &str| GROUP.replace("{tx}", column);

    let mut txn = pool.begin().await?;

    sqlx::query("LOCK TABLE workflow_events, workflow_event_consumers IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *txn)
        .await?;

    let next_xid: i64 =
        sqlx::query_scalar("SELECT pg_snapshot_xmax(pg_current_snapshot())::text::bigint")
            .fetch_one(&mut *txn)
            .await?;

    // Cursors first, while the events still carry the values they refer to
    let consumers = sqlx::query(&format!(
        "UPDATE workflow_event_consumers c \
         SET last_tx = '0', \
             last_position = ( \
                 SELECT count(*) FROM workflow_events e \
                 WHERE ({event_group}, e.tx, e.position) \
                    <= ({cursor_group}, c.last_tx, c.last_position)), \
             updated_at = NOW()",
        event_group = group("e.tx"),
        cursor_group = group("c.last_tx"),
    ))
    .bind(next_xid)
    .execute(&mut *txn)
    .await?
    .rows_affected();

    // position is GENERATED ALWAYS and unique: lift both while renumbering
    for statement in [
        "ALTER TABLE workflow_events DROP CONSTRAINT workflow_events_position_key",
        "ALTER TABLE workflow_events ALTER COLUMN position SET GENERATED BY DEFAULT",
    ] {
        sqlx::query(statement).execute(&mut *txn).await?;
    }

    let events = sqlx::query(&format!(
        "UPDATE workflow_events e SET tx = '0', position = r.n \
         FROM (SELECT id, row_number() OVER (ORDER BY {event_group}, tx, position) AS n \
               FROM workflow_events) r \
         WHERE e.id = r.id",
        event_group = group("tx"),
    ))
    .bind(next_xid)
    .execute(&mut *txn)
    .await?
    .rows_affected();

    for statement in [
        "ALTER TABLE workflow_events ALTER COLUMN position SET GENERATED ALWAYS",
        "ALTER TABLE workflow_events ADD CONSTRAINT workflow_events_position_key UNIQUE (position)",
        "SELECT setval(pg_get_serial_sequence('workflow_events', 'position'), \
                       COALESCE((SELECT max(position) FROM workflow_events), 0) + 1, false)",
    ] {
        sqlx::query(statement).execute(&mut *txn).await?;
    }

    txn.commit().await?;

    Ok(RebaseReport { events, consumers })
}
