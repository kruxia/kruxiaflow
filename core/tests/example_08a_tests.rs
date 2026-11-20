//! Example 08a (rate-limited API calls) through the real orchestrator.
//!
//! The definition is stored by `WorkflowDefinitionRepository` and submitted by
//! `WorkflowService` with the documented input, so the `WorkflowCreated`
//! payload is the one production publishes. Each `ActivityCompleted` is
//! published the way a worker's completion arrives (outputs object, the
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
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

const EXAMPLE_08A: &str = include_str!("../../examples/08a-rate-limited-api-calls.yaml");

struct Run {
    pool: PgPool,
    wf: Uuid,
    es: Arc<dyn EventSource>,
    q: Arc<dyn ActivityQueue>,
    subs: Arc<dyn SubscriptionService>,
    cfg: OrchestratorConfig,
    loop_keys: Vec<String>,
    /// Resolved `url` parameter of each scheduled activity, in order
    urls: Vec<(String, Value)>,
}

impl Run {
    async fn submit(pool: PgPool, input: Value) -> Run {
        let def = WorkflowDefinition::from_yaml(EXAMPLE_08A).expect("example 08a parses");
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
            .submit_workflow(&def.name, None, input, None, None)
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
            urls: Vec::new(),
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

    /// Complete every pending activity (one at a time) until none is pending
    async fn run_to_end(&mut self) -> BTreeMap<String, u32> {
        let mut runs: BTreeMap<String, u32> = BTreeMap::new();
        for _ in 0..50 {
            let s = self.state().await;
            let Some(a) = s
                .activities
                .values()
                .find(|a| a.status == WorkflowActivityStatus::Pending)
            else {
                return runs;
            };
            let key = a.key.clone();
            let iteration = self.loop_keys.contains(&key).then_some(a.iteration as i32);
            let url: Value = sqlx::query_scalar(
                "SELECT parameters->'url' FROM activity_queue \
                 WHERE workflow_id = $1 AND activity_key = $2 \
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(self.wf)
            .bind(&key)
            .fetch_one(&self.pool)
            .await
            .expect("queued activity row");
            self.urls.push((key.clone(), url));
            *runs.entry(key.clone()).or_default() += 1;
            self.es
                .publish(NewWorkflowEvent {
                    workflow_id: self.wf,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    // `http_request` returns its result as the `response` output
                    // (worker/src/activities/http.rs), so the fake does too
                    payload: json!({ "outputs": { "response": { "status_code": 200 } } }),
                    iteration,
                })
                .await
                .unwrap();
            self.process_all().await;
        }
        panic!("example 08a did not settle: {runs:?}");
    }
}

async fn assert_pages(pool: PgPool, input: Value, pages: u32) {
    let mut run = Run::submit(pool, input).await;
    let runs = run.run_to_end().await;
    let s = run.state().await;
    let st: Vec<_> = s
        .activities
        .values()
        .map(|a| (a.key.clone(), a.status, a.error.clone()))
        .collect();
    assert_eq!(s.status.to_string(), "completed", "{st:?} runs {runs:?}");
    assert_eq!(runs.get("fetch_page"), Some(&pages), "{runs:?}");
    assert_eq!(runs.get("aggregate_results"), Some(&1), "{runs:?}");
    let fetched: Vec<_> = run
        .urls
        .iter()
        .filter(|(k, _)| k == "fetch_page")
        .map(|(_, u)| u.clone())
        .collect();
    let expected: Vec<_> = (0..pages)
        .map(|p| json!(format!("https://httpbin.org/get?page={p}")))
        .collect();
    assert_eq!(fetched, expected);

    // The email lists every page's response
    let text: Option<String> = sqlx::query_scalar(
        "SELECT parameters->'body'->>'Text' FROM activity_queue \
         WHERE workflow_id = $1 AND activity_key = 'aggregate_results' \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(run.wf)
    .fetch_one(&run.pool)
    .await
    .expect("queued aggregate_results row");
    let text = text.unwrap_or_default();
    assert_eq!(
        text.matches("status_code").count(),
        pages as usize,
        "aggregate_results text: {text}"
    );
}

/// The documented submission: `{"num_pages": 3}` fetches pages 0, 1, 2
#[sqlx::test(migrations = "../migrations")]
async fn test_example_08a_documented_input(pool: PgPool) {
    assert_pages(pool, json!({"num_pages": 3}), 3).await;
}

/// `num_pages` is read from the workflow input, not only defaulted
#[sqlx::test(migrations = "../migrations")]
async fn test_example_08a_num_pages_from_input(pool: PgPool) {
    assert_pages(pool, json!({"num_pages": 2}), 2).await;
}

/// No input: the YAML's `default(3)` applies
#[sqlx::test(migrations = "../migrations")]
async fn test_example_08a_default_num_pages(pool: PgPool) {
    assert_pages(pool, json!({}), 3).await;
}
