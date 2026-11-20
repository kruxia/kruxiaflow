//! `{{ACTIVITY.*}}` (#32) and `{{SIGNAL.*}}` (#37) through the real
//! orchestrator.
//!
//! Each definition is stored by `WorkflowDefinitionRepository` and submitted
//! by `WorkflowService`, so the `WorkflowCreated` payload is the one
//! production publishes. Each `ActivityCompleted` is published the way a
//! worker's completion arrives (outputs object, optional `cost_usd`, the
//! iteration of the queued activity) and processed by `process_workflow_event`.
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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

const EXAMPLE_07A: &str = include_str!("../../examples/07a-agentic-research-simple.yaml");

/// A loop step that runs three rounds and reports its `ACTIVITY.*` values
/// as parameters, plus an unbudgeted root step
const LOOP_PARAMETERS: &str = r#"
name: activity_context_parameters
activities:
  - key: step
    activity_name: http_request
    iteration_scoped: true
    iteration_limit: 3
    parameters:
      round: "{{ACTIVITY.iteration}}"
      label: "round {{ACTIVITY.iteration + 1}}"
      spent: "{{ACTIVITY.accumulated_cost_usd}}"
      left: "{{ACTIVITY.remaining_budget_usd}}"
    outputs:
      - result
    settings:
      budget:
        limit: 1.00
        action: continue
    depends_on:
      - activity_key: step

  - key: unbudgeted
    activity_name: http_request
    parameters:
      spent: "{{ACTIVITY.accumulated_cost_usd}}"
      left: "{{ACTIVITY.remaining_budget_usd}}"
    outputs:
      - result
"#;

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

/// A loop whose back-edge condition reads the looping step's own iteration,
/// and a step after it whose condition reads its own remaining budget
const LOOP_CONDITION: &str = r#"
name: activity_context_condition
activities:
  - key: after
    activity_name: http_request
    parameters:
      done: "{{step.iteration}}"
    outputs:
      - result
    settings:
      budget:
        limit: 1.00
        action: continue
    depends_on:
      - activity_key: step
        conditions:
          - "{{ step.iteration >= 2 }}"
          - "{{ ACTIVITY.remaining_budget_usd > 0.5 }}"

  - key: step
    activity_name: http_request
    iteration_scoped: true
    iteration_limit: 5
    parameters:
      round: "{{step.iteration}}"
    outputs:
      - result
    depends_on:
      - activity_key: step
        conditions:
          - "{{ ACTIVITY.iteration < 2 }}"
"#;

/// An LLM step whose prompt reads `ACTIVITY.*`: a retry resolves its
/// parameters again for the budget check
const LLM_RETRY: &str = r#"
name: activity_context_llm_retry
activities:
  - key: ask
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-haiku-4-5-20251001
      prompt: "Round {{ACTIVITY.iteration}}, spent ${{ACTIVITY.accumulated_cost_usd}}"
      max_tokens: 10
    outputs:
      - result
    settings:
      budget:
        limit: 1.00
        action: abort
"#;

struct Run {
    pool: PgPool,
    wf: Uuid,
    es: Arc<dyn EventSource>,
    q: Arc<dyn ActivityQueue>,
    subs: Arc<dyn SubscriptionService>,
    cfg: OrchestratorConfig,
    loop_keys: Vec<String>,
    /// Resolved parameters of each scheduled activity, in order
    params: Vec<(String, Value)>,
}

impl Run {
    async fn submit(pool: PgPool, yaml: &str) -> Run {
        let def = WorkflowDefinition::from_yaml(yaml).expect("definition parses");
        let loop_keys = def
            .activities
            .iter()
            .filter(|a| a.is_loop_activity)
            .map(|a| a.key.clone())
            .collect();
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
            loop_keys,
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

    /// Complete every pending activity (one at a time, in key order) until
    /// none is pending. `cost` gives the `cost_usd` each completion reports.
    async fn run_to_end(&mut self, cost: impl Fn(&str, usize) -> Option<&'static str>) {
        let mut runs: BTreeMap<String, usize> = BTreeMap::new();
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
            let iteration = self.loop_keys.contains(&key).then_some(a.iteration as i32);
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
            let round = runs.entry(key.clone()).or_default();
            let mut payload = json!({ "outputs": { "result": { "status_code": 200 } } });
            if let Some(c) = cost(&key, *round) {
                payload["cost_usd"] = json!(c);
            }
            *round += 1;
            self.es
                .publish(NewWorkflowEvent {
                    workflow_id: self.wf,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload,
                    iteration,
                })
                .await
                .unwrap();
            self.process_all().await;
        }
        panic!("workflow did not settle: {:?}", self.params);
    }

