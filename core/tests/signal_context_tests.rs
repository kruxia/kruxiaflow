//! `{{SIGNAL.*}}` through the real orchestrator (#37).
//!
//! Each definition is stored by `WorkflowDefinitionRepository` and submitted
//! by `WorkflowService`, so the `WorkflowCreated` payload is the one
//! production publishes. An `ActivitySignaled` is published with the payload
//! the signal API (or the expiry checker) publishes, and each
//! `ActivityCompleted` the way a worker's completion arrives; both are
//! processed by `process_workflow_event`.
mod common;

use kruxiaflow_core::events::{EventSource, NewWorkflowEvent, WorkflowEventType};
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowActivityStatus, WorkflowState, load_materialized_state},
};
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowService,
};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// A step that waits for a signal and reads its data, and a step after it
const SIGNAL_PARAMETERS: &str = r#"
name: signal_context_parameters
activities:
  - key: approval
    activity_name: http_request
    parameters:
      approved: "{{SIGNAL.approved}}"
      note: "approved by {{SIGNAL.approver}}"
    outputs:
      - result
    settings:
      wait_for_signal:
        event_name: approve
        timeout_seconds: 3600
        on_timeout: continue

  - key: after
    activity_name: http_request
    parameters:
      status: "{{approval.result.status_code}}"
    outputs:
      - result
    depends_on:
      - approval
"#;

/// A step that waits for a signal and only asks whether it got data
const SIGNAL_PRESENCE: &str = r#"
name: signal_context_presence
activities:
  - key: approval
    activity_name: http_request
    parameters:
      no_data: "{{SIGNAL is none}}"
    outputs:
      - result
    settings:
      wait_for_signal:
        event_name: approve
        timeout_seconds: 3600
        on_timeout: continue
"#;

struct Run {
    pool: PgPool,
    wf: Uuid,
    es: Arc<dyn EventSource>,
    q: Arc<dyn ActivityQueue>,
    subs: Arc<dyn SubscriptionService>,
    cfg: OrchestratorConfig,
    /// Resolved parameters of each scheduled activity, in order
    params: Vec<(String, Value)>,
}

impl Run {
    async fn submit(pool: PgPool, yaml: &str) -> Run {
        let run = Run::create(pool, yaml).await;
        run.process_all().await;
        run
    }

