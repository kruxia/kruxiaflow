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
use kruxiaflow_core::{
    PostgresSubscriptionService, SubscriptionService, TemplateContext, resolve_template_value,
};
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
            params: Vec::new(),
        };
        run.process_all().await;
        run
    }

    async fn process_all(&self) {
        loop {
            let events = self.es.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                break;
            }
            for e in events {
                kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                    &e, &self.es, &self.q, &self.subs, &self.cfg,
                )
                .await
                .expect("process event");
                self.es
                    .update_position("orchestrator", e.cursor())
                    .await
                    .unwrap();
            }
        }
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
/// undefined, and using it fails that step
#[test]
fn test_signal_is_undefined_without_a_wait() {
    let context = TemplateContext::new();
    let value = Value::String("{{SIGNAL.approved}}".to_string());
    assert!(resolve_template_value(&value, &context).is_err());
}