    /// Publish and process a worker failure of `key`
    async fn fail(&self, key: &str, payload: Value) {
        self.es
            .publish(NewWorkflowEvent {
                workflow_id: self.wf,
                event_type: WorkflowEventType::ActivityFailed,
                activity_key: Some(key.to_string()),
                payload,
                iteration: None,
            })
            .await
            .unwrap();
        self.process_all().await;
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

    async fn status(&self, key: &str) -> WorkflowActivityStatus {
        self.state().await.activities[key].status
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

/// `{{ACTIVITY.iteration}}` in a loop step's own parameters is the round
/// being scheduled: 0, 1 and 2 over three rounds
#[sqlx::test(migrations = "../migrations")]
async fn test_activity_iteration_in_loop_parameters(pool: PgPool) {
    let mut run = Run::submit(pool, LOOP_PARAMETERS).await;
    run.run_to_end(|_, _| None).await;
    assert_eq!(run.state().await.status.to_string(), "completed");
    assert_eq!(
        run.param("step", "round"),
        vec![json!(0), json!(1), json!(2)]
    );
    assert_eq!(
        run.param("step", "label"),
        vec![json!("round 1"), json!("round 2"), json!("round 3")]
    );
}

/// `ACTIVITY.accumulated_cost_usd` is what this activity reported in earlier
/// rounds; `ACTIVITY.remaining_budget_usd` is its `settings.budget.limit`
/// minus that, and null for an activity without a budget
#[sqlx::test(migrations = "../migrations")]
async fn test_activity_cost_and_budget_in_loop_parameters(pool: PgPool) {
    let mut run = Run::submit(pool, LOOP_PARAMETERS).await;
    run.run_to_end(|key, round| match (key, round) {
        ("step", 0) => Some("0.25"),
        ("step", 1) => Some("0.50"),
        ("step", _) => Some("0.10"),
        _ => Some("0.05"),
    })
    .await;
    assert_eq!(run.state().await.status.to_string(), "completed");
    assert_eq!(
        run.param("step", "spent"),
        vec![json!(0), json!(0.25), json!(0.75)]
    );
    assert_eq!(
        run.param("step", "left"),
        vec![json!(1), json!(0.75), json!(0.25)]
    );
    assert_eq!(run.param("unbudgeted", "spent"), vec![json!(0)]);
    assert_eq!(run.param("unbudgeted", "left"), vec![Value::Null]);
}

/// In a `depends_on` condition, `ACTIVITY` is the activity the condition
/// gates: on the back-edge, the looping step, which stops after rounds 0 and
/// 1; on `after`'s dependency, `after` itself, which has budget left
#[sqlx::test(migrations = "../migrations")]
async fn test_activity_context_in_conditions(pool: PgPool) {
    let mut run = Run::submit(pool, LOOP_CONDITION).await;
    run.run_to_end(|_, _| None).await;
    let s = run.state().await;
    assert_eq!(s.status.to_string(), "completed");
    assert_eq!(run.param("step", "round"), vec![json!(0), json!(1)]);
    assert_eq!(run.param("after", "done"), vec![json!(2)]);
}

/// A retried LLM step resolves its parameters again for the budget check;
/// `ACTIVITY.*` resolves there too, so the retry goes ahead
#[sqlx::test(migrations = "../migrations")]
async fn test_activity_context_in_llm_retry(pool: PgPool) {
    let run = Run::submit(pool, LLM_RETRY).await;
    let queued: Value = sqlx::query_scalar(
        "SELECT parameters->'prompt' FROM activity_queue WHERE workflow_id = $1",
    )
    .bind(run.wf)
    .fetch_one(&run.pool)
    .await
    .unwrap();
    assert_eq!(queued, json!("Round 0, spent $0.0"));

    run.fail(
        "ask",
        json!({"error": "Rate limited", "will_retry": true, "cost_usd": "0.10"}),
    )
    .await;
    let s = run.state().await;
    let ask = &s.activities["ask"];
    assert_eq!(
        ask.status,
        WorkflowActivityStatus::Pending,
        "{:?}",
        ask.error
    );
    assert_eq!(ask.attempt, 2);
}

/// Example 07a's `{{ACTIVITY.*}}` lines render for the activity that uses
/// them, with that activity's own budget from the example. Only those lines
/// are rendered: the rest of the prompts is outside this fix.
#[test]
fn test_example_07a_activity_templates_render() {
    let def = WorkflowDefinition::from_yaml(EXAMPLE_07A).expect("example 07a parses");
    let mut context = TemplateContext::new();
    for a in &def.activities {
        let cost = match a.key.as_str() {
            "perform_search" => Decimal::new(2, 2), // $0.02 over earlier rounds
            "evaluate" => Decimal::new(4, 3),       // $0.004
            _ => Decimal::ZERO,
        };
        context.add_activity_state(a.key.clone(), vec![], None, 1, cost, "completed".into());
    }

    let rendered = |key: &str| -> String {
        let a = def.activities.iter().find(|a| a.key == key).unwrap();
        let prompt = a.parameters.as_ref().unwrap()["prompt"].as_str().unwrap();
        let lines: Vec<&str> = prompt.lines().filter(|l| l.contains("ACTIVITY")).collect();
        assert_eq!(lines.len(), 2, "{key}: {lines:?}");
        let mut ctx = context.clone();
        ctx.set_current_activity(a);
        resolve_template_value(&Value::String(lines.join("\n")), &ctx)
            .unwrap_or_else(|e| panic!("{key}: {e}"))
            .as_str()
            .unwrap()
            .to_string()
    };

    assert_eq!(
        rendered("perform_search"),
        "Current iteration: 1\nAccumulated cost so far: $0.02"
    );
    // evaluate's budget is $0.01: $0.01 - $0.004
    assert_eq!(
        rendered("evaluate"),
        "Current iteration: 1\nRemaining budget: $0.006"
    );
}

/// The data a signal carries is `{{SIGNAL}}` in the parameters of the step
/// it resumes (#37), and a step that uses it still waits first
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
    run.run_to_end(|_, _| None).await;

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
        run.run_to_end(|_, _| None).await;
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
