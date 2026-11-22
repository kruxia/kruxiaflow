//! A `wait_for_signal` step in a self-loop takes one signal per round, and a
//! replayed signal from an earlier round changes nothing.
//!
//! Events are processed by `process_workflow_event` directly, so a test can
//! process an event a second time the way a replay after a crash does.
mod common;

use kruxiaflow_core::events::{EventSource, NewWorkflowEvent, WorkflowEvent, WorkflowEventType};
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowActivityStatus, WorkflowState, load_materialized_state},
};
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowService,
};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// `poll` waits for a signal each round, three rounds
const SELF_LOOP: &str = r#"
name: signal_round_self_loop
activities:
  - key: start
    activity_name: http_request
  - key: poll
    activity_name: http_request
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 3600
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: poll
        conditions: ["{{ poll.status == 'completed' }}"]
"#;

struct Run {
    pool: PgPool,
    wf: Uuid,
    es: Arc<dyn EventSource>,
    q: Arc<dyn ActivityQueue>,
    subs: Arc<dyn SubscriptionService>,
    cfg: OrchestratorConfig,
}

impl Run {
    async fn submit(pool: PgPool, yaml: &str) -> Run {
        let def = WorkflowDefinition::from_yaml(yaml).expect("definition parses");
        WorkflowDefinitionRepository::new(pool.clone())
            .store(def.clone())
            .await
            .expect("store definition");
        let created = WorkflowService::new(pool.clone())
            .submit_workflow(&def.name, None, json!({}), None, None)
            .await
            .expect("submit workflow");
        let run = Run {
            es: Arc::new(common::settled_event_source(pool.clone())),
            q: Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default())),
            subs: Arc::new(PostgresSubscriptionService::new(pool.clone())),
            cfg: OrchestratorConfig::new(pool.clone()),
            wf: created.id,
            pool,
        };
        run.process_all().await;
        run
    }

    async fn process(&self, event: &WorkflowEvent) {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event, &self.es, &self.q, &self.subs, &self.cfg,
        )
        .await
        .expect("process event");
    }

    /// Process every event published so far, and return them
    async fn process_all(&self) -> Vec<WorkflowEvent> {
        let mut processed = Vec::new();
        loop {
            let events = self.es.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                return processed;
            }
            for e in events {
                self.process(&e).await;
                self.es
                    .update_position("orchestrator", e.cursor())
                    .await
                    .unwrap();
                processed.push(e);
            }
        }
    }

    async fn state(&self) -> WorkflowState {
        let mut tx = self.pool.begin().await.unwrap();
        let s = load_materialized_state(&mut tx, self.wf).await.unwrap();
        tx.commit().await.unwrap();
        s
    }

    /// Publish an event for `key` in loop round `round`, process it and
    /// whatever it leads to, and return the published event as processed
    async fn publish(
        &self,
        event_type: WorkflowEventType,
        key: &str,
        round: Option<i32>,
    ) -> WorkflowEvent {
        let payload = match event_type {
            WorkflowEventType::ActivitySignaled => {
                json!({ "event_name": "go", "signal_data": { "round": round } })
            }
            _ => json!({ "outputs": {} }),
        };
        self.es
            .publish(NewWorkflowEvent {
                workflow_id: self.wf,
                event_type,
                activity_key: Some(key.to_string()),
                payload,
                iteration: round,
            })
            .await
            .unwrap();
        self.process_all()
            .await
            .into_iter()
            .find(|e| e.event_type == event_type && e.activity_key.as_deref() == Some(key))
            .expect("published event processed")
    }

    /// The loop round of `key`'s open subscription, if it has one
    async fn subscription(&self, key: &str) -> Option<Option<i32>> {
        sqlx::query_scalar(
            "SELECT iteration FROM activity_event_subscriptions \
             WHERE workflow_id = $1 AND activity_key = $2",
        )
        .bind(self.wf)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }
}

/// Round 0's `ActivitySignaled`, processed again while round 1 waits, leaves
/// round 1's wait open: its subscription stays, and the step keeps waiting
#[sqlx::test(migrations = "../migrations")]
async fn test_replayed_signal_of_earlier_round_changes_nothing(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let run = Run::submit(pool, SELF_LOOP).await;
    run.publish(WorkflowEventType::ActivityCompleted, "start", None)
        .await;
    assert_eq!(run.subscription("poll").await, Some(Some(0)));

    // Round 0: signaled, then completed; round 1 waits
    let round0 = run
        .publish(WorkflowEventType::ActivitySignaled, "poll", Some(0))
        .await;
    assert_eq!(
        run.state().await.activities["poll"].status,
        WorkflowActivityStatus::Pending
    );
    run.publish(WorkflowEventType::ActivityCompleted, "poll", Some(0))
        .await;
    let poll = &run.state().await.activities["poll"];
    assert_eq!(poll.status, WorkflowActivityStatus::Waiting);
    assert_eq!(poll.iteration, 1);
    assert_eq!(run.subscription("poll").await, Some(Some(1)));

    // Replay round 0's signal
    run.process(&round0).await;

    let poll = &run.state().await.activities["poll"];
    assert_eq!(poll.status, WorkflowActivityStatus::Waiting);
    assert_eq!(poll.signal_data, Some(json!({ "round": 0 })));
    assert_eq!(run.subscription("poll").await, Some(Some(1)));

    // Round 1's own signal still ends round 1's wait
    run.publish(WorkflowEventType::ActivitySignaled, "poll", Some(1))
        .await;
    let poll = &run.state().await.activities["poll"];
    assert_eq!(poll.status, WorkflowActivityStatus::Pending);
    assert_eq!(poll.signal_data, Some(json!({ "round": 1 })));
}
