//! End-to-end tests for `wait_for_signal` through the real stack: API server,
//! orchestrator (`run_orchestrator`) and a std worker, with signals sent to
//! `POST /api/v1/workflows/{id}/signal` exactly as a client sends them.
//!
//! - A wait outside any loop takes one signal, then runs its activity.
//! - A wait inside a loop takes one signal per round.
//! - A wait takes exactly one signal: a duplicate, concurrent or late signal
//!   is rejected (`signaled: false`) with or without `data`, the step runs
//!   once per round, and a signal accepted before the deadline cannot also
//!   time out. Some tests pause the orchestrator to hold open the window
//!   between a signal and its processing.
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
    event_source: Arc<PostgresEventSource>,
    activity_queue: Arc<dyn ActivityQueue>,
    subscriptions: Arc<dyn SubscriptionService>,
    /// The running orchestrator: its own shutdown token and task
    orchestrator: Option<(CancellationToken, tokio::task::JoinHandle<()>)>,
    /// How often the next orchestrator started checks for expired waits
    timeout_check_interval: Duration,
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

        let pool_for_config = pool.clone();
        let mut stack = Stack {
            pool,
            api_url,
            token,
            client,
            shutdown,
            manager,
            handles,
            event_source,
            activity_queue,
            subscriptions,
            orchestrator: None,
            timeout_check_interval: OrchestratorConfig::new(pool_for_config).timeout_check_interval,
        };
        stack.resume_orchestrator();
        stack
    }

    /// Start the orchestrator (again): it resumes from its durable cursor
    fn resume_orchestrator(&mut self) {
        assert!(self.orchestrator.is_none(), "orchestrator already running");
        let token = self.shutdown.child_token();
        let event_source = self.event_source.clone();
        let activity_queue = self.activity_queue.clone();
        let subscriptions = self.subscriptions.clone();
        let pool = self.pool.clone();
        let timeout_check_interval = self.timeout_check_interval;
        let orch_shutdown = token.clone();
        let handle = tokio::spawn(async move {
            run_orchestrator(
                event_source,
                activity_queue,
                subscriptions,
                OrchestratorConfig::new(pool)
                    .with_poll_interval(Duration::from_millis(1), Duration::from_millis(10))
                    .with_timeout_check_interval(timeout_check_interval),
                Some(orch_shutdown),
            )
            .await
            .expect("Orchestrator failed");
        });
        self.orchestrator = Some((token, handle));
    }

    /// Stop the orchestrator and wait for it to exit, so that signals sent
    /// meanwhile are all taken by the API before any is processed: the window
    /// between a signal and the orchestrator deleting its subscription, which
    /// orchestrator lag (load, poll backoff, a restart) opens in production
    async fn pause_orchestrator(&mut self) {
        let (token, handle) = self.orchestrator.take().expect("orchestrator not running");
        token.cancel();
        handle.await.expect("orchestrator task panicked");
    }

    async fn stop(mut self) {
        self.manager.stop(self.handles).await;
        if let Some((token, handle)) = self.orchestrator.take() {
            token.cancel();
            let _ = handle.await;
        }
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
        self.signal_opt(id, key, event_name, Some(data)).await
    }

    /// Send a signal; `None` leaves `data` out of the request body entirely
    async fn signal_opt(
        &self,
        id: Uuid,
        key: &str,
        event_name: &str,
        data: Option<Value>,
    ) -> Value {
        let mut body = json!({"activity_key": key, "event_name": event_name});
        if let Some(data) = data {
            body["data"] = data;
        }
        let r = self
            .client
            .post(format!("{}/api/v1/workflows/{id}/signal", self.api_url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::OK);
        r.json().await.unwrap()
    }

    /// Poll until `key` is waiting in loop round `round` (`Value::Null` or a
    /// number, as the materialized state has it)
    async fn wait_for_waiting(&self, id: Uuid, key: &str, round: Value) {
        let start = Instant::now();
        loop {
            let (status, activities) = self.workflow(id).await;
            if activities[key]["status"] == "waiting" && activities[key]["iteration"] == round {
                return;
            }
            assert!(
                start.elapsed() < TIMEOUT && status != "completed" && status != "failed",
                "{key} never waited in round {round}: {status} {activities:#}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Poll until the workflow reaches a terminal status
    async fn wait_terminal(&self, id: Uuid) -> (String, Value) {
        let start = Instant::now();
        loop {
            let (status, activities) = self.workflow(id).await;
            if status == "completed" || status == "failed" {
                return (status, activities);
            }
            assert!(
                start.elapsed() < TIMEOUT,
                "workflow {id} still {status}: {activities:#}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Events of one type for one activity, as (iteration, payload)
    async fn events(&self, id: Uuid, event_type: &str, key: &str) -> Vec<(Option<i32>, Value)> {
        sqlx::query_as(
            "SELECT iteration, payload FROM workflow_events \
             WHERE workflow_id = $1 AND event_type::text = $2 AND activity_key = $3 \
             ORDER BY position",
        )
        .bind(id)
        .bind(event_type)
        .bind(key)
        .fetch_all(&self.pool)
        .await
        .unwrap()
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
#[ignore = "needs #18 (fix/loop-tail-rerun): on main the step closing a two-step loop re-runs every pass"]
async fn test_signal_in_loop_head(pool: PgPool) {
    signal_in_loop(pool, "head").await;
}

/// The step that closes the loop waits for a signal each round
#[sqlx::test(migrations = "../migrations")]
#[ignore = "needs #18 (fix/loop-tail-rerun): on main the step closing a two-step loop re-runs every pass"]
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

// --- Duplicate and late signals ---------------------------------------------
//
// A wait takes exactly one signal per round: once a signal has been accepted,
// any further signal for the same wait must be answered `signaled: false`,
// whether or not either signal carries `data`, and the step must run once.

/// `start` -> `approve` (waits for `go`) -> `done`
async fn submit_approval(stack: &Stack) -> Uuid {
    let get = http_get(&stack.api_url);
    stack
        .submit(&format!(
            r#"
name: signal_duplicate
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
        .await
}

/// Two signals for one wait, both taken by the API before the orchestrator
/// processes either. The first is accepted, the second rejected; the step
/// runs once, with the first signal's data.
async fn duplicate_signal(pool: PgPool, first: Option<Value>, second: Option<Value>) {
    let mut stack = Stack::start(pool).await;
    let id = submit_approval(&stack).await;
    stack.wait_for_waiting(id, "approve", json!(0)).await;

    stack.pause_orchestrator().await;
    let r1 = stack.signal_opt(id, "approve", "go", first.clone()).await;
    let r2 = stack.signal_opt(id, "approve", "go", second).await;
    stack.resume_orchestrator();

    let (status, activities) = stack.wait_terminal(id).await;
    let signaled = stack.events(id, "ActivitySignaled", "approve").await;
    let completed = stack.events(id, "ActivityCompleted", "approve").await;
    let summary = format!("r1={r1} r2={r2}\nsignaled={signaled:?}\n{activities:#}");

    assert_eq!(status, "completed", "{summary}");
    assert_eq!(r1["signaled"], true, "{summary}");
    assert_eq!(completed.len(), 1, "approve ran more than once: {summary}");
    assert_eq!(signaled.len(), 1, "{summary}");
    assert_eq!(
        activities["approve"].get("signal_data").cloned(),
        first,
        "approve ran without the first signal's data: {summary}"
    );
    assert_eq!(
        r2["signaled"], false,
        "a second signal for a wait already signaled was accepted: {summary}"
    );
    stack.stop().await;
}

/// Two signals without `data`: the second must be rejected
#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_without_data(pool: PgPool) {
    duplicate_signal(pool, None, None).await;
}

/// Two signals with `data` (the comparison): the second is rejected
#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_with_data(pool: PgPool) {
    duplicate_signal(pool, Some(json!({"n": 1})), Some(json!({"n": 2}))).await;
}

/// A signal without `data`, then one with: the second must be rejected, not
/// acknowledged while its data is dropped
#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_without_then_with_data(pool: PgPool) {
    duplicate_signal(pool, None, Some(json!({"approved": false}))).await;
}

/// Two signals sent back to back with the orchestrator running: the second
/// is rejected, whether it reaches the API before or after the orchestrator
/// takes the first
async fn back_to_back_signals(pool: PgPool, data: Option<Value>) {
    let stack = Stack::start(pool).await;
    let id = submit_approval(&stack).await;
    stack.wait_for_waiting(id, "approve", json!(0)).await;

    let r1 = stack.signal_opt(id, "approve", "go", data.clone()).await;
    let r2 = stack.signal_opt(id, "approve", "go", data).await;

    let (status, activities) = stack.wait_terminal(id).await;
    let completed = stack.events(id, "ActivityCompleted", "approve").await;
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(completed.len(), 1, "{activities:#}");
    assert_eq!(r1["signaled"], true, "r1={r1}");
    assert_eq!(
        r2["signaled"], false,
        "second signal accepted: r1={r1} r2={r2}"
    );
    stack.stop().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_back_to_back_signals_without_data(pool: PgPool) {
    back_to_back_signals(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_back_to_back_signals_with_data(pool: PgPool) {
    back_to_back_signals(pool, Some(json!({"n": 1}))).await;
}

/// Two signals sent concurrently with the orchestrator running: exactly one
/// is accepted
async fn concurrent_signals(pool: PgPool, data: Option<Value>) {
    let stack = Stack::start(pool).await;
    let id = submit_approval(&stack).await;
    stack.wait_for_waiting(id, "approve", json!(0)).await;

    let (r1, r2) = tokio::join!(
        stack.signal_opt(id, "approve", "go", data.clone()),
        stack.signal_opt(id, "approve", "go", data),
    );

    let (status, activities) = stack.wait_terminal(id).await;
    let completed = stack.events(id, "ActivityCompleted", "approve").await;
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(completed.len(), 1, "{activities:#}");
    let accepted = [&r1, &r2].iter().filter(|r| r["signaled"] == true).count();
    assert_eq!(accepted, 1, "r1={r1} r2={r2}");
    stack.stop().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_concurrent_signals_without_data(pool: PgPool) {
    concurrent_signals(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_concurrent_signals_with_data(pool: PgPool) {
    concurrent_signals(pool, Some(json!({"n": 1}))).await;
}

/// A signal that arrives after the orchestrator has taken the wait's signal
/// (its subscription is gone) is rejected, with or without `data`
async fn late_signal(pool: PgPool, data: Option<Value>) {
    let stack = Stack::start(pool).await;
    let id = submit_approval(&stack).await;
    stack.wait_for_waiting(id, "approve", json!(0)).await;

    let r1 = stack.signal_opt(id, "approve", "go", data.clone()).await;
    let start = Instant::now();
    while stack.workflow(id).await.1["approve"]["status"] == "waiting" {
        assert!(start.elapsed() < TIMEOUT, "signal never taken");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let r2 = stack.signal_opt(id, "approve", "go", data).await;

    let (status, activities) = stack.wait_terminal(id).await;
    let completed = stack.events(id, "ActivityCompleted", "approve").await;
    assert_eq!(status, "completed", "{activities:#}");
    assert_eq!(completed.len(), 1, "{activities:#}");
    assert_eq!(r1["signaled"], true, "r1={r1}");
    assert_eq!(r2["signaled"], false, "late signal accepted: r2={r2}");
    stack.stop().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_late_signal_without_data(pool: PgPool) {
    late_signal(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_late_signal_with_data(pool: PgPool) {
    late_signal(pool, Some(json!({"n": 1}))).await;
}

/// A self-loop (three rounds) whose first round gets two signals before the
/// orchestrator processes either. The second is rejected; it must not carry
/// over to (consume) a later round's wait, and every round runs once.
async fn duplicate_signal_in_loop(pool: PgPool, data: Option<Value>) {
    let mut stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let id = stack
        .submit(&format!(
            r#"
name: signal_self_loop_duplicate
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

    stack.wait_for_waiting(id, "poll", json!(0)).await;
    stack.pause_orchestrator().await;
    let r1 = stack.signal_opt(id, "poll", "go", data.clone()).await;
    let r2 = stack.signal_opt(id, "poll", "go", data.clone()).await;
    stack.resume_orchestrator();

    // Round 1 must wait for its own signal: the duplicate did not carry over
    stack.wait_for_waiting(id, "poll", json!(1)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, activities) = stack.workflow(id).await;
    assert_eq!(
        activities["poll"]["status"], "waiting",
        "round 1 ran without a signal of its own: {activities:#}"
    );
    for round in [1, 2] {
        stack.wait_for_waiting(id, "poll", json!(round)).await;
        let r = stack.signal_opt(id, "poll", "go", data.clone()).await;
        assert_eq!(r["signaled"], true, "round {round}: {r}");
    }

    let (status, activities) = stack.wait_terminal(id).await;
    let signaled = stack.events(id, "ActivitySignaled", "poll").await;
    let completed = stack.events(id, "ActivityCompleted", "poll").await;
    let summary = format!("r1={r1} r2={r2}\nsignaled={signaled:?}\n{activities:#}");
    assert_eq!(status, "completed", "{summary}");
    assert_eq!(activities["poll"]["iteration"], 3, "{summary}");
    assert_eq!(
        completed.len(),
        3,
        "poll did not run once per round: {summary}"
    );
    let rounds: Vec<_> = signaled.iter().map(|(i, _)| *i).collect();
    assert_eq!(rounds, vec![Some(0), Some(1), Some(2)], "{summary}");
    assert_eq!(r1["signaled"], true, "{summary}");
    assert_eq!(
        r2["signaled"], false,
        "a second signal for round 0 was accepted: {summary}"
    );
    stack.stop().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_in_loop_round_without_data(pool: PgPool) {
    duplicate_signal_in_loop(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_in_loop_round_with_data(pool: PgPool) {
    duplicate_signal_in_loop(pool, Some(json!({"n": 1}))).await;
}

/// A signal accepted before the wait's deadline ends the wait: it can no
/// longer time out. Storage level: once signaled, the expiry checker's query
/// (`expire_subscriptions`) must not return the subscription.
async fn signaled_wait_cannot_expire(pool: PgPool, data: Option<Value>) {
    let mut stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let id = stack
        .submit(&format!(
            r#"
name: signal_then_expire
activities:
  - key: approve
{get}
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 1
        on_timeout: fail
"#
        ))
        .await;
    stack.wait_for_waiting(id, "approve", json!(0)).await;

    stack.pause_orchestrator().await;
    let r = stack.signal_opt(id, "approve", "go", data).await;
    assert_eq!(r["signaled"], true, "{r}");
    // Past the deadline, the signal still unprocessed (orchestrator lag)
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let expired = stack.subscriptions.expire_subscriptions(100).await.unwrap();
    assert!(
        expired.is_empty(),
        "a wait signaled before its deadline was expired: {expired:?}"
    );
    stack.resume_orchestrator();
    let (status, activities) = stack.wait_terminal(id).await;
    assert_eq!(status, "completed", "{activities:#}");
    stack.stop().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signaled_wait_cannot_expire_without_data(pool: PgPool) {
    signaled_wait_cannot_expire(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signaled_wait_cannot_expire_with_data(pool: PgPool) {
    signaled_wait_cannot_expire(pool, Some(json!({"n": 1}))).await;
}

/// End to end: a signal accepted before the deadline, the orchestrator back
/// after the deadline with its expiry checker due at once. The workflow must
/// complete: the signal won, the wait must not also fail on timeout.
async fn signal_before_deadline_completes(pool: PgPool, data: Option<Value>) {
    let mut stack = Stack::start(pool).await;
    let get = http_get(&stack.api_url);
    let id = stack
        .submit(&format!(
            r#"
name: signal_then_timeout
activities:
  - key: approve
{get}
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 1
        on_timeout: fail
  - key: done
{get}
    depends_on: [approve]
"#
        ))
        .await;
    stack.wait_for_waiting(id, "approve", json!(0)).await;

    stack.pause_orchestrator().await;
    let r = stack.signal_opt(id, "approve", "go", data).await;
    assert_eq!(r["signaled"], true, "{r}");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    stack.timeout_check_interval = Duration::from_millis(1);
    stack.resume_orchestrator();

    let (status, activities) = stack.wait_terminal(id).await;
    let failed = stack.events(id, "ActivityFailed", "approve").await;
    assert_eq!(
        status, "completed",
        "signaled before its deadline, yet failed: {failed:?}\n{activities:#}"
    );
    stack.stop().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_before_deadline_completes_without_data(pool: PgPool) {
    signal_before_deadline_completes(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_before_deadline_completes_with_data(pool: PgPool) {
    signal_before_deadline_completes(pool, Some(json!({"n": 1}))).await;
}
