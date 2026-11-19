//! Every example in `examples/`, run end to end through the real orchestrator
//! with fake activity results.
//!
//! Each example is stored by `WorkflowDefinitionRepository` and submitted by
//! `WorkflowService` with the input its README entry or header documents.
//! Activities are claimed from the real queue and completed through
//! `ActivityWorkerService`, the service the worker API calls, so every
//! completion carries the iteration of the queued row, as a worker's does.
//! Only the activity bodies are fake: each result has the shape the std
//! activity returns (`http_request` → `response`, `echo` → `echo`,
//! `llm_prompt`/`embedding`/`postgres_*` → `result`). Delayed or scheduled
//! activities are made due at once, after recording the delay.
//!
//! The assertions state what each example intends. A failing test is a
//! finding: its message carries the run log, each activity's final status
//! and error, and the resolved parameters of every activity that ran.
mod common;

use kruxiaflow_core::ActivityWorkerService;
use kruxiaflow_core::PendingActivityRecord;
use kruxiaflow_core::events::EventSource;
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowState, load_materialized_state},
};
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowService,
};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

const DB_URL: &str = "postgres://app:secret@db.invalid:5432/app";
const WORKER_ID: &str = "sweep-worker";

/// A fake activity's result
enum Reply {
    /// Outputs object (name → value) and cost
    Done(Value, Option<Decimal>),
    /// Failure with an error message, retryable or not
    Fail(String, bool),
}

/// One activity execution the harness performed
struct Ran {
    key: String,
    round: u32,
    params: Value,
}

/// How a run ended
struct Report {
    example: String,
    workflow_status: String,
    error_message: Option<String>,
    /// key → (status, error, iteration)
    activities: BTreeMap<String, (String, Option<String>, u32)>,
    /// key → number of executions
    runs: BTreeMap<String, u32>,
    ran: Vec<Ran>,
    /// Activities that were queued for later: (key, queued iteration,
    /// seconds from now)
    delays: Vec<(String, Option<i32>, f64)>,
    /// True if the run stopped with nothing claimable and the workflow
    /// still not terminal
    hung: bool,
}

impl Report {
    fn runs(&self, key: &str) -> u32 {
        self.runs.get(key).copied().unwrap_or(0)
    }

    /// Resolved parameters of the `n`th execution (0-based) of `key`
    fn params(&self, key: &str, n: usize) -> &Value {
        self.ran
            .iter()
            .filter(|r| r.key == key)
            .nth(n)
            .map(|r| &r.params)
            .unwrap_or(&Value::Null)
    }

    fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "\n===== {}: workflow {}{}{}",
            self.example,
            self.workflow_status,
            if self.hung { " (HUNG)" } else { "" },
            self.error_message
                .as_ref()
                .map(|e| format!(" error={e}"))
                .unwrap_or_default()
        );
        for (k, (s, e, it)) in &self.activities {
            let _ = writeln!(
                out,
                "  {k}: {s} iteration={it} runs={}{}",
                self.runs(k),
                e.as_ref()
                    .map(|e| format!(" error={e}"))
                    .unwrap_or_default()
            );
        }
        for (k, it, secs) in &self.delays {
            let _ = writeln!(out, "  delayed {k} (iteration {it:?}): due in {secs:.0}s");
        }
        for r in &self.ran {
            let mut p = r.params.to_string();
            if p.len() > 700 {
                p.truncate(700);
                p.push_str("...");
            }
            let _ = writeln!(out, "  ran {} round {}: {p}", r.key, r.round);
        }
        out
    }
}

struct Sweep {
    example: String,
    pool: PgPool,
    wf: Uuid,
    es: Arc<dyn EventSource>,
    q: Arc<dyn ActivityQueue>,
    subs: Arc<dyn SubscriptionService>,
    cfg: OrchestratorConfig,
    svc: ActivityWorkerService,
}