    /// Store and submit the workflow, without processing any event
    async fn create(pool: PgPool, yaml: &str) -> Run {
        let def = WorkflowDefinition::from_yaml(yaml).expect("definition parses");
        WorkflowDefinitionRepository::new(pool.clone())
            .store(def.clone())
            .await
            .expect("store definition");
        let created = WorkflowService::new(pool.clone())
            .submit_workflow(&def.name, None, json!({}), None, None)
            .await
            .expect("submit workflow");
        Run {
            es: Arc::new(common::settled_event_source(pool.clone())),
            q: Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default())),
            subs: Arc::new(PostgresSubscriptionService::new(pool.clone())),
            cfg: OrchestratorConfig::new(pool.clone()),
            wf: created.id,
            pool,
            params: Vec::new(),
        }
    }

    async fn process_all(&self) {
        self.try_process_all().await.expect("process event");
    }

    /// Process every event published so far; stop at the first that fails
    async fn try_process_all(&self) -> Result<(), String> {
        loop {
            let events = self.es.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                return Ok(());
            }
            for e in events {
                kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                    &e, &self.es, &self.q, &self.subs, &self.cfg,
                )
                .await
                .map_err(|err| format!("{:?} for {:?}: {err}", e.event_type, e.activity_key))?;
                self.es
                    .update_position("orchestrator", e.cursor())
                    .await
                    .unwrap();
            }
        }
    }

    /// Publish an event for `key` in loop round `round`, and process it
    async fn publish(
        &self,
        event_type: WorkflowEventType,
        key: &str,
        payload: Value,
        round: Option<i32>,
    ) {
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
        self.process_all().await;
    }

    /// The parameters `key` was last queued with
    async fn queued(&self, key: &str) -> Value {
        sqlx::query_scalar(
            "SELECT parameters FROM activity_queue \
             WHERE workflow_id = $1 AND activity_key = $2 \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(self.wf)
        .bind(key)
        .fetch_one(&self.pool)
        .await
        .expect("queued activity row")
    }

    async fn state(&self) -> WorkflowState {
        let mut tx = self.pool.begin().await.unwrap();
        let s = load_materialized_state(&mut tx, self.wf).await.unwrap();
        tx.commit().await.unwrap();
        s
    }

    async fn status(&self, key: &str) -> WorkflowActivityStatus {
        self.state().await.activities[key].status
    }

    /// Complete every pending activity (one at a time, in key order) until
    /// none is pending, recording the parameters each was queued with
    async fn run_to_end(&mut self) {
        for _ in 0..50 {
            let s = self.state().await;
            let mut pending: Vec<_> = s
                .activities
                .values()
                .filter(|a| a.status == WorkflowActivityStatus::Pending)
                .collect();
            pending.sort_by(|a, b| a.key.cmp(&b.key));
            let Some(a) = pending.first() else {
                return;
            };
            let key = a.key.clone();
            let params: Value = sqlx::query_scalar(
                "SELECT parameters FROM activity_queue \
                 WHERE workflow_id = $1 AND activity_key = $2 \
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(self.wf)
            .bind(&key)
            .fetch_one(&self.pool)
            .await
            .expect("queued activity row");
            self.params.push((key.clone(), params));
            self.es
                .publish(NewWorkflowEvent {
                    workflow_id: self.wf,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload: json!({ "outputs": { "result": { "status_code": 200 } } }),
                    iteration: None,
                })
                .await
                .unwrap();
            self.process_all().await;
        }
        panic!("workflow did not settle: {:?}", self.params);
    }

    /// Publish and process an `ActivitySignaled` for `key`, with the payload
    /// the signal API (or the expiry checker) publishes
    async fn signal(&self, key: &str, payload: Value) {
        self.es
            .publish(NewWorkflowEvent {
                workflow_id: self.wf,
                event_type: WorkflowEventType::ActivitySignaled,
                activity_key: Some(key.to_string()),
                payload,
                iteration: None,
            })
            .await
            .unwrap();
        self.process_all().await;
    }

    /// The values of parameter `name` that `key` was scheduled with, in order
    fn param(&self, key: &str, name: &str) -> Vec<Value> {
        self.params
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, p)| p[name].clone())
            .collect()
    }
}

/// The data a signal carries is `{{SIGNAL}}` in the parameters of the step
/// it resumes, and a step that uses it still waits first
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_data_in_parameters(pool: PgPool) {
    let mut run = Run::submit(pool, SIGNAL_PARAMETERS).await;
    // Its templates aren't resolved while it waits, so `SIGNAL` isn't needed yet
    assert_eq!(
        run.status("approval").await,
        WorkflowActivityStatus::Waiting
    );

    run.signal(
        "approval",
        json!({
            "event_name": "approve",
            "signal_data": { "approved": true, "approver": "ana" }
        }),
    )
    .await;
    run.run_to_end().await;

    assert_eq!(run.param("approval", "approved"), vec![json!(true)]);
    assert_eq!(
        run.param("approval", "note"),
        vec![json!("approved by ana")]
    );
    assert_eq!(run.param("after", "status"), vec![json!(200)]);
    let s = run.state().await;
    assert_eq!(
        s.activities["after"].status,
        WorkflowActivityStatus::Completed
    );
}

/// A signal without data, and a timeout that continues, give `SIGNAL` as
/// null, not undefined
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_without_data_is_null(pool: PgPool) {
    for payload in [
        json!({ "event_name": "approve", "signal_data": null }),
        json!({ "event_name": "approve" }),
        json!({ "reason": "timeout", "on_timeout": "continue" }),
    ] {
        let mut run = Run::submit(pool.clone(), SIGNAL_PRESENCE).await;
        assert_eq!(
            run.status("approval").await,
            WorkflowActivityStatus::Waiting
        );
        run.signal("approval", payload.clone()).await;
        run.run_to_end().await;
        assert_eq!(
            run.param("approval", "no_data"),
            vec![json!(true)],
            "{payload}"
        );
    }
}

