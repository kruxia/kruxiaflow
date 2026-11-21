//! End-to-end tests for `wait_for_signal` through the real stack: API server,
//! orchestrator (`run_orchestrator`) and a std worker, with signals sent to
//! `POST /api/v1/workflows/{id}/signal` exactly as a client sends them.
//!
//! - A wait outside any loop takes one signal, then runs its activity.
//! - A wait inside a loop takes one signal per round.
//!
//! Activities call the API server's own health endpoint (no external services).
use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::{
    OrchestratorConfig, PostgresSubscriptionService, SubscriptionService, run_orchestrator,
};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_std_worker::{ActivityRegistry, HttpRequestActivity, WorkerConfig, WorkerManager};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(60);

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

/// The running stack: API server, orchestrator and one std worker
struct Stack {
    pool: PgPool,
    api_url: String,
    token: String,
    client: reqwest::Client,
    shutdown: CancellationToken,
    manager: WorkerManager,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl Stack {
    async fn start(pool: PgPool) -> Stack {
        let auth_service = PostgresAuthService::new(
            pool.clone(),
            AuthConfig {
                rsa_private_key_pem: include_str!("../../oauth/tests/private.pem").to_string(),
                rsa_public_key_pem: Some(include_str!("../../oauth/tests/public.pem").to_string()),
                jwt_issuer: "test".to_string(),
                jwt_audience: "test".to_string(),
                token_ttl: 3600,
            },
        )
        .expect("Failed to create auth service");
        create_test_oauth_client(&pool, "test_client", "test_secret").await;
        create_test_oauth_client(&pool, "test_worker", "test_worker_secret").await;

        let activity_queue: Arc<dyn ActivityQueue> = Arc::new(PostgresQueue::new(
            pool.clone(),
            QueueConfig {
                poll_interval: Duration::from_millis(10),
                ..QueueConfig::default()
            },
        ));
        let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
        let storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
        let subscriptions: Arc<dyn SubscriptionService> =
            Arc::new(PostgresSubscriptionService::new(pool.clone()));
        let shutdown = CancellationToken::new();

        let state = AppState::new(
            pool.clone(),
            Arc::new(auth_service),
            activity_queue.clone(),
            event_source.clone(),
            storage.clone(),
            Arc::new(kruxiaflow_core::cache::NoOpCache::new()),
            subscriptions.clone(),
            shutdown.clone(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let app = app_router(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("API server failed");
        });

        let client = reqwest::Client::new();
        let start = Instant::now();
        loop {
            if let Ok(r) = client.get(format!("{api_url}/health")).send().await
                && r.status().is_success()
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "API not ready");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let token: Value = client
            .post(format!("{api_url}/api/v1/oauth/token"))
            .json(&json!({
                "grant_type": "client_credentials",
                "client_id": "test_client",
                "client_secret": "test_secret"
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let token = token["access_token"].as_str().unwrap().to_string();

        let orch_shutdown = shutdown.clone();
        let orch_pool = pool.clone();
        tokio::spawn(async move {
            run_orchestrator(
                event_source,
                activity_queue,
                subscriptions,
                OrchestratorConfig::new(orch_pool)
                    .with_poll_interval(Duration::from_millis(1), Duration::from_millis(10))
                    .with_timeout_check_interval(Duration::from_millis(100)),
                Some(orch_shutdown),
            )
            .await
            .expect("Orchestrator failed");
        });

        let mut registry = ActivityRegistry::new(Arc::new(kruxiaflow_core::NoOpCache::new()));
        registry.register(Arc::new(HttpRequestActivity::new()));
        let manager = WorkerManager::new(
            WorkerConfig {
                api_url: api_url.clone(),
                worker_id: format!("test_worker_{}", Uuid::now_v7()),
                worker: "std".to_string(),
                poll_max_activities: 10,
                poll_interval: Duration::from_millis(10),
                max_concurrent_activities: 10,
                activity_timeout: Duration::from_secs(30),
                heartbeat_interval: Duration::from_secs(30),
                client_id: Some("test_worker".to_string()),
                client_secret: Some("test_worker_secret".to_string()),
                shutdown_timeout: Duration::from_secs(30),
                ..WorkerConfig::default()
            },
            registry,
            storage,
        );
        let handles = manager.start().await.expect("Failed to start worker");

        Stack {
            pool,
            api_url,
            token,
            client,
            shutdown,
            manager,
            handles,
        }
    }

    async fn stop(self) {
        self.manager.stop(self.handles).await;
        self.shutdown.cancel();
    }

    /// Deploy a definition and submit one workflow of it
    async fn submit(&self, yaml: &str) -> Uuid {
        let deploy = self
            .client
            .post(format!("{}/api/v1/workflow_definitions", self.api_url))
            .bearer_auth(&self.token)
            .header("Content-Type", "text/yaml")
            .body(yaml.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(deploy.status(), reqwest::StatusCode::CREATED);
        let deployed: Value = deploy.json().await.unwrap();
        let submit = self
            .client
            .post(format!("{}/api/v1/workflows", self.api_url))
            .bearer_auth(&self.token)
            .json(&json!({
                "definition_name": deployed["name"],
                "version": deployed["version"],
                "input": {}
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(submit.status(), reqwest::StatusCode::CREATED);
        let submitted: Value = submit.json().await.unwrap();
        submitted["workflow_id"].as_str().unwrap().parse().unwrap()
    }

    /// Workflow status and its activities' materialized state
    async fn workflow(&self, id: Uuid) -> (String, Value) {
        sqlx::query_as::<_, (String, Value)>(
            "SELECT status::text, activities FROM workflows WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn signal(&self, id: Uuid, key: &str, event_name: &str, data: Value) -> Value {
        let r = self
            .client
            .post(format!("{}/api/v1/workflows/{id}/signal", self.api_url))
            .bearer_auth(&self.token)
            .json(&json!({"activity_key": key, "event_name": event_name, "data": data}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::OK);
        r.json().await.unwrap()
    }

    /// Run the workflow to a terminal status, signalling `key` each time it
    /// waits. Returns the final status, activities and the signals sent.
    async fn drive(&self, id: Uuid, key: &str) -> (String, Value, u32) {
        let start = Instant::now();
        let mut signals = 0;
        while start.elapsed() < TIMEOUT {
            let (status, activities) = self.workflow(id).await;
            if status == "completed" || status == "failed" {
                return (status, activities, signals);
            }
            if activities[key]["status"] == "waiting" {
                let round = activities[key]["iteration"].clone();
                signals += 1;
                let r = self.signal(id, key, "go", json!({"n": signals})).await;
                assert_eq!(r["signaled"], true, "signal {signals}: {r}");
                // Wait for the orchestrator to take the signal before looking
                // for the next wait (which may already have begun)
                while start.elapsed() < TIMEOUT {
                    let (status, activities) = self.workflow(id).await;
                    if activities[key]["status"] != "waiting"
                        || activities[key]["iteration"] != round
                        || status == "completed"
                        || status == "failed"
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                continue;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let (status, activities) = self.workflow(id).await;
        let events: Vec<(String, Option<String>, Option<i32>, Value)> = sqlx::query_as(
            "SELECT event_type::text, activity_key, iteration, payload FROM workflow_events \
             WHERE workflow_id = $1 ORDER BY position",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let subs: Vec<(String, Option<Value>)> = sqlx::query_as(
            "SELECT activity_key, signal_data FROM activity_event_subscriptions WHERE workflow_id = $1",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        panic!(
            "workflow {id} still {status} after {TIMEOUT:?} ({signals} signals): {activities:#}\nevents: {events:#?}\nsubscriptions: {subs:?}"
        );
    }
}

fn http_get(api_url: &str) -> String {
    format!(
        r#"    worker: std
    activity_name: http_request
    parameters:
      method: GET
      url: "{api_url}/health""#
    )
}

/// A wait outside any loop: one signal, then the activity runs
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_single_wait(pool: PgPool) {
    let stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let id = stack
        .submit(&format!(
            r#"
name: signal_once
activities:
  - key: start
{get}
  - key: approve
{get}
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 600
    depends_on: [start]
  - key: done
{get}
    depends_on: [approve]
"#
        ))
        .await;

    let (status, activities, signals) = stack.drive(id, "approve").await;
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(signals, 1);
    for key in ["start", "approve", "done"] {
        assert_eq!(
            activities[key]["status"], "completed",
            "{key}: {activities:#}"
        );
    }
    stack.stop().await;
}

/// A two-step loop (`head` -> `tail`, back to `head`, three rounds) whose
/// `wait` step takes one signal per round
async fn signal_in_loop(pool: PgPool, wait: &str) {
    let stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let settings = |key: &str| {
        if key == wait {
            "    settings:\n      wait_for_signal:\n        event_name: go\n        timeout_seconds: 600\n"
        } else {
            ""
        }
    };
    let id = stack
        .submit(&format!(
            r#"
name: signal_loop_{wait}
activities:
  - key: start
{get}
  - key: head
{get}
{head_settings}    iteration_limit: 3
    depends_on:
      - start
      - activity_key: tail
        conditions: ["{{{{ tail.status == 'completed' }}}}"]
  - key: tail
{get}
{tail_settings}    depends_on: [head]
"#,
            head_settings = settings("head"),
            tail_settings = settings("tail"),
        ))
        .await;

    let (status, activities, signals) = stack.drive(id, wait).await;
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(signals, 3, "{activities:#}");
    for key in ["head", "tail"] {
        assert_eq!(
            activities[key]["status"], "completed",
            "{key}: {activities:#}"
        );
        assert_eq!(activities[key]["iteration"], 3, "{key}: {activities:#}");
    }
    stack.stop().await;
}

/// The loop head waits for a signal each round
#[sqlx::test(migrations = "../migrations")]
#[ignore = "needs #18"]
async fn test_signal_in_loop_head(pool: PgPool) {
    signal_in_loop(pool, "head").await;
}

/// The step that closes the loop waits for a signal each round
#[sqlx::test(migrations = "../migrations")]
#[ignore = "needs #18"]
async fn test_signal_in_loop_tail(pool: PgPool) {
    signal_in_loop(pool, "tail").await;
}

/// A self-loop (`poll` depends on itself, three rounds) that waits for a
/// signal each round
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_in_self_loop(pool: PgPool) {
    let stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let id = stack
        .submit(&format!(
            r#"
name: signal_self_loop
activities:
  - key: start
{get}
  - key: poll
{get}
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 600
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: poll
        conditions: ["{{{{ poll.status == 'completed' }}}}"]
"#
        ))
        .await;

    let (status, activities, signals) = stack.drive(id, "poll").await;
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(signals, 3, "{activities:#}");
    assert_eq!(activities["poll"]["iteration"], 3, "{activities:#}");
    stack.stop().await;
}

/// A self-loop whose wait times out and continues each round: three rounds
/// with no signal
#[sqlx::test(migrations = "../migrations")]
async fn test_timeout_continue_in_self_loop(pool: PgPool) {
    let stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let id = stack
        .submit(&format!(
            r#"
name: signal_timeout_self_loop
activities:
  - key: start
{get}
  - key: poll
{get}
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 1
        on_timeout: continue
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: poll
        conditions: ["{{{{ poll.status == 'completed' }}}}"]
"#
        ))
        .await;

    let start = Instant::now();
    let (status, activities) = loop {
        let (status, activities) = stack.workflow(id).await;
        if status == "completed" || status == "failed" || start.elapsed() > TIMEOUT {
            break (status, activities);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(activities["poll"]["iteration"], 3, "{activities:#}");
    stack.stop().await;
}