impl Sweep {
    async fn submit(pool: PgPool, example: &str, yaml: &str, input: Value) -> Sweep {
        let def = WorkflowDefinition::from_yaml(yaml)
            .unwrap_or_else(|e| panic!("{example} does not parse: {e}"));
        WorkflowDefinitionRepository::new(pool.clone())
            .store(def.clone())
            .await
            .expect("store definition");
        let created = WorkflowService::new(pool.clone())
            .submit_workflow(&def.name, None, input, None, None)
            .await
            .expect("submit workflow");
        let es: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
        let q: Arc<dyn ActivityQueue> =
            Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
        let secrets = HashMap::from([("db_url".to_string(), DB_URL.to_string())]);
        let sweep = Sweep {
            example: example.to_string(),
            svc: ActivityWorkerService::new(q.clone(), es.clone()),
            subs: Arc::new(PostgresSubscriptionService::new(pool.clone())),
            cfg: OrchestratorConfig::new(pool.clone()).with_secrets(secrets),
            wf: created.id,
            es,
            q,
            pool,
        };
        sweep.process_all().await;
        sweep
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

    /// Claim and complete activities until nothing is claimable
    async fn drive<F>(self, mut respond: F) -> Report
    where
        F: FnMut(&PendingActivityRecord, u32) -> Reply,
    {
        let mut runs: BTreeMap<String, u32> = BTreeMap::new();
        let mut ran = Vec::new();
        let mut delays = Vec::new();
        for _ in 0..200 {
            self.process_all().await;
            // Record, then remove, any delay (settings.delay, scheduled_for,
            // retry backoff)
            let later: Vec<(String, Option<i32>, f64)> = sqlx::query_as(
                "SELECT activity_key, iteration, \
                 EXTRACT(EPOCH FROM scheduled_for - now())::float8 \
                 FROM activity_queue WHERE workflow_id = $1 \
                 AND status = 'pending'::activity_status AND scheduled_for > now()",
            )
            .bind(self.wf)
            .fetch_all(&self.pool)
            .await
            .unwrap();
            if !later.is_empty() {
                delays.extend(later);
                sqlx::query(
                    "UPDATE activity_queue SET scheduled_for = now() \
                     WHERE workflow_id = $1 AND status = 'pending'::activity_status",
                )
                .bind(self.wf)
                .execute(&self.pool)
                .await
                .unwrap();
            }
            let claimed = self
                .svc
                .poll_activities("std", WORKER_ID, 50)
                .await
                .expect("poll");
            if claimed.is_empty() {
                break;
            }
            for a in claimed {
                let round = *runs.get(&a.activity_key).unwrap_or(&0);
                *runs.entry(a.activity_key.clone()).or_default() += 1;
                let reply = respond(&a, round);
                ran.push(Ran {
                    key: a.activity_key.clone(),
                    round,
                    params: a.parameters.clone(),
                });
                match reply {
                    Reply::Done(outputs, cost) => self
                        .svc
                        .complete_activity(a.id, WORKER_ID.to_string(), outputs, cost, None)
                        .await
                        .expect("complete"),
                    Reply::Fail(msg, retryable) => {
                        self.svc
                            .fail_activity(
                                a.id,
                                WORKER_ID.to_string(),
                                "EXECUTION_ERROR".to_string(),
                                msg,
                                retryable,
                                None,
                                None,
                            )
                            .await
                            .expect("fail");
                    }
                }
                self.process_all().await;
            }
        }
        self.process_all().await;
        let s = self.state().await;
        let status = s.status.to_string();
        let hung = !matches!(status.as_str(), "completed" | "failed" | "cancelled");
        let report = Report {
            example: self.example.clone(),
            workflow_status: status,
            error_message: s.error_message.clone(),
            activities: s
                .activities
                .values()
                .map(|a| {
                    (
                        a.key.clone(),
                        (
                            format!("{:?}", a.status).to_lowercase(),
                            a.error.clone(),
                            a.iteration,
                        ),
                    )
                })
                .collect(),
            runs,
            ran,
            delays,
            hung,
        };
        println!("{}", report.render());
        report
    }
}

// ---------------------------------------------------------------------------
// Fake activity bodies, shaped like the std activities' outputs
// ---------------------------------------------------------------------------

fn http_ok(body: Value) -> Reply {
    Reply::Done(
        json!({ "response": { "status": 200, "success": true, "body": body } }),
        None,
    )
}

fn query_args(url: &str) -> serde_json::Map<String, Value> {
    url.split_once('?')
        .map(|(_, q)| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), json!(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// A plausible body for each URL the examples call
fn http_body(params: &Value) -> Value {
    let url = params["url"].as_str().unwrap_or_default();
    if url.contains("mailpit") {
        json!({ "ID": "fake-message-id" })
    } else if url.contains("api.weather.gov/points/") {
        json!({ "properties": {
            "forecast": "https://api.weather.gov/gridpoints/LWX/97,71/forecast" } })
    } else if url.contains("api.weather.gov/gridpoints/") {
        json!({ "properties": { "periods": [{
            "temperature": 41,
            "shortForecast": "Partly Sunny",
            "detailedForecast": "Partly sunny, with a high near 41." }] } })
    } else if url.contains("geocoding.geo.census.gov") {
        json!({ "result": { "addressMatches": [{
            "coordinates": { "x": -77.0365, "y": 38.8977 } }] } })
    } else if url.contains("httpbin.org") {
        json!({ "args": query_args(url), "url": url })
    } else {
        json!({})
    }
}

fn llm(content: &str) -> Reply {
    let cost = Decimal::from_str("0.0021").unwrap();
    Reply::Done(
        json!({ "result": {
            "content": content,
            "model": "claude-haiku-4-5-20251001",
            "provider": "anthropic",
            "finish_reason": "Stop",
            "cost_usd": cost,
            "budget_skipped_models": [],
            "usage": { "prompt_tokens": 120, "output_tokens": 80,
                       "total_tokens": 200, "cached_tokens": null } } }),
        Some(cost),
    )
}

fn embedding(params: &Value) -> Reply {
    let n = params["input"].as_array().map(Vec::len).unwrap_or(1);
    let vectors: Vec<Value> = (0..n).map(|i| json!([0.1 * i as f64, 0.2, 0.3])).collect();
    Reply::Done(
        json!({ "result": {
            "embeddings": vectors,
            "embeddings_file": null,
            "embedding_count": n,
            "model": "gemini-embedding-001",
            "provider": "google",
            "usage": { "prompt_tokens": 30, "output_tokens": 0,
                       "total_tokens": 30, "cached_tokens": null } } }),
        Some(Decimal::from_str("0.0001").unwrap()),
    )
}

fn pg_rows(rows: Value) -> Reply {
    Reply::Done(json!({ "result": { "rows": rows } }), None)
}

fn pg_affected(n: u64) -> Reply {
    Reply::Done(json!({ "result": { "rows_affected": n } }), None)
}

/// The generic fake for an activity; examples override what they branch on
fn fake(a: &PendingActivityRecord) -> Reply {
    let p = &a.parameters;
    match a.activity_name.as_str() {
        "http_request" => http_ok(http_body(p)),
        "echo" => Reply::Done(json!({ "echo": p }), None),
        "llm_prompt" => llm(&format!("Fake reply from {}.", a.activity_key)),
        "embedding" => embedding(p),
        "postgres_query" => {
            let q = p["query"].as_str().unwrap_or_default().to_uppercase();
            if q.trim_start().starts_with("SELECT") || q.contains("RETURNING") {
                pg_rows(json!([{ "id": 1 }]))
            } else {
                pg_affected(1)
            }
        }
        "postgres_transaction" => {
            let n = p["statements"].as_array().map(Vec::len).unwrap_or(0);
            Reply::Done(
                json!({ "result": {
                    "success": true,
                    "statements_executed": n,
                    "results": (0..n).map(|_| json!({ "rows_affected": 1 })).collect::<Vec<_>>() } }),
                None,
            )
        }
        other => Reply::Fail(format!("sweep has no fake for {other}"), false),
    }
}

fn completed(r: &Report) {
    assert!(
        !r.hung && r.workflow_status == "completed",
        "{} did not complete{}",
        r.example,
        r.render()
    );
    let not_done: Vec<_> = r
        .activities
        .iter()
        .filter(|(_, (s, _, _))| s != "completed" && s != "skipped")
        .map(|(k, (s, e, _))| format!("{k}={s} {e:?}"))
        .collect();
    assert!(not_done.is_empty(), "{not_done:?}{}", r.render());
}

fn assert_text(r: &Report, key: &str, n: usize, needle: &str) {
    let p = r.params(key, n).to_string();
    assert!(
        p.contains(needle),
        "{key} run {n} params lack {needle:?}: {p}{}",
        r.render()
    );
}

macro_rules! example {
    ($file:literal) => {
        ($file, include_str!(concat!("../../examples/", $file)))
    };
}

/// Simulates #32 by replacing `{{ACTIVITY.*}}` with literal text, so the
/// rest of an example can be checked while #32 is open
fn without_activity_context(yaml: &str) -> String {
    yaml.replace("{{ACTIVITY.iteration}}", "N")
        .replace("{{ACTIVITY.accumulated_cost_usd}}", "0.00")
        .replace("{{ACTIVITY.remaining_budget_usd}}", "0.00")
}

/// Apply text replacements to an example, each of which must match, to run
/// past a bug already found and look for the next one
fn patched(yaml: &str, patches: &[(&str, &str)]) -> String {
    patches.iter().fold(yaml.to_string(), |y, (from, to)| {
        assert!(y.contains(from), "patch target not found: {from}");
        y.replace(from, to)
    })
}

// ---------------------------------------------------------------------------
// 01, 01b: weather reports
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_01_weather_report(pool: PgPool) {
    let (name, yaml) = example!("01-weather-report.yaml");
    let r = Sweep::submit(pool, name, yaml, json!({}))
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    assert_text(&r, "send_notification", 0, "Temperature: 41");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_01b_weather_report_dynamic(pool: PgPool) {
    let (name, yaml) = example!("01b-weather-report-dynamic.yaml");
    let input = json!({
        "address": "1600 Pennsylvania Avenue NW, Washington, DC 20500",
        "city": "Washington", "state": "DC" });
    let r = Sweep::submit(pool, name, yaml, input)
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    assert_text(&r, "get_grid", 0, "points/38.8977,-77.0365");
    assert_text(&r, "fetch_forecast", 0, "gridpoints/LWX/97,71/forecast");
    assert_text(&r, "send_notification", 0, "Conditions: Partly Sunny");
}

// ---------------------------------------------------------------------------
// 02: conditional branches on the echoed `valid`
// ---------------------------------------------------------------------------

async fn run_02(pool: PgPool, valid: bool) -> Report {
    let (name, yaml) = example!("02-user-validation.yaml");
    let email = if valid {
        "user@example.com"
    } else {
        "bad@example.com"
    };
    Sweep::submit(pool, name, yaml, json!({ "email": email, "valid": valid }))
        .await
        .drive(|a, _| fake(a))
        .await
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_02_user_validation_valid(pool: PgPool) {
    let r = run_02(pool, true).await;
    completed(&r);
    assert_eq!(r.runs("store_valid_user"), 1, "{}", r.render());
    assert_eq!(r.runs("store_invalid_user"), 0, "{}", r.render());
    assert_eq!(r.runs("send_notification"), 1, "{}", r.render());
    assert_text(&r, "send_notification", 0, "Valid: true");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_02_user_validation_invalid(pool: PgPool) {
    let r = run_02(pool, false).await;
    completed(&r);
    assert_eq!(r.runs("store_valid_user"), 0, "{}", r.render());
    assert_eq!(r.runs("store_invalid_user"), 1, "{}", r.render());
    assert_eq!(r.runs("send_notification"), 1, "{}", r.render());
    assert_text(&r, "send_notification", 0, "Valid: false");
}

// ---------------------------------------------------------------------------
// 03: fan-out, fan-in
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_03_document_processing(pool: PgPool) {
    let (name, yaml) = example!("03-document-processing.yaml");
    let r = Sweep::submit(pool, name, yaml, json!({}))
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    assert_eq!(r.runs("aggregate_results"), 1, "{}", r.render());
    assert_text(&r, "store_summary", 0, "Doc 2: Architecture");
}

// ---------------------------------------------------------------------------
// 04: LLM moderation, stored
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_04_moderate_content(pool: PgPool) {
    let (name, yaml) = example!("04-moderate-content.yaml");
    let input = json!({ "user_content": "Check out this amazing product!",
                        "content_id": "content_12345" });
    let r = Sweep::submit(pool, name, yaml, input)
        .await
        .drive(|a, _| match a.activity_name.as_str() {
            "llm_prompt" => llm(r#"{"violates": false, "reason": "Promotional but harmless", "severity": "none", "confidence": 0.93}"#),
            _ => fake(a),
        })
        .await;
    completed(&r);
    assert_text(&r, "store_moderation_result", 0, "content_12345");
    assert_text(&r, "store_moderation_result", 0, "0.0021");
}

// ---------------------------------------------------------------------------
// 05, 05a, 05b, 05c: LLM with fallback, stored
// ---------------------------------------------------------------------------

async fn run_05(pool: PgPool, name: &str, yaml: &str) {
    let input = json!({ "question": "What are the key differences between Rust and Go?" });
    let r = Sweep::submit(pool, name, yaml, input)
        .await
        .drive(|a, _| match a.activity_name.as_str() {
            "llm_prompt" => llm("Rust has no GC; Go has a GC and goroutines."),
            _ => fake(a),
        })
        .await;
    completed(&r);
    assert_text(&r, "store_response", 0, "Rust has no GC");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_05_research_assistant(pool: PgPool) {
    let (name, yaml) = example!("05-research-assistant.yaml");
    run_05(pool, name, yaml).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_05a_research_assistant_anthropic(pool: PgPool) {
    let (name, yaml) = example!("05a-research-assistant-anthropic.yaml");
    run_05(pool, name, yaml).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_05b_research_assistant_openai(pool: PgPool) {
    let (name, yaml) = example!("05b-research-assistant-openai.yaml");
    run_05(pool, name, yaml).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_05c_research_assistant_google(pool: PgPool) {
    let (name, yaml) = example!("05c-research-assistant-google.yaml");
    run_05(pool, name, yaml).await;
}

// ---------------------------------------------------------------------------
// 06a, 06b, 06c: caching and RAG
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_06a_faq_bot_caching(pool: PgPool) {
    let (name, yaml) = example!("06a-faq-bot-caching.yaml");
    let r = Sweep::submit(
        pool,
        name,
        yaml,
        json!({ "question": "What are your business hours?" }),
    )
    .await
    .drive(|a, _| match a.activity_name.as_str() {
        "llm_prompt" => llm("We are open 9am-5pm, Monday to Friday."),
        _ => fake(a),
    })
    .await;
    completed(&r);
    assert_text(&r, "store_answer", 0, "9am-5pm");
    // faq_log.cache_hit is BOOLEAN NOT NULL
    let cache_hit = &r.params("store_answer", 0)["params"][3];
    assert!(
        cache_hit.is_boolean(),
        "cache_hit param is {cache_hit}{}",
        r.render()
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_06b_rag_index_builder(pool: PgPool) {
    let (name, yaml) = example!("06b-rag-index-builder.yaml");
    let input = json!({
        "chunks": ["Rust is a systems programming language...",
                   "Ownership model ensures memory safety...",
                   "Cargo is the package manager..."],
        "source": "rust_documentation" });
    let r = Sweep::submit(pool, name, yaml, input)
        .await
        .drive(|a, _| match a.activity_key.as_str() {
            "store_chunks" => pg_rows(json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }])),
            _ => fake(a),
        })
        .await;
    completed(&r);
    assert_text(&r, "confirm_indexing", 0, "Chunks Indexed: 3");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_06c_rag_query(pool: PgPool) {
    let (name, yaml) = example!("06c-rag-query.yaml");
    let r = Sweep::submit(
        pool,
        name,
        yaml,
        json!({ "question": "What is Rust's ownership model?" }),
    )
    .await
    .drive(|a, _| match a.activity_key.as_str() {
        "search_similar_chunks" => pg_rows(json!([
                { "content": "Ownership model ensures memory safety...",
                  "metadata": { "source": "rust_documentation", "chunk_index": 1 },
                  "distance": 0.12 }])),
        "generate_answer" => llm("Each value has one owner (rust_documentation)."),
        _ => fake(a),
    })
    .await;
    completed(&r);
    assert_text(
        &r,
        "generate_answer",
        0,
        "Ownership model ensures memory safety",
    );
    assert_text(&r, "send_response", 0, "Each value has one owner");
}

// ---------------------------------------------------------------------------
// 07a, 07b: research loops; both exits
// ---------------------------------------------------------------------------

/// `sufficient_round`: the evaluation round (0-based) that says SUFFICIENT;
/// None to never say it
async fn run_07a(pool: PgPool, yaml: &str, sufficient_round: Option<u32>) -> Report {
    Sweep::submit(
        pool,
        "07a-agentic-research-simple.yaml",
        yaml,
        json!({ "topic": "Benefits of Rust for systems programming" }),
    )
    .await
    .drive(move |a, round| match a.activity_key.as_str() {
        "perform_search" => llm(&format!(
            r#"{{"findings": "finding {round}", "sources_consulted": ["s{round}"], "confidence": 0.7}}"#
        )),
        "evaluate" if Some(round) == sufficient_round => llm("SUFFICIENT"),
        "evaluate" => llm("CONTINUE"),
        _ => fake(a),
    })
    .await
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_as_written(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let r = run_07a(pool, yaml, Some(2)).await;
    completed(&r);
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_sufficient_after_3_rounds(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let r = run_07a(pool, &without_activity_context(yaml), Some(2)).await;
    completed(&r);
    assert_eq!(r.runs("perform_search"), 3, "{}", r.render());
    assert_eq!(r.runs("evaluate"), 3, "{}", r.render());
    assert_eq!(r.runs("compile_report"), 1, "{}", r.render());
    assert_text(&r, "compile_report", 0, "finding 2");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_never_sufficient(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let r = run_07a(pool, &without_activity_context(yaml), None).await;
    assert!(!r.hung, "{}", r.render());
    assert_eq!(r.runs("perform_search"), 5, "{}", r.render());
    assert_eq!(r.runs("compile_report"), 0, "{}", r.render());
}

async fn run_07b(pool: PgPool, yaml: &str, sufficient_round: Option<u32>) -> Report {
    Sweep::submit(
        pool,
        "07b-agentic-research-complete.yaml",
        yaml,
        json!({ "research_topic": "Impact of quantum computing on cryptography" }),
    )
    .await
    .drive(move |a, round| match a.activity_key.as_str() {
        "search_information" => llm(&format!(
            r#"{{"findings": "finding {round}", "sources": ["s{round}"], "coverage_areas": [], "gaps_remaining": []}}"#
        )),
        "evaluate_sufficiency" if Some(round) == sufficient_round => {
            llm("SUFFICIENT\nAll questions are covered. Gaps: none.")
        }
        "evaluate_sufficiency" => llm("CONTINUE\nPost-quantum migration timelines are missing."),
        "compile_report" => llm("# Report\nQuantum computing threatens RSA."),
        _ => fake(a),
    })
    .await
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_07b_as_written(pool: PgPool) {
    let (_, yaml) = example!("07b-agentic-research-complete.yaml");
    let r = run_07b(pool, yaml, Some(2)).await;
    completed(&r);
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_07b_sufficient_after_3_rounds(pool: PgPool) {
    let (_, yaml) = example!("07b-agentic-research-complete.yaml");
    let r = run_07b(pool, &without_activity_context(yaml), Some(2)).await;
    completed(&r);
    assert_eq!(r.runs("search_information"), 3, "{}", r.render());
    assert_eq!(r.runs("evaluate_sufficiency"), 3, "{}", r.render());
    assert_eq!(r.runs("compile_report"), 1, "{}", r.render());
    assert_eq!(r.runs("publish_success"), 1, "{}", r.render());
    assert_eq!(r.runs("publish_failure"), 0, "{}", r.render());
    assert_text(&r, "search_information", 0, "Current iteration: 0");
    assert_text(&r, "search_information", 2, "finding 1");
    assert_text(&r, "compile_report", 0, "finding 2");
    assert_text(&r, "publish_success", 0, "Iterations: 3");
    assert_text(&r, "publish_success", 0, "Quantum computing threatens RSA");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_07b_never_sufficient(pool: PgPool) {
    let (_, yaml) = example!("07b-agentic-research-complete.yaml");
    let r = run_07b(pool, &without_activity_context(yaml), None).await;
    completed(&r);
    assert_eq!(r.runs("search_information"), 5, "{}", r.render());
    assert_eq!(r.runs("evaluate_sufficiency"), 5, "{}", r.render());
    assert_eq!(r.runs("compile_report"), 0, "{}", r.render());
    assert_eq!(r.runs("publish_success"), 0, "{}", r.render());
    assert_eq!(r.runs("publish_failure"), 1, "{}", r.render());
    assert_text(&r, "publish_failure", 0, "after 5 iterations");
    assert_text(&r, "publish_failure", 0, "Post-quantum migration timelines");
}

// ---------------------------------------------------------------------------
// 08a, 08b, 08c: loops with delay, scheduling, cascading delays
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_08a_rate_limited_api_calls(pool: PgPool) {
    let (name, yaml) = example!("08a-rate-limited-api-calls.yaml");
    let r = Sweep::submit(pool, name, yaml, json!({ "num_pages": 3 }))
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    assert_eq!(r.runs("fetch_page"), 3, "{}", r.render());
    assert_eq!(r.runs("aggregate_results"), 1, "{}", r.render());
    assert_text(&r, "fetch_page", 2, "page=2");
    assert_text(&r, "aggregate_results", 0, "page");
}

/// 08a with `{{fetch_page.result}}` read as `{{fetch_page.response}}`, the
/// output http_request produces; checks the 5s delay between rounds
#[sqlx::test(migrations = "../migrations")]
async fn sweep_08a_patched_output_name(pool: PgPool) {
    let (name, yaml) = example!("08a-rate-limited-api-calls.yaml");
    let yaml = patched(
        yaml,
        &[("{{fetch_page.result}}", "{{fetch_page.response}}")],
    );
    let r = Sweep::submit(pool, name, &yaml, json!({ "num_pages": 3 }))
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    assert_eq!(r.runs("fetch_page"), 3, "{}", r.render());
    assert_text(&r, "aggregate_results", 0, "page=2");
    // "No delay on first iteration", "5s delay after loop-back condition met"
    let delayed: Vec<Option<i32>> = r
        .delays
        .iter()
        .filter(|(k, _, s)| k == "fetch_page" && *s > 3.0)
        .map(|(_, it, _)| *it)
        .collect();
    assert_eq!(delayed, vec![Some(1), Some(2)], "{}", r.render());
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_08b_scheduled_daily_report(pool: PgPool) {
    let (name, yaml) = example!("08b-scheduled-daily-report.yaml");
    // A time in the future, so the schedule is visible as a delay
    let report_time = (chrono::Utc::now() + chrono::Duration::hours(2))
        .format("%Y-%m-%dT%H:%M:%S+00:00")
        .to_string();
    let r = Sweep::submit(pool, name, yaml, json!({ "report_time": report_time }))
        .await
        .drive(|a, _| match a.activity_name.as_str() {
            "llm_prompt" => llm("# Daily report\nAll metrics nominal."),
            _ => fake(a),
        })
        .await;
    completed(&r);
    assert!(
        r.delays
            .iter()
            .any(|(k, _, s)| k == "generate_report" && *s > 3600.0),
        "generate_report was not scheduled ~2h out{}",
        r.render()
    );
    assert_text(&r, "generate_report", 0, &report_time[..10]);
    assert_text(&r, "send_report", 0, "All metrics nominal");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_08c_delayed_reminders(pool: PgPool) {
    let (name, yaml) = example!("08c-delayed-reminders.yaml");
    let input = json!({
        "task_name": "Complete quarterly report",
        "assigned_user": "alice@example.com",
        "manager_email": "manager@example.com",
        "oncall_email": "oncall@example.com" });
    let r = Sweep::submit(pool, name, yaml, input)
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    for (key, secs) in [
        ("send_first_reminder", 60.0),
        ("send_escalated_reminder", 120.0),
        ("send_final_escalation", 300.0),
    ] {
        assert!(
            r.delays
                .iter()
                .any(|(k, _, s)| k == key && (*s - secs).abs() < 10.0),
            "{key} not delayed {secs}s{}",
            r.render()
        );
    }
    assert_text(&r, "send_final_escalation", 0, "oncall@example.com");
}

// ---------------------------------------------------------------------------
// 09a, 09b: streaming
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_09a_streaming_llm(pool: PgPool) {
    let (name, yaml) = example!("09a-streaming-llm.yaml");
    let r = Sweep::submit(
        pool,
        name,
        yaml,
        json!({ "topic": "a robot learning to paint" }),
    )
    .await
    .drive(|a, _| fake(a))
    .await;
    completed(&r);
    assert_text(&r, "generate_story", 0, "a robot learning to paint");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_09b_streaming_research(pool: PgPool) {
    let (name, yaml) = example!("09b-streaming-research.yaml");
    let r = Sweep::submit(
        pool,
        name,
        yaml,
        json!({ "topic": "Impact of quantum computing on cryptography" }),
    )
    .await
    .drive(|a, _| match a.activity_key.as_str() {
        "summarize_topic" => llm("Quantum computers may break RSA."),
        "gather_sources" => llm("1. When? 2. Which algorithms? 3. What migrates first?"),
        _ => fake(a),
    })
    .await;
    completed(&r);
    assert_text(
        &r,
        "analyze_findings",
        0,
        "Quantum computers may break RSA.",
    );
    assert_text(&r, "analyze_findings", 0, "Which algorithms?");
}

// ---------------------------------------------------------------------------
// 10: order processing; enough and not enough stock
// ---------------------------------------------------------------------------

async fn run_10(pool: PgPool, unreserved: i64) -> Report {
    let (name, yaml) = example!("10-order-processing.yaml");
    let input = json!({
        "customer_id": "cust_123", "customer_email": "customer@example.com",
        "product_id": "prod_001", "quantity": 2, "amount": 99.99 });
    Sweep::submit(pool, name, yaml, input)
        .await
        .drive(move |a, _| match a.activity_key.as_str() {
            "validate_inventory" => pg_rows(json!([
                { "available": unreserved, "reserved": 0, "unreserved": unreserved }])),
            "reserve_inventory" => pg_rows(json!([{ "product_id": "prod_001", "reserved": 2 }])),
            "process_payment" => pg_rows(json!([{ "transaction_id": 7001, "status": "pending" }])),
            "record_order" => Reply::Done(
                json!({ "result": {
                    "success": true, "statements_executed": 2,
                    "results": [{ "rows": [{ "order_id": 42 }] }, { "rows_affected": 1 }] } }),
                None,
            ),
            _ => fake(a),
        })
        .await
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_10_order_processing_in_stock(pool: PgPool) {
    let r = run_10(pool, 10).await;
    completed(&r);
    assert_eq!(r.runs("send_confirmation"), 1, "{}", r.render());
    assert_text(&r, "send_confirmation", 0, "Order Confirmation - #42");
    assert_text(&r, "send_confirmation", 0, "Payment ID: 7001");
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_10_order_processing_out_of_stock(pool: PgPool) {
    let r = run_10(pool, 1).await;
    assert!(!r.hung, "{}", r.render());
    assert_eq!(r.runs("reserve_inventory"), 0, "{}", r.render());
    assert_eq!(r.runs("process_payment"), 0, "{}", r.render());
    assert_eq!(r.runs("send_confirmation"), 0, "{}", r.render());
}

// ---------------------------------------------------------------------------
// Patched runs: past the first bug in an example, to find the next
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../migrations")]
async fn sweep_06b_patched_output_name(pool: PgPool) {
    let (name, yaml) = example!("06b-rag-index-builder.yaml");
    let yaml = patched(
        yaml,
        &[
            (
                "{{generate_embeddings.embeddings.embeddings}}",
                "{{generate_embeddings.result.embeddings}}",
            ),
            (
                "{{store_chunks.result.row_count}}",
                "{{store_chunks.result.rows | length}}",
            ),
            ("{{generate_embeddings.embeddings.cost_usd}}", "(unknown)"),
        ],
    );
    let input = json!({
        "chunks": ["Rust is a systems programming language...",
                   "Ownership model ensures memory safety...",
                   "Cargo is the package manager..."],
        "source": "rust_documentation" });
    let r = Sweep::submit(pool, name, &yaml, input)
        .await
        .drive(|a, _| match a.activity_key.as_str() {
            "store_chunks" => pg_rows(json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }])),
            _ => fake(a),
        })
        .await;
    completed(&r);
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_06c_patched_output_name(pool: PgPool) {
    let (name, yaml) = example!("06c-rag-query.yaml");
    let yaml = patched(
        yaml,
        &[
            (
                "{{embed_question.embeddings.embeddings[0]}}",
                "{{embed_question.result.embeddings[0]}}",
            ),
            ("{{embed_question.embeddings.cost_usd}}", "0.0001"),
            (
                "{{embed_question.embeddings.cost_usd + generate_answer.result.cost_usd}}",
                "{{generate_answer.result.cost_usd}}",
            ),
        ],
    );
    let r = Sweep::submit(
        pool,
        name,
        &yaml,
        json!({ "question": "What is Rust's ownership model?" }),
    )
    .await
    .drive(|a, _| match a.activity_key.as_str() {
        "search_similar_chunks" => pg_rows(json!([
                { "content": "Ownership model ensures memory safety...",
                  "metadata": { "source": "rust_documentation", "chunk_index": 1 },
                  "distance": 0.12 }])),
        "generate_answer" => llm("Each value has one owner (rust_documentation)."),
        _ => fake(a),
    })
    .await;
    completed(&r);
}

/// 07a with #32 simulated and `perform_search.findings` read as
/// `perform_search.result`, the output llm_prompt produces
#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_patched_output_name(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let yaml = patched(
        &without_activity_context(yaml),
        &[("perform_search.findings", "perform_search.result")],
    );
    let r = run_07a(pool, &yaml, Some(2)).await;
    completed(&r);
}

/// As above, and the first round's missing `perform_search.result` defaulted
/// to `[]`
#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_patched_first_round_default(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let yaml = patched(
        &without_activity_context(yaml),
        &[(
            "perform_search.findings",
            "(perform_search.result | default([]))",
        )],
    );
    let r = run_07a(pool, &yaml, Some(2)).await;
    completed(&r);
}

/// As above, and `| json` read as `| tojson`, the filter MiniJinja has
#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_patched_tojson(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let yaml = patched(
        &without_activity_context(yaml),
        &[
            (
                "perform_search.findings",
                "(perform_search.result | default([]))",
            ),
            ("| json}}", "| tojson}}"),
        ],
    );
    let r = run_07a(pool, &yaml, Some(2)).await;
    completed(&r);
}

/// As above, and `| contains(substring='X')` read as `'X' in ...`
#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_patched_in_operator(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let yaml = patched(
        &without_activity_context(yaml),
        &[
            (
                "perform_search.findings",
                "(perform_search.result | default([]))",
            ),
            ("| json}}", "| tojson}}"),
            (
                "{{evaluate.result.content | contains(substring='CONTINUE')}}",
                "{{ 'CONTINUE' in evaluate.result.content }}",
            ),
            (
                "{{evaluate.result.content | contains(substring='SUFFICIENT')}}",
                "{{ 'SUFFICIENT' in evaluate.result.content }}",
            ),
        ],
    );
    let r = run_07a(pool, &yaml, Some(2)).await;
    completed(&r);
}

/// As above, and `evaluate.result.content` read from the latest round:
/// `evaluate` is iteration-scoped, so `evaluate.result` is an array
#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_patched_latest_round(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let patches = [
        (
            "perform_search.findings",
            "(perform_search.result | default([]))",
        ),
        ("| json}}", "| tojson}}"),
        (
            "{{evaluate.result.content | contains(substring='CONTINUE')}}",
            "{{ 'CONTINUE' in (evaluate.result | last).content }}",
        ),
        (
            "{{evaluate.result.content | contains(substring='SUFFICIENT')}}",
            "{{ 'SUFFICIENT' in (evaluate.result | last).content }}",
        ),
    ];
    let yaml = patched(&without_activity_context(yaml), &patches);
    let r = run_07a(pool.clone(), &yaml, Some(2)).await;
    completed(&r);
    assert_eq!(r.runs("perform_search"), 3, "{}", r.render());
    assert_eq!(r.runs("evaluate"), 3, "{}", r.render());
    assert_eq!(r.runs("compile_report"), 1, "{}", r.render());
}

/// The patched 07a when evaluate never says SUFFICIENT
#[sqlx::test(migrations = "../migrations")]
async fn sweep_07a_patched_never_sufficient(pool: PgPool) {
    let (_, yaml) = example!("07a-agentic-research-simple.yaml");
    let patches = [
        (
            "perform_search.findings",
            "(perform_search.result | default([]))",
        ),
        ("| json}}", "| tojson}}"),
        (
            "{{evaluate.result.content | contains(substring='CONTINUE')}}",
            "{{ 'CONTINUE' in (evaluate.result | last).content }}",
        ),
        (
            "{{evaluate.result.content | contains(substring='SUFFICIENT')}}",
            "{{ 'SUFFICIENT' in (evaluate.result | last).content }}",
        ),
    ];
    let yaml = patched(&without_activity_context(yaml), &patches);
    let r = run_07a(pool, &yaml, None).await;
    assert!(!r.hung, "{}", r.render());
    assert_eq!(r.runs("perform_search"), 5, "{}", r.render());
    assert_eq!(r.runs("compile_report"), 0, "{}", r.render());
}

#[sqlx::test(migrations = "../migrations")]
async fn sweep_10_patched_no_started_at(pool: PgPool) {
    let (_, yaml) = example!("10-order-processing.yaml");
    let yaml = patched(yaml, &[("{{WORKFLOW.started_at}}", "(unknown)")]);
    let input = json!({
        "customer_id": "cust_123", "customer_email": "customer@example.com",
        "product_id": "prod_001", "quantity": 2, "amount": 99.99 });
    let r = Sweep::submit(pool, "10-order-processing.yaml", &yaml, input)
        .await
        .drive(|a, _| match a.activity_key.as_str() {
            "validate_inventory" => pg_rows(json!([
                { "available": 10, "reserved": 0, "unreserved": 10 }])),
            "reserve_inventory" => pg_rows(json!([{ "product_id": "prod_001", "reserved": 2 }])),
            "process_payment" => pg_rows(json!([{ "transaction_id": 7001, "status": "pending" }])),
            "record_order" => Reply::Done(
                json!({ "result": {
                    "success": true, "statements_executed": 2,
                    "results": [{ "rows": [{ "order_id": 42 }] }, { "rows_affected": 1 }] } }),
                None,
            ),
            _ => fake(a),
        })
        .await;
    completed(&r);
    assert_text(&r, "send_confirmation", 0, "Order Confirmation - #42");
    assert_text(&r, "send_confirmation", 0, "Payment ID: 7001");
}

// ---------------------------------------------------------------------------
// Engine probe behind 08a: does `settings.delay` apply to every loop round?
// ---------------------------------------------------------------------------

/// A self-loop with a delay and no other features, so it runs on `main` too
const LOOP_DELAY_PROBE: &str = r#"
name: loop_delay_probe
activities:
  - key: tick
    activity_name: echo
    iteration_limit: 3
    parameters:
      n: 1
    settings:
      delay: "5s"
    depends_on:
      - activity_key: tick
        conditions:
          - "{{ tick.status == 'completed' }}"
"#;

#[sqlx::test(migrations = "../migrations")]
async fn sweep_probe_loop_delay_every_round(pool: PgPool) {
    let r = Sweep::submit(pool, "loop_delay_probe", LOOP_DELAY_PROBE, json!({}))
        .await
        .drive(|a, _| fake(a))
        .await;
    completed(&r);
    assert_eq!(r.runs("tick"), 3, "{}", r.render());
    let delayed: Vec<Option<i32>> = r
        .delays
        .iter()
        .filter(|(k, _, s)| k == "tick" && *s > 3.0)
        .map(|(_, it, _)| *it)
        .collect();
    // "Wait 5 seconds after dependencies are met" (docs/architecture.md),
    // which happens once per round
    assert!(
        delayed.contains(&Some(1)) && delayed.contains(&Some(2)),
        "delayed rounds: {delayed:?}{}",
        r.render()
    );
}