/// `SIGNAL` belongs to steps that wait for a signal: anywhere else it is
/// undefined, and using it fails that step's scheduling
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_is_undefined_without_a_wait(pool: PgPool) {
    let run = Run::create(
        pool,
        r#"
name: signal_context_no_wait
activities:
  - key: step
    activity_name: http_request
    parameters:
      approved: "{{SIGNAL.approved}}"
"#,
    )
    .await;
    let err = run
        .try_process_all()
        .await
        .expect_err("a step without a wait resolved {{SIGNAL.approved}}");
    assert!(
        err.contains("Failed to resolve templates in activity step"),
        "{err}"
    );
}

/// A retried step that waited still sees its signal's data: the retry's
/// budget check resolves an LLM step's parameters again
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_data_on_retry(pool: PgPool) {
    let run = Run::submit(
        pool,
        r#"
name: signal_context_retry
activities:
  - key: ask
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-haiku-4-5-20251001
      prompt: "{{SIGNAL.question}}"
    settings:
      retry:
        max_attempts: 3
      wait_for_signal:
        event_name: go
        timeout_seconds: 3600
"#,
    )
    .await;
    assert_eq!(run.status("ask").await, WorkflowActivityStatus::Waiting);
    run.publish(
        WorkflowEventType::ActivitySignaled,
        "ask",
        json!({ "event_name": "go", "signal_data": { "question": "why?" } }),
        None,
    )
    .await;
    assert_eq!(run.queued("ask").await["prompt"], json!("why?"));

    run.publish(
        WorkflowEventType::ActivityFailed,
        "ask",
        json!({ "error_message": "provider down", "will_retry": true, "attempt": 1 }),
        None,
    )
    .await;
    let s = run.state().await;
    assert_eq!(s.activities["ask"].status, WorkflowActivityStatus::Pending);
    assert_eq!(s.activities["ask"].attempt, 2);
}

/// `delay` can come from the signal's data
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_data_in_delay(pool: PgPool) {
    let run = Run::submit(
        pool,
        r#"
name: signal_context_delay
activities:
  - key: later
    activity_name: http_request
    settings:
      delay: "{{SIGNAL.delay}}"
      wait_for_signal:
        event_name: go
        timeout_seconds: 3600
"#,
    )
    .await;
    run.publish(
        WorkflowEventType::ActivitySignaled,
        "later",
        json!({ "event_name": "go", "signal_data": { "delay": "2h" } }),
        None,
    )
    .await;
    let in_future: bool = sqlx::query_scalar(
        "SELECT scheduled_for > NOW() + INTERVAL '1 hour' FROM activity_queue \
         WHERE workflow_id = $1 AND activity_key = 'later'",
    )
    .bind(run.wf)
    .fetch_one(&run.pool)
    .await
    .unwrap();
    assert!(in_future, "the signal's delay was not applied");
}

/// In a loop, each round's step sees its own round's signal data
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_data_per_round(pool: PgPool) {
    let run = Run::submit(
        pool,
        r#"
name: signal_context_rounds
activities:
  - key: start
    activity_name: http_request
  - key: poll
    activity_name: http_request
    parameters:
      n: "{{SIGNAL.n}}"
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 3600
    iteration_limit: 2
    depends_on:
      - start
      - activity_key: poll
        conditions: ["{{ poll.status == 'completed' }}"]
"#,
    )
    .await;
    run.publish(
        WorkflowEventType::ActivityCompleted,
        "start",
        json!({ "outputs": {} }),
        None,
    )
    .await;
    let mut seen = Vec::new();
    for round in 0..2 {
        assert_eq!(
            run.status("poll").await,
            WorkflowActivityStatus::Waiting,
            "round {round}"
        );
        run.publish(
            WorkflowEventType::ActivitySignaled,
            "poll",
            json!({ "event_name": "go", "signal_data": { "n": round + 1 } }),
            Some(round),
        )
        .await;
        seen.push(run.queued("poll").await["n"].clone());
        run.publish(
            WorkflowEventType::ActivityCompleted,
            "poll",
            json!({ "outputs": {} }),
            Some(round),
        )
        .await;
    }
    assert_eq!(seen, vec![json!(1), json!(2)]);
}
