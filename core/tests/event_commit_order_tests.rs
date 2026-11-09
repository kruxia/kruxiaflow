//! Delivery guarantees of the commit-safe event cursor
//! (docs/implementation/2026-11-08-commit-order-cursor-and-notify-wake.md).
//!
//! These tests open transactions on separate connections to reproduce commit
//! orderings that a single-connection test never sees. Assertions that a poll
//! returns *nothing* hold regardless of other tests on the instance (an open
//! transaction of this test is what blocks delivery); assertions that a poll
//! returns events first settle, because an unrelated test's open transaction
//! can hold delivery back for a moment (see `common`).

mod common;

use kruxiaflow_core::events::{
    EventSource, NewWorkflowEvent, PostgresEventSource, WorkflowEventType,
};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

const NIL: Uuid = Uuid::nil();

/// Insert an event on `conn` (inside whatever transaction it has open),
/// returning (id, position)
async fn insert_event(conn: &mut PgConnection, workflow_id: Uuid) -> (Uuid, i64) {
    sqlx::query_as(
        "INSERT INTO workflow_events (workflow_id, event_type, payload) \
         VALUES ($1, 'WorkflowCreated', '{}') RETURNING id, position",
    )
    .bind(workflow_id)
    .fetch_one(conn)
    .await
    .expect("insert event")
}

/// Publish an event through the event source (autocommit), returning its id
async fn publish(source: &PostgresEventSource, pool: &PgPool, workflow_id: Uuid) -> Uuid {
    source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("publish");
    sqlx::query_scalar("SELECT id FROM workflow_events WHERE workflow_id = $1")
        .bind(workflow_id)
        .fetch_one(pool)
        .await
        .expect("published id")
}

