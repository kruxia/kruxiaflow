// ============================================================================
// End-to-end tests for Example 16: sandboxed code with run_code
//
// Same shape as api/tests/example_03_e2e_test.rs and example_10_e2e_test.rs:
// the real example YAML is deployed through the API and run by the engine's
// orchestrator and this add-on's worker, then its output is checked.
//
// Requires a database (DATABASE_URL, as for every #[sqlx::test]) and the
// `monty` worker binary (from contrib/kruxiaflow-monty):
//
//   KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty \
//     cargo nextest run --test example_16_run_code_e2e_test
//
// Without KRUXIAFLOW_MONTY_BINARY the end-to-end test prints a notice and
// returns. 16b needs an LLM, so only its definition is checked here.
// ============================================================================

use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::definition::WorkflowDefinition;
use kruxiaflow_core::{
    OrchestratorConfig, PostgresSubscriptionService, SubscriptionService, run_orchestrator,
};
use kruxiaflow_monty::{MontyConfig, RunCodeActivity, WORKER};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_worker::{Worker, WorkerConfig};
use serde_json::json;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const EXAMPLE_16A: &str = include_str!("../examples/16a-run-code-order-rules.yaml");
const EXAMPLE_16B: &str = include_str!("../examples/16b-run-code-repair-loop.yaml");

/// Wait until the API server responds to /health (poll every 10ms, panic after 10s)
async fn wait_for_api_ready(api_url: &str) {
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(resp) = client.get(format!("{}/health", api_url)).send().await
            && resp.status().is_success()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("API server did not become ready within 10s");
}

fn test_rsa_private_key() -> String {
    include_str!("../../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../../oauth/tests/public.pem").to_string()
}

/// Create OAuth client for testing
async fn create_test_oauth_client(pool: &PgPool, client_id: &str, client_secret: &str) {
    let secret_hash = bcrypt::hash(client_secret, bcrypt::DEFAULT_COST).unwrap();
    sqlx::query(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
    )
    .bind(client_id)
    .bind(&secret_hash)
    .bind("Test Client")
    .execute(pool)
    .await
    .expect("Failed to create test OAuth client");
}

/// Get OAuth token for testing
async fn get_test_token(api_url: &str, client_id: &str, client_secret: &str) -> String {
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/oauth/token", api_url))
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": client_id,
            "client_secret": client_secret
        }))
        .send()
        .await
        .expect("Failed to get token");
    let body: serde_json::Value = response.json().await.expect("Failed to parse token");
    body["access_token"]
        .as_str()
        .expect("No access_token")
        .to_string()
}

/// The run_code configuration for the worker, or None when no binary is set
fn monty_config() -> Option<MontyConfig> {
    let binary = std::env::var_os("KRUXIAFLOW_MONTY_BINARY").filter(|b| !b.is_empty())?;
    Some(MontyConfig {
        binary: PathBuf::from(binary),
        min_processes: 1,
        max_processes: 2,
        request_timeout: Duration::from_secs(30),
        recycle_after: 1,
        max_memory_bytes: 64 * 1024 * 1024,
        max_duration: Duration::from_secs(10),
        max_recursion_depth: 1000,
    })
}

