//! A step that begins to wait for a signal opens its subscription, then
//! publishes `ActivityWaiting`. When the pass stops between the two, the
//! subscription is committed but the workflow's state is not, and the event
//! that made the step ready is processed again: that replay finds the step's
//! subscription already open, and the step waits on it.
//!
//! The pass is stopped by an event source that refuses the `ActivityWaiting`
//! publish once, which leaves the database as a crash at that point would.
mod common;

use async_trait::async_trait;
use kruxiaflow_core::events::{
    EventCursor, EventError, EventSource, NewWorkflowEvent, WorkflowEvent, WorkflowEventType,
};
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    orchestrator::process_workflow_event,
    workflow_state::{WorkflowActivityStatus, WorkflowState, load_materialized_state},
};
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowService,
};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// `gate` waits for a signal once `start` completes
const GATE: &str = r#"
name: wait_replay_gate
activities:
  - key: start
    activity_name: http_request
  - key: gate
    activity_name: http_request
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 3600
    depends_on:
      - start
"#;

/// `poll` waits for a signal each round, three rounds
const SELF_LOOP: &str = r#"
name: wait_replay_self_loop
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

/// An event source that refuses the first publish of one event type, then
/// delegates everything to the inner source
struct RefuseOnce {
    inner: Arc<dyn EventSource>,
    refuse: Mutex<Option<WorkflowEventType>>,
}

#[async_trait]
impl EventSource for RefuseOnce {
    async fn publish(&self, event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
        let refuse = {
            let mut refuse = self.refuse.lock().unwrap();
            if *refuse == Some(event.event_type) {
                refuse.take()
            } else {
                None
            }
        };
        if refuse.is_some() {
            return Err(EventError::Invalid("engine stopped".to_string()));
        }
        self.inner.publish(event).await
    }

    async fn poll(&self, consumer_id: &str) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
        self.inner.poll(consumer_id).await
    }

    async fn update_position(
        &self,
        consumer_id: &str,
        cursor: EventCursor,
    ) -> kruxiaflow_core::events::Result<()> {
        self.inner.update_position(consumer_id, cursor).await
    }
}

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

    /// Process every event published so far
    async fn process_all(&self) {
        loop {
            let events = self.es.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                return;
            }
            for e in events {
                process_workflow_event(&e, &self.es, &self.q, &self.subs, &self.cfg)
                    .await
                    .expect("process event");
                self.es
                    .update_position("orchestrator", e.cursor())
                    .await
                    .unwrap();
            }
        }
    }

    /// Publish an event for `key` in loop round `round`, without processing it
    async fn publish(&self, event_type: WorkflowEventType, key: &str, round: Option<i32>) {
        let payload = match event_type {
            WorkflowEventType::ActivitySignaled => json!({ "event_name": "go", "signal_data": {} }),
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
    }

    /// Process the next event with the `ActivityWaiting` publish refused, as
    /// if the engine stopped there, and leave the cursor before it
    async fn process_next_stopping_at_waiting(&self) {
        let events = self.es.poll("orchestrator").await.unwrap();
        assert_eq!(events.len(), 1, "one event to process");
        let stopping: Arc<dyn EventSource> = Arc::new(RefuseOnce {
            inner: self.es.clone(),
            refuse: Mutex::new(Some(WorkflowEventType::ActivityWaiting)),
        });
        let result =
            process_workflow_event(&events[0], &stopping, &self.q, &self.subs, &self.cfg).await;
        assert!(
            result.is_err(),
            "the pass stops at the ActivityWaiting publish"
        );
    }

    async fn state(&self) -> WorkflowState {
        let mut tx = self.pool.begin().await.unwrap();
        let s = load_materialized_state(&mut tx, self.wf).await.unwrap();
        tx.commit().await.unwrap();
        s
    }

    /// The loop round of `key`'s subscription, if it has one
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

/// The event that made `gate` ready, processed again after the pass stopped
/// between opening the subscription and publishing `ActivityWaiting`, leaves
/// `gate` waiting on that subscription, and a signal then runs it
#[sqlx::test(migrations = "../migrations")]
async fn test_replay_after_stop_at_waiting_publish_waits(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let run = Run::submit(pool, GATE).await;

    run.publish(WorkflowEventType::ActivityCompleted, "start", None)
        .await;
    run.process_next_stopping_at_waiting().await;
    assert_eq!(
        run.subscription("gate").await,
        Some(None),
        "the subscription is committed"
    );
    assert_ne!(
        run.state().await.activities["gate"].status,
        WorkflowActivityStatus::Waiting,
        "the state is not"
    );

    // Restart: the same event is processed again
    run.process_all().await;
    let gate = &run.state().await.activities["gate"];
    assert_eq!(
        gate.status,
        WorkflowActivityStatus::Waiting,
        "error: {:?}",
        gate.error
    );
    assert_eq!(run.subscription("gate").await, Some(None));

    run.publish(WorkflowEventType::ActivitySignaled, "gate", None)
        .await;
    run.process_all().await;
    assert_eq!(
        run.state().await.activities["gate"].status,
        WorkflowActivityStatus::Pending
    );
}

/// The same inside a loop: round 1's wait, stopped at its `ActivityWaiting`
/// publish and replayed, waits on round 1's subscription
#[sqlx::test(migrations = "../migrations")]
async fn test_replay_after_stop_at_waiting_publish_waits_in_loop_round(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    let run = Run::submit(pool, SELF_LOOP).await;
    run.publish(WorkflowEventType::ActivityCompleted, "start", None)
        .await;
    run.process_all().await;
    run.publish(WorkflowEventType::ActivitySignaled, "poll", Some(0))
        .await;
    run.process_all().await;

    run.publish(WorkflowEventType::ActivityCompleted, "poll", Some(0))
        .await;
    run.process_next_stopping_at_waiting().await;
    assert_eq!(run.subscription("poll").await, Some(Some(1)));

    run.process_all().await;
    let poll = &run.state().await.activities["poll"];
    assert_eq!(
        poll.status,
        WorkflowActivityStatus::Waiting,
        "error: {:?}",
        poll.error
    );
    assert_eq!(poll.iteration, 1);
    assert_eq!(run.subscription("poll").await, Some(Some(1)));
}
