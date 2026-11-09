//! Guard and repair for event cursors that outlive their database cluster
//! (`kruxiaflow_core::events::rebase`).
//!
//! A dump/restore or logical migration is simulated by shifting the stored
//! transaction ids far above anything this cluster has assigned.

mod common;

use kruxiaflow_core::events::{
    EventError, EventSource, EventWakeMode, NewWorkflowEvent, PostgresEventSource,
    WorkflowEventType, find_foreign_xids, rebase_event_xids, verify_event_xids,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Far above any transaction id a test cluster reaches
const FOREIGN_OFFSET: i64 = 1_000_000_000_000;

async fn publish(source: &PostgresEventSource, pool: &PgPool) -> Uuid {
    let workflow_id = Uuid::now_v7();
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

async fn poll_ids(source: &PostgresEventSource, consumer: &str) -> Vec<Uuid> {
    source
        .poll(consumer)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect()
}

/// Make the stored events and cursors look as if another cluster wrote them
/// (tx 0 is not a transaction id and would be restored unchanged)
async fn simulate_restore_into_new_cluster(pool: &PgPool) {
    sqlx::query(
        "UPDATE workflow_events SET tx = (tx::text::bigint + $1)::text::xid8 WHERE tx <> '0'",
    )
    .bind(FOREIGN_OFFSET)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE workflow_event_consumers \
         SET last_tx = (last_tx::text::bigint + $1)::text::xid8 WHERE last_tx <> '0'",
    )
    .bind(FOREIGN_OFFSET)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_healthy_database_passes_the_guard(pool: PgPool) {
    // Empty
    assert_eq!(find_foreign_xids(&pool).await.unwrap(), None);

    // With events and a cursor
    let source = PostgresEventSource::new(pool.clone());
    publish(&source, &pool).await;
    publish(&source, &pool).await;
    common::settle_events(&pool).await;
    let events = source.poll("c").await.unwrap();
    source
        .update_position("c", events[1].cursor())
        .await
        .unwrap();

    assert_eq!(find_foreign_xids(&pool).await.unwrap(), None);
    verify_event_xids(&pool).await.unwrap();
    assert!(
        PostgresEventSource::connect(pool.clone(), EventWakeMode::Poll)
            .await
            .is_ok()
    );

    // A rebase of a healthy database keeps order and cursors
    let ids: Vec<Uuid> = events.iter().map(|e| e.id).collect();
    let report = rebase_event_xids(&pool).await.unwrap();
    assert_eq!((report.events, report.consumers), (2, 1));
    assert!(poll_ids(&source, "c").await.is_empty());
    assert_eq!(poll_ids(&source, "fresh").await, ids);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_foreign_cluster_is_detected_and_rebased(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());

    // Four events; "orch" has processed the first two, "idle" none
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(publish(&source, &pool).await);
    }
    common::settle_events(&pool).await;
    let events = source.poll("orch").await.unwrap();
    assert_eq!(events.iter().map(|e| e.id).collect::<Vec<_>>(), ids);
    source
        .update_position("orch", events[1].cursor())
        .await
        .unwrap();
    sqlx::query("INSERT INTO workflow_event_consumers (consumer_id) VALUES ('idle')")
        .execute(&pool)
        .await
        .unwrap();

    simulate_restore_into_new_cluster(&pool).await;

    // Detected, and a consuming process refuses to start
    let found = find_foreign_xids(&pool)
        .await
        .unwrap()
        .expect("foreign transaction ids");
    assert!(found.max_event_tx.unwrap() >= FOREIGN_OFFSET);
    assert!(found.max_cursor_tx.unwrap() >= FOREIGN_OFFSET);
    assert!(found.next_xid < FOREIGN_OFFSET);
    assert!(matches!(
        verify_event_xids(&pool).await,
        Err(EventError::ForeignCluster { .. })
    ));
    let refused = PostgresEventSource::connect(pool.clone(), EventWakeMode::Poll).await;
    match refused {
        Err(e @ EventError::ForeignCluster { .. }) => {
            assert!(e.to_string().contains("kruxiaflow rebase-events"));
        }
        other => panic!("expected ForeignCluster, got {:?}", other.map(|_| ())),
    }

    // The hazard the guard exists for: the restored events are held back
    // (their tx is above xmin), and an event published now sorts behind the
    // restored cursor. Nothing is delivered, with no error anywhere.
    let published_after_restore = publish(&source, &pool).await;
    assert!(poll_ids(&source, "orch").await.is_empty());

    let report = rebase_event_xids(&pool).await.unwrap();
    assert_eq!((report.events, report.consumers), (5, 2));
    assert_eq!(find_foreign_xids(&pool).await.unwrap(), None);
    assert!(
        PostgresEventSource::connect(pool.clone(), EventWakeMode::Poll)
            .await
            .is_ok()
    );

    // "orch" resumes after the two it had processed, and the event published
    // after the restore comes last rather than being lost
    common::settle_events(&pool).await;
    let resumed = source.poll("orch").await.unwrap();
    assert_eq!(
        resumed.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![ids[2], ids[3], published_after_restore]
    );
    // Positions are the ranks in the stream
    assert_eq!(
        resumed.iter().map(|e| e.position).collect::<Vec<_>>(),
        vec![3, 4, 5]
    );

    // "idle" still starts from the beginning, in the original order
    let mut all = ids.clone();
    all.push(published_after_restore);
    assert_eq!(poll_ids(&source, "idle").await, all);

    // New events continue the stream
    source
        .update_position("orch", resumed[2].cursor())
        .await
        .unwrap();
    let next = publish(&source, &pool).await;
    common::settle_events(&pool).await;
    let events = source.poll("orch").await.unwrap();
    assert_eq!(events.iter().map(|e| e.id).collect::<Vec<_>>(), vec![next]);
    assert_eq!(events[0].position, 6);

    // Replay from an id still resolves
    let replay = source.get_events_from_id(ids[3], None, 100).await.unwrap();
    assert_eq!(
        replay.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![published_after_restore, next]
    );
}

/// History already at tx 0 (from the migration or an earlier rebase) stays
/// first, ahead of restored events with real transaction ids
#[sqlx::test(migrations = "../migrations")]
async fn test_rebase_keeps_tx_zero_history_first(pool: PgPool) {
    let source = PostgresEventSource::new(pool.clone());
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(publish(&source, &pool).await);
    }
    // The first two are pre-cursor history; the consumer is past them
    sqlx::query("UPDATE workflow_events SET tx = '0' WHERE id = ANY($1)")
        .bind(&ids[..2])
        .execute(&pool)
        .await
        .unwrap();
    common::settle_events(&pool).await;
    let events = source.poll("c").await.unwrap();
    assert_eq!(events.iter().map(|e| e.id).collect::<Vec<_>>(), ids);
    source
        .update_position("c", events[1].cursor())
        .await
        .unwrap();

    // Only the third event carries a real transaction id; make it foreign
    simulate_restore_into_new_cluster(&pool).await;
    assert!(find_foreign_xids(&pool).await.unwrap().is_some());

    rebase_event_xids(&pool).await.unwrap();

    assert_eq!(poll_ids(&source, "c").await, vec![ids[2]]);
    assert_eq!(poll_ids(&source, "fresh").await, ids);
}