#[sqlx::test(migrations = "../../migrations")]
async fn test_example_16a_run_code_order_rules(pool: PgPool) {
    let Some(monty) = monty_config() else {
        eprintln!("skipped: set KRUXIAFLOW_MONTY_BINARY to run the Example 16a end-to-end test");
        return;
    };

    // Auth and OAuth clients
    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };
    let auth_service =
        PostgresAuthService::new(pool.clone(), auth_config).expect("Failed to create auth service");
    create_test_oauth_client(&pool, "test_client", "test_secret").await;
    create_test_oauth_client(&pool, "test_worker", "test_worker_secret").await;

    // Shared services
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        ..QueueConfig::default()
    };
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let shutdown_token = CancellationToken::new();
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    // API server
    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        activity_queue.clone(),
        event_source.clone(),
        workflow_storage.clone(),
        Arc::new(kruxiaflow_core::cache::NoOpCache::new()),
        subscription_service.clone(),
        shutdown_token.clone(),
    );
    let app = app_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind API");
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("API server failed");
    });
    wait_for_api_ready(&api_url).await;
    let token = get_test_token(&api_url, "test_client", "test_secret").await;

    // Orchestrator
    let orchestrator_event_source = event_source.clone();
    let orchestrator_queue = activity_queue.clone();
    let orchestrator_pool = pool.clone();
    let orchestrator_shutdown = shutdown_token.clone();
    let orchestrator_subscription = subscription_service.clone();
    tokio::spawn(async move {
        let config = OrchestratorConfig::new(orchestrator_pool)
            .with_poll_interval(Duration::from_millis(1), Duration::from_millis(10));
        run_orchestrator(
            orchestrator_event_source,
            orchestrator_queue,
            orchestrator_subscription,
            config,
            Some(orchestrator_shutdown),
        )
        .await
        .expect("Orchestrator failed");
    });

    // This add-on's worker, built the way its main does it
    let activity = RunCodeActivity::new(monty, WORKER)
        .await
        .expect("valid Monty configuration and binary");
    let worker_config = WorkerConfig {
        api_url: api_url.clone(),
        worker_id: format!("test_worker_{}", Uuid::now_v7()),
        worker: WORKER.to_string(),
        poll_max_activities: 10,
        poll_interval: Duration::from_millis(10),
        max_concurrent_activities: 4,
        activity_timeout: Duration::from_secs(30),
        heartbeat_interval: Duration::from_secs(30),
        client_id: Some("test_worker".to_string()),
        client_secret: Some("test_worker_secret".to_string()),
        shutdown_timeout: Duration::from_secs(30),
        ..WorkerConfig::default()
    };
    let worker = Worker::builder()
        .config(worker_config)
        .register(activity)
        .build()
        .expect("worker configuration");
    let worker_handle = worker.handle();
    let worker_task = tokio::spawn(async move { worker.run().await });

    // Deploy the example exactly as shipped
    let client = reqwest::Client::new();
    let deploy_response = client
        .post(format!("{}/api/v1/workflow_definitions", api_url))
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "text/yaml")
        .body(EXAMPLE_16A)
        .send()
        .await
        .expect("Failed to deploy workflow");
    assert_eq!(
        deploy_response.status(),
        reqwest::StatusCode::CREATED,
        "Deploy failed: {:?}",
        deploy_response.text().await
    );
    let deployed: serde_json::Value = deploy_response.json().await.unwrap();

    // Submit with the input documented in the example header
    let submit_response = client
        .post(format!("{}/api/v1/workflows", api_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": deployed["name"],
            "version": deployed["version"],
            "input": {"orders": [
                {"id": "A1", "total": 612.50, "first_order": true, "weekday": "Sat"},
                {"id": "A2", "total": 80.00, "first_order": true, "weekday": "Sun"},
                {"id": "A3", "total": 900.00, "first_order": false, "weekday": "Sat"}
            ]}
        }))
        .send()
        .await
        .expect("Failed to submit workflow");
    assert_eq!(
        submit_response.status(),
        reqwest::StatusCode::CREATED,
        "Submit failed: {:?}",
        submit_response.text().await
    );
    let submitted: serde_json::Value = submit_response.json().await.unwrap();
    let workflow_id = submitted["workflow_id"].as_str().unwrap().to_string();

    // Poll for completion (timeout after 60 seconds)
    let start = std::time::Instant::now();
    let mut final_status = None;
    while start.elapsed() < Duration::from_secs(60) {
        let status: serde_json::Value = client
            .get(format!("{}/api/v1/workflows/{}", api_url, workflow_id))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .expect("Failed to get workflow status")
            .json()
            .await
            .expect("Failed to parse status");
        let s = status["status"].as_str().unwrap_or_default().to_string();
        if s == "completed" || s == "failed" {
            final_status = Some((s, status));
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (status, detail) = final_status.expect("Workflow did not finish within 60s");
    assert_eq!(status, "completed", "Workflow did not complete: {detail:#}");

    // The rule's output, through the activity output API
    let output: serde_json::Value = client
        .get(format!(
            "{}/api/v1/workflows/{}/activities/check_orders/output",
            api_url, workflow_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .expect("Failed to get activity output")
        .json()
        .await
        .expect("Failed to parse activity output");
    let result = &output["output"][0]["value"];
    assert_eq!(result["ok"], json!(true), "{result:#}");
    assert_eq!(result["value"]["flagged"], json!(["A1"]));
    assert_eq!(result["value"]["checked"], json!(3));
    assert!(result["value"]["checked_at"].is_string());
    assert_eq!(result["logs"], json!(["flagged A1: 612.5"]));
    // The run record carries the limits the example asked for
    assert_eq!(result["run"]["limits"]["max_memory"], json!(16000000));
    assert_eq!(
        result["run"]["limits"]["max_feed_duration_secs"],
        json!(2.0)
    );
    assert_eq!(
        result["run"]["capabilities"]["granted"],
        json!(["log", "now"])
    );

    println!("✅ Example 16a: run_code order rules end-to-end test passed!");

    worker_handle.shutdown();
    worker_task.await.expect("worker stopped");
    shutdown_token.cancel();
}

#[test]
fn test_example_16a_verify_workflow_definition() {
    let definition = WorkflowDefinition::from_yaml(EXAMPLE_16A).expect("Failed to parse 16a");
    assert_eq!(definition.name, "run_code_order_rules");
    assert_eq!(definition.activities.len(), 1);
    let step = &definition.activities[0];
    assert_eq!(step.key, "check_orders");
    assert_eq!(step.activity_name.as_deref(), Some("run_code"));
    assert_eq!(step.worker, WORKER, "run_code runs on the add-on's worker");
    let params = step.parameters.as_ref().expect("parameters");
    assert_eq!(params["capabilities"], json!(["log", "now"]));
    assert!(
        params["code"]
            .as_str()
            .unwrap()
            .contains("from decimal import Decimal")
    );
}

#[test]
fn test_example_16b_verify_workflow_definition() {
    // 16b needs an LLM, so it is not run here; its loop structure is checked
    let definition = WorkflowDefinition::from_yaml(EXAMPLE_16B).expect("Failed to parse 16b");
    assert_eq!(definition.name, "run_code_repair_loop");
    let keys: Vec<&str> = definition
        .activities
        .iter()
        .map(|a| a.key.as_str())
        .collect();
    assert_eq!(keys, vec!["describe", "write_rule", "run_rule", "done"]);

    let write_rule = &definition.activities[1];
    assert!(write_rule.is_loop_activity, "write_rule is the loop head");
    assert_eq!(write_rule.iteration_limit, Some(3));
    let back_edge = write_rule
        .depends_on
        .iter()
        .flatten()
        .find(|d| d.activity_key == "run_rule")
        .expect("write_rule depends on run_rule");
    assert!(back_edge.is_back_edge);
    assert_eq!(
        back_edge.conditions.as_deref(),
        Some(&["{{ run_rule.result.ok == false }}".to_string()][..])
    );

    let run_rule = &definition.activities[2];
    assert_eq!(run_rule.activity_name.as_deref(), Some("run_code"));
    assert_eq!(
        run_rule.worker, WORKER,
        "run_code runs on the add-on's worker"
    );
    assert!(run_rule.is_loop_activity);
}