async fn ids_where_id_after(pool: &PgPool, after: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM workflow_events WHERE id > $1 ORDER BY id")
        .bind(after)
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn poll_ids(source: &PostgresEventSource, consumer: &str) -> Vec<Uuid> {
    source
        .poll(consumer)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect()
}

/// The 0.8.1 hazard: an event whose transaction commits after a later event
/// was read must still be delivered.
#[sqlx::test(migrations = "../migrations")]
async fn test_late_commit_is_not_skipped(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    // A inserts first (lower id, lower xid) and stays open
    let mut a = pool.begin().await.unwrap();
    let (a_id, _) = insert_event(&mut a, Uuid::now_v7()).await;

    // B inserts and commits
    let b_id = publish(&source, &pool, Uuid::now_v7()).await;
    assert!(a_id < b_id);

    // A pins xmin: B is not delivered yet
    assert!(poll_ids(&source, "c").await.is_empty());

    // The old predicate (id > cursor) returns B alone at this point...
    assert_eq!(ids_where_id_after(&pool, NIL).await, vec![b_id]);

    a.commit().await.unwrap();
    common::settle_events(&pool).await;

    // Both arrive, A first
    let events = source.poll("c").await.unwrap();
    assert_eq!(
        events.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![a_id, b_id]
    );

    // ...and a cursor advanced to B under the old predicate never sees A
    assert!(ids_where_id_after(&pool, b_id).await.is_empty());

    source
        .update_position("c", events[1].cursor())
        .await
        .unwrap();
    assert!(poll_ids(&source, "c").await.is_empty());
}

/// A transaction can take its xid before another one and insert its event
/// after it: a lower tx with a higher position. A cursor on position alone
/// would step over the earlier-positioned event; the (tx, position) cursor
/// delivers it.
#[sqlx::test(migrations = "../migrations")]
async fn test_lower_xid_with_higher_position_is_not_skipped(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    // T1 takes an xid first
    let mut t1 = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_current_xact_id()")
        .execute(&mut *t1)
        .await
        .unwrap();

    // T2 (higher xid) inserts E2 and stays open
    let mut t2 = pool.begin().await.unwrap();
    let (e2_id, e2_position) = insert_event(&mut t2, Uuid::now_v7()).await;

    // T1 inserts E1 after E2 (higher position) and commits
    let (e1_id, e1_position) = insert_event(&mut t1, Uuid::now_v7()).await;
    assert!(e1_position > e2_position);
    t1.commit().await.unwrap();

    // E1 is deliverable (its transaction is below xmin); E2 is not committed
    common::settle_events(&pool).await;
    let events = source.poll("c").await.unwrap();
    assert_eq!(events.iter().map(|e| e.id).collect::<Vec<_>>(), vec![e1_id]);
    source
        .update_position("c", events[0].cursor())
        .await
        .unwrap();

    t2.commit().await.unwrap();
    common::settle_events(&pool).await;

    // A position-only cursor at E1 would now be past E2 forever
    let past_by_position: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM workflow_events WHERE position > $1")
            .bind(e1_position)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(past_by_position.is_empty());

    // The (tx, position) cursor still delivers it
    assert_eq!(poll_ids(&source, "c").await, vec![e2_id]);
}

/// The documented cost: any open writing transaction on the instance that
/// started before an event's transaction delays that event until it ends,
/// even if it never touches workflow_events.
#[sqlx::test(migrations = "../migrations")]
async fn test_unrelated_open_transaction_delays_delivery(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO workflow_definitions (name, activities) VALUES ('blocker', '[]')")
        .execute(&mut *blocker)
        .await
        .unwrap();

    let event_id = publish(&source, &pool, Uuid::now_v7()).await;
    assert!(poll_ids(&source, "c").await.is_empty());

    blocker.commit().await.unwrap();
    common::settle_events(&pool).await;
    assert_eq!(poll_ids(&source, "c").await, vec![event_id]);
}

/// A read-only transaction takes no xid and blocks nothing
#[sqlx::test(migrations = "../migrations")]
async fn test_read_only_transaction_does_not_delay_delivery(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    let mut reader = pool.begin().await.unwrap();
    sqlx::query("SELECT count(*) FROM workflow_events")
        .execute(&mut *reader)
        .await
        .unwrap();

    let event_id = publish(&source, &pool, Uuid::now_v7()).await;
    common::settle_events(&pool).await;
    assert_eq!(poll_ids(&source, "c").await, vec![event_id]);

    reader.rollback().await.unwrap();
}

/// An event inserted inside a savepoint carries the top-level transaction's
/// id (`pg_current_xact_id()` never returns a subtransaction id), so it is
/// held back for as long as the outer transaction is open
#[sqlx::test(migrations = "../migrations")]
async fn test_savepoint_insert_carries_top_level_xid(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    let mut outer = pool.begin().await.unwrap();
    let top_xid: i64 = sqlx::query_scalar("SELECT pg_current_xact_id()::text::bigint")
        .fetch_one(&mut *outer)
        .await
        .unwrap();

    sqlx::query("SAVEPOINT s")
        .execute(&mut *outer)
        .await
        .unwrap();
    let event_tx: i64 = sqlx::query_scalar(
        "INSERT INTO workflow_events (workflow_id, event_type, payload) \
         VALUES ($1, 'WorkflowCreated', '{}') RETURNING tx::text::bigint",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&mut *outer)
    .await
    .unwrap();
    sqlx::query("RELEASE SAVEPOINT s")
        .execute(&mut *outer)
        .await
        .unwrap();
    assert_eq!(event_tx, top_xid);

    // Released savepoint, outer transaction still open: not delivered
    assert!(poll_ids(&source, "c").await.is_empty());

    outer.commit().await.unwrap();
    common::settle_events(&pool).await;
    assert_eq!(source.poll("c").await.unwrap().len(), 1);
}

/// WebSocket replay (`get_events_from_id`) has the same guarantee as poll
#[sqlx::test(migrations = "../migrations")]
async fn test_replay_from_id_includes_late_commit(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    let e0_id = publish(&source, &pool, Uuid::now_v7()).await;

    let mut a = pool.begin().await.unwrap();
    let a_workflow = Uuid::now_v7();
    let (a_id, _) = insert_event(&mut a, a_workflow).await;
    let b_workflow = Uuid::now_v7();
    let b_id = publish(&source, &pool, b_workflow).await;

    // B is held back while A is open
    let replay = source.get_events_from_id(e0_id, None, 100).await.unwrap();
    assert!(replay.is_empty());

    a.commit().await.unwrap();
    common::settle_events(&pool).await;

    let ids = |events: Vec<kruxiaflow_core::WorkflowEvent>| {
        events.into_iter().map(|e| e.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(source.get_events_from_id(e0_id, None, 100).await.unwrap()),
        vec![a_id, b_id]
    );
    assert_eq!(
        ids(source.get_events_from_id(a_id, None, 100).await.unwrap()),
        vec![b_id]
    );
    assert!(
        source
            .get_events_from_id(b_id, None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(source
            .get_events_from_id(e0_id, Some(&[b_workflow]), 100)
            .await
            .unwrap()),
        vec![b_id]
    );
    assert_eq!(
        ids(source.get_events_from_id(e0_id, None, 1).await.unwrap()),
        vec![a_id]
    );

    // An unknown id resolves to no cursor: empty replay
    assert!(
        source
            .get_events_from_id(Uuid::now_v7(), None, 100)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Positions increase with insert order; without conflicting inserts they
/// are consecutive. (An insert that hits ON CONFLICT DO NOTHING still
/// consumes an identity value, so positions are not gap-free in general.)
#[sqlx::test(migrations = "../migrations")]
async fn test_positions_increase(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());
    for _ in 0..3 {
        publish(&source, &pool, Uuid::now_v7()).await;
    }
    common::settle_events(&pool).await;

    let events = source.poll("c").await.unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[1].position, events[0].position + 1);
    assert_eq!(events[2].position, events[1].position + 1);
    assert!(events.windows(2).all(|w| w[0].cursor() < w[1].cursor()));
}

/// update_position never moves a cursor backwards
#[sqlx::test(migrations = "../migrations")]
async fn test_update_position_is_forward_only(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());
    for _ in 0..2 {
        publish(&source, &pool, Uuid::now_v7()).await;
    }
    common::settle_events(&pool).await;
    let events = source.poll("c").await.unwrap();
    assert_eq!(events.len(), 2);

    source
        .update_position("c", events[1].cursor())
        .await
        .unwrap();
    source
        .update_position("c", events[0].cursor())
        .await
        .unwrap();
    assert!(poll_ids(&source, "c").await.is_empty());
}

/// The migration backfills position in id order, carries consumer cursors
/// across, and its down migration restores them.
#[sqlx::test(migrations = false)]
async fn test_commit_order_migration_round_trip(pool: PgPool) {
    const BEFORE: i64 = 20261101000001;
    let migrator = sqlx::migrate!("../migrations");
    migrator.run(&pool).await.unwrap();
    migrator.undo(&pool, BEFORE).await.unwrap();

    // Old schema: insert events in an order different from id order
    let mut ids: Vec<Uuid> = (0..4).map(|_| Uuid::now_v7()).collect();
    ids.sort();
    for &i in &[2usize, 0, 3, 1] {
        sqlx::query(
            "INSERT INTO workflow_events (id, workflow_id, event_type, payload) \
             VALUES ($1, $2, 'WorkflowCreated', '{}')",
        )
        .bind(ids[i])
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO workflow_event_consumers (consumer_id, last_event_id) \
         VALUES ('c', $1), ('fresh', '00000000-0000-0000-0000-000000000000')",
    )
    .bind(ids[1])
    .execute(&pool)
    .await
    .unwrap();

    migrator.run(&pool).await.unwrap();

    // Positions follow id order
    let by_position: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM workflow_events ORDER BY position")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(by_position, ids);

    // The cursor carried over: 'c' resumes after ids[1], 'fresh' from the start
    let source = PostgresEventSource::new(pool.clone());
    common::settle_events(&pool).await;
    assert_eq!(poll_ids(&source, "c").await, ids[2..].to_vec());
    assert_eq!(poll_ids(&source, "fresh").await, ids);

    // New events sort after the backfilled ones
    let new_id = publish(&source, &pool, Uuid::now_v7()).await;
    common::settle_events(&pool).await;
    let events = source.poll("c").await.unwrap();
    assert_eq!(events.last().unwrap().id, new_id);

    // Down again: the cursor maps back to its event id
    source
        .update_position("c", events[0].cursor())
        .await
        .unwrap();
    migrator.undo(&pool, BEFORE).await.unwrap();
    let last: Uuid = sqlx::query_scalar(
        "SELECT last_event_id FROM workflow_event_consumers WHERE consumer_id = 'c'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(last, events[0].id);

    migrator.run(&pool).await.unwrap();
}
