//! The machine shop: small workflows inspired by physical machines, run
//! against a real engine (API server, orchestrator and std worker). It's an
//! R&D exploration of how the engine behaves, alone and under contention.
//!
//! Each machine lives in `machines/<name>/` as a `workflow.yaml` and a
//! `card.yaml` (its spec card: the story, the engine functions it
//! exercises, and what each run should do). See `machines/README.md`.
//!
//! Two modes, one test each:
//! - **bench**: the machines run one at a time, so nothing else is running
//! - **shop**: every run of every machine starts at once in one engine
//!
//! Each run ends as one of:
//! - **works**: it did what its card says
//! - **broken**: it didn't, and it isn't a trap (fails the test)
//! - **known broken**: a trap that still shows its bug (expected)
//! - **fixed**: a trap that now passes (take the trap label off)
//!
//! Each mode writes, under `$CARGO_TARGET_TMPDIR/machine-shop/` (or
//! `MACHINE_SHOP_OUT`):
//! - `<mode>.md`: a scoreboard, also printed
//! - `results/<tree>-<mode>-<time>.json`: the results file (see
//!   `machines/README.md`), which accumulates across runs and trees
//! - one page per view in `machines/views/`, with the machines and every
//!   results file embedded, to open from disk
//!
//! Set `MACHINE_SHOP_ONLY=domino-run,level-crossing` to run some machines,
//! `MACHINE_SHOP_TREE` to label the tree in the results (default: the
//! branch name), and `MACHINE_SHOP_HOST` to label where it ran (default:
//! `<os>/<arch>`).
//!
//! Set `MACHINE_SHOP_PLUG=on` (or a seed) to give every run a twin that has
//! the plug pulled partway through: the engine's tasks are dropped at once,
//! restarted on the same database, and the twin's outcome is compared with
//! the uninterrupted run's. See "Pull the plug" in `machines/README.md`.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::{
    OrchestratorConfig, PostgresSubscriptionService, SubscriptionService, run_orchestrator,
};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_std_worker::{WorkerConfig, WorkerManager, register_std_activities};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// What the Stack draws for engineers: timelines, ladder rungs, datasheets
mod machine_shop_stack;

// ---------------------------------------------------------------------------
// Spec cards
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Card {
    name: String,
    kind: Kind,
    picture: String,
    /// The machine in an edge setting (a shop, a clinic, a vehicle, a
    /// gateway): one or two sentences, true to what it exercises
    #[serde(default)]
    edge: Option<String>,
    functions: Vec<String>,
    #[serde(default)]
    trap: Option<Trap>,
    runs: Vec<RunSpec>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Simple,
    Compound,
}

/// A trap names the issue it reproduces as `issue: 25`, or several as
/// `issues: [29, 37]` when it needs them all fixed
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Trap {
    #[serde(default)]
    issue: Option<u32>,
    #[serde(default)]
    issues: Vec<u32>,
    /// The runs that show the bug; empty means every run
    #[serde(default)]
    runs: Vec<String>,
    broken_looks_like: String,
}

impl Trap {
    /// Every issue it names, in card order
    fn issues(&self) -> Vec<u32> {
        self.issue
            .into_iter()
            .chain(self.issues.iter().copied())
            .collect()
    }

    /// The trap covers this run: it names it, or names no runs
    fn covers(&self, run: &str) -> bool {
        self.runs.is_empty() || self.runs.iter().any(|r| r == run)
    }
}

/// `#29, #37`
fn issue_list(issues: &[u32]) -> String {
    issues
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSpec {
    name: String,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    /// Signals to send, in order, each once its step is waiting
    #[serde(default)]
    signals: Vec<SignalSpec>,
    expect: Expect,
}

/// A signal the runner sends through the public API once `step` is
/// waiting. Without `data` the request has no `data` field; `data: null`
/// sends an explicit null. An `early` signal is sent at once, before the
/// step waits, and a `late` one once the step has stopped waiting without
/// one (its wait timed out); the API must refuse both.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalSpec {
    step: String,
    event: String,
    #[serde(default, deserialize_with = "present")]
    data: Option<Value>,
    #[serde(default)]
    early: bool,
    #[serde(default)]
    late: bool,
}

/// A field that is present, even as null, is `Some`; only a missing one is
/// `None` (through `#[serde(default)]`)
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

fn default_timeout() -> u64 {
    15
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    status: String,
    steps: BTreeMap<String, StepExpect>,
    #[serde(default)]
    outputs: Vec<OutputExpect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepExpect {
    status: String,
    #[serde(default)]
    runs: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputExpect {
    step: String,
    output: String,
    equals: Value,
}

struct Machine {
    dir: String,
    card: Card,
    workflow_yaml: String,
}

fn machines_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Load every machine, check each card against its workflow, and apply
/// `MACHINE_SHOP_ONLY` if set
fn load_machines() -> Vec<Machine> {
    let only: Option<BTreeSet<String>> = std::env::var("MACHINE_SHOP_ONLY")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(machines_dir())
        .expect("machines/ directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("card.yaml").is_file())
        .collect();
    dirs.sort();

    let mut machines = Vec::new();
    for path in dirs {
        let dir = path.file_name().unwrap().to_string_lossy().to_string();
        if only.as_ref().is_some_and(|o| !o.contains(&dir)) {
            continue;
        }
        let card_text = std::fs::read_to_string(path.join("card.yaml")).unwrap();
        let card: Card = serde_yaml::from_str(&card_text)
            .unwrap_or_else(|e| panic!("machines/{dir}/card.yaml: {e}"));
        let workflow_yaml = std::fs::read_to_string(path.join("workflow.yaml"))
            .unwrap_or_else(|e| panic!("machines/{dir}/workflow.yaml: {e}"));
        lint_card(&dir, &card, &workflow_yaml);
        machines.push(Machine {
            dir,
            card,
            workflow_yaml,
        });
    }
    assert!(!machines.is_empty(), "no machines to run");
    machines
}

/// A card must say what happens to every step, in every run, and only
/// name steps that exist
fn lint_card(dir: &str, card: &Card, workflow_yaml: &str) {
    let wf: serde_yaml::Value = serde_yaml::from_str(workflow_yaml)
        .unwrap_or_else(|e| panic!("machines/{dir}/workflow.yaml: {e}"));
    let activities = wf["activities"]
        .as_sequence()
        .unwrap_or_else(|| panic!("machines/{dir}/workflow.yaml: no activities"));
    let keys: BTreeSet<String> = activities
        .iter()
        .filter_map(|a| a["key"].as_str().map(str::to_string))
        .collect();
    // Steps that wait for a signal
    let waits: BTreeSet<String> = activities
        .iter()
        .filter(|a| a["settings"]["wait_for_signal"].is_mapping())
        .filter_map(|a| a["key"].as_str().map(str::to_string))
        .collect();
    assert!(!card.runs.is_empty(), "machines/{dir}: card has no runs");
    if let Some(t) = &card.trap {
        let issues = t.issues();
        assert!(
            t.issue.is_none() || t.issues.is_empty(),
            "machines/{dir}: a trap names `issue` or `issues`, not both"
        );
        assert!(!issues.is_empty(), "machines/{dir}: a trap names its issue");
        let unique: BTreeSet<u32> = issues.iter().copied().collect();
        assert_eq!(
            unique.len(),
            issues.len(),
            "machines/{dir}: a trap names each issue once"
        );
    }
    for run in &card.runs {
        let named: BTreeSet<String> = run.expect.steps.keys().cloned().collect();
        assert_eq!(
            named, keys,
            "machines/{dir}, run '{}': the card must list every step, and only those",
            run.name
        );
        for o in &run.expect.outputs {
            assert!(
                keys.contains(&o.step),
                "machines/{dir}, run '{}': output check names unknown step {}",
                run.name,
                o.step
            );
        }
        for s in &run.signals {
            assert!(
                !(s.early && s.late),
                "machines/{dir}, run '{}': a signal is early or late, not both",
                run.name
            );
            assert!(
                keys.contains(&s.step),
                "machines/{dir}, run '{}': signal to unknown step {}",
                run.name,
                s.step
            );
            assert!(
                waits.contains(&s.step),
                "machines/{dir}, run '{}': signal to {}, which has no settings.wait_for_signal",
                run.name,
                s.step
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A real engine: API server, orchestrator, std worker
// ---------------------------------------------------------------------------

/// A workflow that has ended: completed, failed or cancelled
fn is_settled(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

/// A step's status in `GET /api/v1/workflows/{id}`, or `absent`
fn step_status<'a>(wf: &'a Value, step: &str) -> &'a str {
    wf["activities"]
        .as_array()
        .and_then(|list| list.iter().find(|a| a["activity_key"] == step))
        .and_then(|a| a["status"].as_str())
        .unwrap_or("absent")
}

/// The `application_name` on the engine's own connections, so the runner
/// can count them in `pg_stat_activity` apart from its own and from those a
/// machine's `postgres_query` opens
const ENGINE_APP_NAME: &str = "kruxiaflow-machine-shop-engine";

/// The engine's processes (API server, orchestrator and std worker) in a
/// Tokio runtime of their own, with a connection pool of their own.
/// Shutting that runtime down drops every task the engine spawned, detached
/// ones included (a worker's running activities, the orchestrator's timeout
/// checker), and with them every connection they hold.
struct Plant {
    runtime: Option<tokio::runtime::Runtime>,
    shutdown: CancellationToken,
    /// The orchestrator, which closes the engine's pool when it stops
    orchestrator: Option<tokio::task::JoinHandle<()>>,
    port: u16,
}

impl Drop for Plant {
    fn drop(&mut self) {
        // A runtime can't be dropped the normal way inside another one
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

impl Plant {
    /// Start the engine on the database `pool` names, serving the API on
    /// `port`, or on any free port
    async fn start(pool: &PgPool, port: Option<u16>) -> Plant {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("machine-shop-engine")
            .enable_all()
            .build()
            .expect("engine runtime");
        // Owned by the plant at once, so a failed start still shuts it down
        let mut plant = Plant {
            runtime: Some(runtime),
            shutdown: CancellationToken::new(),
            orchestrator: None,
            port: 0,
        };
        let options = (*pool.connect_options())
            .clone()
            .application_name(ENGINE_APP_NAME);
        let max_connections = pool.options().get_max_connections();
        // {{SECRET.shop_db_url}} is the test's own database, for machines
        // that keep state across attempts in it (postgres_query)
        let secrets = HashMap::from([("shop_db_url".to_string(), test_db_url(pool))]);
        let start = start_engine(
            options,
            max_connections,
            secrets,
            port,
            plant.shutdown.clone(),
        );
        let started = plant.runtime.as_ref().unwrap().spawn(start);
        let (port, orchestrator) = started.await.expect("engine start");
        plant.port = port;
        plant.orchestrator = Some(orchestrator);
        plant
    }

    /// Stop gracefully: the orchestrator finishes the events it's on and
    /// closes the pool; then the rest is dropped
    async fn stop(mut self) {
        self.shutdown.cancel();
        if let Some(orchestrator) = self.orchestrator.take() {
            let _ = tokio::time::timeout(Duration::from_secs(10), orchestrator).await;
        }
    }
}

/// Bind the API's port: any free one, or the one it had before a restart.
/// The old listener is gone by then, but the port can take a moment to free.
async fn bind_api(port: Option<u16>) -> tokio::net::TcpListener {
    let Some(port) = port else {
        return tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind API");
    };
    let started = Instant::now();
    loop {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return listener,
            Err(e) if started.elapsed() < Duration::from_secs(5) => {
                tracing::debug!(%e, port, "API port not free yet");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("bind API on port {port} again: {e}"),
        }
    }
}

/// Start the API server, orchestrator and std worker. Runs inside the
/// engine's runtime, so everything they spawn belongs to it. Returns the
/// API's port and the orchestrator's task.
async fn start_engine(
    options: PgConnectOptions,
    max_connections: u32,
    secrets: HashMap<String, String>,
    port: Option<u16>,
    shutdown: CancellationToken,
) -> (u16, tokio::task::JoinHandle<()>) {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .expect("engine pool");
    let auth_config = AuthConfig {
        rsa_private_key_pem: include_str!("../../oauth/tests/private.pem").to_string(),
        rsa_public_key_pem: Some(include_str!("../../oauth/tests/public.pem").to_string()),
        jwt_issuer: "machine-shop".to_string(),
        jwt_audience: "machine-shop".to_string(),
        token_ttl: 3600,
    };
    let auth = PostgresAuthService::new(pool.clone(), auth_config).expect("auth service");
    create_oauth_client(&pool, "shop_client", "shop_secret").await;
    create_oauth_client(&pool, "shop_worker", "shop_worker_secret").await;

    let queue: Arc<dyn ActivityQueue> = Arc::new(PostgresQueue::new(
        pool.clone(),
        QueueConfig {
            poll_interval: Duration::from_millis(10),
            ..QueueConfig::default()
        },
    ));
    let events = Arc::new(PostgresEventSource::new(pool.clone()));
    let storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let subscriptions: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    let state = AppState::new(
        pool.clone(),
        Arc::new(auth),
        queue.clone(),
        events.clone(),
        storage.clone(),
        Arc::new(kruxiaflow_core::cache::NoOpCache::new()),
        subscriptions.clone(),
        shutdown.clone(),
    );
    let listener = bind_api(port).await;
    let port = listener.local_addr().expect("API address").port();
    let app = app_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("API server");
    });

    let orchestrator = tokio::spawn(async move {
        // The timeout checker also ends expired signal waits; at the
        // default 30s, a run that waits out a short signal timeout would
        // take half a minute
        let config = OrchestratorConfig::new(pool.clone())
            .with_poll_interval(Duration::from_millis(1), Duration::from_millis(10))
            .with_timeout_check_interval(Duration::from_secs(1))
            .with_secrets(secrets);
        // An error stops the orchestrator; say so, since twins then hang.
        // Shutting the runtime down can also end it this way.
        if let Err(e) = run_orchestrator(events, queue, subscriptions, config, Some(shutdown)).await
        {
            eprintln!("machine shop: the orchestrator stopped with an error: {e}");
        }
        // No connection may outlive the test, or its database can't be
        // dropped
        let _ = tokio::time::timeout(Duration::from_secs(5), pool.close()).await;
    });

    let registry = register_std_activities(Arc::new(kruxiaflow_core::NoOpCache::new()));
    let worker = WorkerManager::new(
        WorkerConfig {
            api_url: format!("http://127.0.0.1:{port}"),
            worker_id: format!("machine_shop_{}", Uuid::now_v7()),
            worker: "std".to_string(),
            poll_max_activities: 10,
            poll_interval: Duration::from_millis(10),
            max_concurrent_activities: 50,
            activity_timeout: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(30),
            client_id: Some("shop_worker".to_string()),
            client_secret: Some("shop_worker_secret".to_string()),
            shutdown_timeout: Duration::from_secs(5),
            ..WorkerConfig::default()
        },
        registry,
        storage,
    );
    // The worker's tasks run until the runtime stops
    let _ = worker.start().await.expect("start worker");
    (port, orchestrator)
}

/// A client for the runner. A new one after a restart, so no request goes
/// out on a kept-alive connection to the stopped server.
fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Wait until the API answers its health check
async fn wait_healthy(http: &reqwest::Client, url: &str) {
    let started = Instant::now();
    loop {
        if let Ok(r) = http.get(format!("{url}/health")).send().await
            && r.status().is_success()
        {
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "API not ready");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// An activity queue row: key, round, status, retries, retries allowed,
/// and when it was claimed
type QueueRow = (String, Option<i32>, String, i32, i32, Option<String>);

/// What the database held once the plug was pulled
struct Pulled {
    /// When the plug was pulled, by the database's clock
    at: chrono::DateTime<chrono::Utc>,
    /// When the restarted API answered, by the database's clock
    back: chrono::DateTime<chrono::Utc>,
    /// From pulling the plug to the restarted API answering
    down_seconds: f64,
    /// Each workflow's events
    events: HashMap<Uuid, i64>,
    /// The workflows that had already finished
    settled: HashSet<Uuid>,
}

/// A real engine, and the runner's view of it: the API's address, a token,
/// and the runner's own pool on the test database for reading results
struct Engine {
    url: String,
    token: String,
    pool: PgPool,
    http: std::sync::Mutex<reqwest::Client>,
    plant: tokio::sync::Mutex<Option<Plant>>,
}

async fn create_oauth_client(pool: &PgPool, client_id: &str, client_secret: &str) {
    // Hashing is slow on purpose; a restarted engine finds its clients there
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM oauth_clients WHERE client_id = $1)")
            .bind(client_id)
            .fetch_one(pool)
            .await
            .expect("look up OAuth client");
    if exists {
        return;
    }
    let secret_hash = bcrypt::hash(client_secret, bcrypt::DEFAULT_COST).unwrap();
    sqlx::query(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW()) ON CONFLICT (client_id) DO NOTHING",
    )
    .bind(client_id)
    .bind(&secret_hash)
    .bind("Machine shop")
    .execute(pool)
    .await
    .expect("create OAuth client");
}

/// The URL of this test's database, which `#[sqlx::test]` created on the
/// server `DATABASE_URL` names
fn test_db_url(pool: &PgPool) -> String {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let name = pool
        .connect_options()
        .get_database()
        .expect("the test pool names its database")
        .to_string();
    let (url, query) = match base.split_once('?') {
        Some((url, query)) => (url, format!("?{query}")),
        None => (base.as_str(), String::new()),
    };
    let path = url.rfind('/').expect("DATABASE_URL has a database path");
    format!("{}/{name}{query}", &url[..path])
}

/// How many connections the engine holds on the test database now
async fn engine_connections(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity
         WHERE datname = current_database() AND application_name = $1",
    )
    .bind(ENGINE_APP_NAME)
    .fetch_one(pool)
    .await
}

impl Engine {
    async fn start(pool: PgPool) -> Engine {
        let plant = Plant::start(&pool, None).await;
        let url = format!("http://127.0.0.1:{}", plant.port);
        let http = http_client();
        wait_healthy(&http, &url).await;

        let token: Value = http
            .post(format!("{url}/api/v1/oauth/token"))
            .json(&json!({
                "grant_type": "client_credentials",
                "client_id": "shop_client",
                "client_secret": "shop_secret"
            }))
            .send()
            .await
            .expect("token request")
            .json()
            .await
            .expect("token response");
        let token = token["access_token"]
            .as_str()
            .expect("access_token")
            .to_string();

        Engine {
            url,
            token,
            pool,
            http: std::sync::Mutex::new(http),
            plant: tokio::sync::Mutex::new(Some(plant)),
        }
    }

    fn http(&self) -> reqwest::Client {
        self.http.lock().expect("http client lock").clone()
    }

    /// Stop gracefully. The runner's pool stays open.
    async fn stop(self) {
        if let Some(plant) = self.plant.into_inner() {
            plant.stop().await;
        }
    }

    /// Pull the plug: shut the engine's runtime down at once, wait until
    /// PostgreSQL has seen its connections go, read what the database holds
    /// for `ids`, then start the engine again on the same database and the
    /// same port. A request in flight meanwhile gets no answer.
    async fn pull_plug(&self, ids: &[Uuid]) -> Pulled {
        let mut plant = self.plant.lock().await;
        let started = Instant::now();
        let port = plant.as_ref().expect("the engine is running").port;
        let at = self.db_now().await;
        // Dropping the plant drops every task the engine spawned
        drop(plant.take());
        loop {
            match engine_connections(&self.pool).await {
                Ok(0) => break,
                Ok(n) => assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "{n} engine connections still open 10s after the plug was pulled"
                ),
                Err(e) => panic!("count engine connections: {e}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let events = self.event_counts(ids).await;
        let settled = self.settled(ids).await;
        *plant = Some(Plant::start(&self.pool, Some(port)).await);
        let http = http_client();
        wait_healthy(&http, &self.url).await;
        *self.http.lock().expect("http client lock") = http;
        Pulled {
            at,
            back: self.db_now().await,
            down_seconds: started.elapsed().as_secs_f64(),
            events,
            settled,
        }
    }

    /// The database's clock, which stamps workflows and their events
    async fn db_now(&self) -> chrono::DateTime<chrono::Utc> {
        sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&self.pool)
            .await
            .expect("database clock")
    }

    /// When the workflow was created, which its timeline counts from
    async fn created_at(&self, id: Uuid) -> chrono::DateTime<chrono::Utc> {
        sqlx::query_scalar("SELECT created_at FROM workflows WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("workflow row")
    }

    /// How many events each workflow has
    async fn event_counts(&self, ids: &[Uuid]) -> HashMap<Uuid, i64> {
        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT workflow_id, count(*) FROM workflow_events
             WHERE workflow_id = ANY($1) GROUP BY workflow_id",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .expect("event counts");
        rows.into_iter().collect()
    }

    /// The workflows among `ids` that have finished
    async fn settled(&self, ids: &[Uuid]) -> HashSet<Uuid> {
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, status::text FROM workflows WHERE id = ANY($1)")
                .bind(ids)
                .fetch_all(&self.pool)
                .await
                .expect("workflow statuses");
        rows.into_iter()
            .filter(|(_, status)| is_settled(status))
            .map(|(id, _)| id)
            .collect()
    }

    /// The workflow's activity queue rows, as readable lines
    async fn queue_rows(&self, id: Uuid) -> Vec<String> {
        let rows: Vec<QueueRow> = sqlx::query_as(
            "SELECT activity_key, iteration, status::text, retry_count, max_retries,
                    to_char(claimed_at, 'HH24:MI:SS.MS')
             FROM activity_queue WHERE workflow_id = $1 ORDER BY created_at",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("activity queue rows");
        rows.into_iter()
            .map(|(key, round, status, retries, max, claimed)| {
                format!(
                    "queue row {key} (round {}): {status}, {retries} of {max} retries{}",
                    round.unwrap_or(0),
                    claimed.map_or(String::new(), |c| format!(", claimed at {c}"))
                )
            })
            .collect()
    }

    /// What the run cost, from the test database
    async fn footprint(&self, id: Uuid) -> Footprint {
        let (events, payload_bytes, queue_rows): (i64, i64, i64) = sqlx::query_as(
            "SELECT
               (SELECT count(*) FROM workflow_events WHERE workflow_id = $1),
               (SELECT coalesce(sum(pg_column_size(payload)), 0)::bigint
                  FROM workflow_events WHERE workflow_id = $1),
               (SELECT count(*) FROM activity_queue WHERE workflow_id = $1)",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("footprint");
        Footprint {
            events,
            payload_bytes,
            queue_rows,
            peak_connections: None,
        }
    }

    /// Deploy a machine's workflow. A definition the engine rejects is the
    /// engine's answer, not the runner's failure: each of the machine's runs
    /// reports it, and the other machines still run.
    async fn deploy(&self, yaml: &str) -> Result<(String, String), String> {
        let r = self
            .http()
            .post(format!("{}/api/v1/workflow_definitions", self.url))
            .bearer_auth(&self.token)
            .header("Content-Type", "text/yaml")
            .body(yaml.to_string())
            .send()
            .await
            .expect("deploy request");
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        if status != 201 {
            // The validator's own words, when it gives them
            let detail = match &body["error"]["details"] {
                Value::Null => body["error"]["message"].to_string(),
                d => d.to_string(),
            };
            return Err(format!("deploy rejected ({status}): {detail}"));
        }
        Ok((
            body["name"].as_str().unwrap().to_string(),
            body["version"].as_str().unwrap().to_string(),
        ))
    }

    async fn submit(&self, name: &str, version: &str, input: &Value) -> Uuid {
        let r = self
            .http()
            .post(format!("{}/api/v1/workflows", self.url))
            .bearer_auth(&self.token)
            .json(&json!({ "definition_name": name, "version": version, "input": input }))
            .send()
            .await
            .expect("submit request");
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        assert_eq!(status, 201, "{name}: submit failed: {body}");
        body["workflow_id"].as_str().unwrap().parse().unwrap()
    }

    /// The workflow, through the public API: `status`, and `activities`
    /// with each step's `activity_key` and `status`
    async fn workflow(&self, id: Uuid) -> Value {
        // Null when the engine is down, as while the plug is pulled
        match self
            .http()
            .get(format!("{}/api/v1/workflows/{id}", self.url))
            .bearer_auth(&self.token)
            .send()
            .await
        {
            Ok(r) => r.json().await.unwrap_or(Value::Null),
            Err(_) => Value::Null,
        }
    }

    async fn workflow_status(&self, id: Uuid) -> String {
        let body = self.workflow(id).await;
        body["status"].as_str().unwrap_or("unknown").to_string()
    }

    /// Wait until the signal's step is waiting, then send the signal, all
    /// through the public API. Gives up at `deadline`, when the workflow
    /// ends first, or when the step gets past waiting without one. `again`
    /// is a later signal to a step already signaled in this run, a loop
    /// step waiting in its next round: it waits through the round running.
    ///
    /// While the engine is down (the plug pulled) the step reads as absent,
    /// and the runner keeps asking. A signal that got no answer may have
    /// been delivered: if the step then moves on, it was.
    async fn signal(
        &self,
        id: Uuid,
        s: &SignalSpec,
        again: bool,
        deadline: Instant,
    ) -> Result<(), String> {
        if s.early {
            return self.early_signal(id, s, deadline).await;
        }
        if s.late {
            return self.late_signal(id, s, deadline).await;
        }
        let what = format!("signal '{}' to {}", s.event, s.step);
        let mut unanswered = false;
        loop {
            let wf = self.workflow(id).await;
            let workflow = wf["status"].as_str().unwrap_or("unknown");
            let step = step_status(&wf, &s.step);
            // What the step was doing, for the message if this goes wrong
            let mut last = step.to_string();
            match step {
                "waiting" => match self.post_signal(id, s, &what).await? {
                    Some(reply) if reply["signaled"] == true => return Ok(()),
                    // Not delivered: the wait may not be visible yet, so try
                    // again while the step still shows waiting
                    Some(reply) => {
                        last = format!(
                            "waiting, but the signal wasn't delivered: {}",
                            reply["message"]
                        );
                    }
                    None => {
                        unanswered = true;
                        last = "waiting, and the signal got no answer".to_string();
                    }
                },
                "not_scheduled" | "absent" => {}
                "pending" | "running" | "completed" if unanswered => return Ok(()),
                "pending" | "running" | "completed" if again => {}
                other => {
                    return Err(format!(
                        "{what} not sent: {} is {other} and never waited",
                        s.step
                    ));
                }
            }
            if is_settled(workflow) {
                return Err(format!(
                    "{what} not sent: the workflow {workflow} first, with {} {last}",
                    s.step
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{what} not sent before the run's timeout: {} is {last}",
                    s.step
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// `POST /api/v1/workflows/{id}/signal`: the reply, when the API answers
    /// with a success status, or `None` when it doesn't answer at all
    async fn post_signal(
        &self,
        id: Uuid,
        s: &SignalSpec,
        what: &str,
    ) -> Result<Option<Value>, String> {
        let mut body = json!({ "activity_key": s.step, "event_name": s.event });
        if let Some(data) = &s.data {
            body["data"] = data.clone();
        }
        let sent = self
            .http()
            .post(format!("{}/api/v1/workflows/{id}/signal", self.url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await;
        let Ok(r) = sent else {
            return Ok(None);
        };
        let status = r.status();
        let reply: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!("{what}: signal API {status}: {reply}"));
        }
        Ok(Some(reply))
    }

    /// Send the signal at once, while the step isn't waiting yet. The API
    /// must refuse it (`signaled: false`): a signal isn't kept for a wait
    /// that hasn't begun.
    async fn early_signal(
        &self,
        id: Uuid,
        s: &SignalSpec,
        deadline: Instant,
    ) -> Result<(), String> {
        let what = format!("early signal '{}' to {}", s.event, s.step);
        loop {
            let wf = self.workflow(id).await;
            let step = step_status(&wf, &s.step);
            // Null: the engine is down; ask again
            if !wf.is_null() {
                if !matches!(step, "not_scheduled" | "absent") {
                    return Err(format!("{what} not sent: {} is already {step}", s.step));
                }
                if let Some(reply) = self.post_signal(id, s, &what).await? {
                    if reply["signaled"] == false {
                        return Ok(());
                    }
                    return Err(format!(
                        "{what} was accepted with {} {step}: {reply}",
                        s.step
                    ));
                }
            }
            if Instant::now() >= deadline {
                return Err(format!("{what} not sent before the run's timeout"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Wait until the step has waited and stopped waiting without a signal,
    /// or the workflow has ended, then send the signal once. The API must
    /// refuse it (`signaled: false`): a wait that timed out takes no signal.
    async fn late_signal(&self, id: Uuid, s: &SignalSpec, deadline: Instant) -> Result<(), String> {
        let what = format!("late signal '{}' to {}", s.event, s.step);
        loop {
            let wf = self.workflow(id).await;
            let workflow = wf["status"].as_str().unwrap_or("unknown");
            let step = step_status(&wf, &s.step);
            let past_waiting = !matches!(step, "waiting" | "not_scheduled" | "absent");
            if past_waiting || is_settled(workflow) {
                // No answer: the engine went down; ask again
                if let Some(reply) = self.post_signal(id, s, &what).await? {
                    if reply["signaled"] == false {
                        return Ok(());
                    }
                    return Err(format!(
                        "{what} was accepted with {} {step}: {reply}",
                        s.step
                    ));
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{what} not sent before the run's timeout: {} is {step}",
                    s.step
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Poll until the workflow is completed, failed or cancelled, or the
    /// deadline passes. Returns the last status seen.
    async fn wait(&self, id: Uuid, deadline: Instant) -> String {
        loop {
            let status = self.workflow_status(id).await;
            if is_settled(&status) {
                return status;
            }
            if Instant::now() >= deadline {
                // The API may be down; the database has the answer
                if status == "unknown" {
                    return self.stored_status(id).await;
                }
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn stored_status(&self, id: Uuid) -> String {
        sqlx::query_scalar("SELECT status::text FROM workflows WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .expect("workflow status")
            .unwrap_or_else(|| "unknown".to_string())
    }

    async fn step_statuses(&self, id: Uuid) -> BTreeMap<String, String> {
        let (activities,): (Value,) =
            sqlx::query_as("SELECT activities FROM workflows WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .expect("workflow row");
        activities
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let s = v["status"].as_str().unwrap_or("unknown").to_string();
                        (k.clone(), s)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many times each step completed, from the event log
    async fn step_runs(&self, id: Uuid) -> BTreeMap<String, i64> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT activity_key, count(*) FROM workflow_events
             WHERE workflow_id = $1 AND event_type::text = 'ActivityCompleted'
               AND activity_key IS NOT NULL
             GROUP BY activity_key",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("event counts");
        rows.into_iter().collect()
    }

    /// The run's events from `workflow_events`, timed from the workflow's
    /// `created_at`, as the engine stored them
    async fn timeline(&self, id: Uuid) -> Vec<Value> {
        type Row = (
            String,
            Option<String>,
            Option<i32>,
            Value,
            chrono::DateTime<chrono::Utc>,
        );
        let start = self.created_at(id).await;
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT event_type::text, activity_key, iteration, payload, timestamp
             FROM workflow_events WHERE workflow_id = $1
             ORDER BY timestamp, position NULLS LAST, id",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("workflow events");
        let rows: Vec<machine_shop_stack::EventRow> = rows
            .into_iter()
            .map(
                |(event, step, round, payload, at)| machine_shop_stack::EventRow {
                    event,
                    step,
                    round,
                    payload,
                    at,
                },
            )
            .collect();
        machine_shop_stack::timeline(&rows, start)
    }

    /// One named output of a step, through the public output API
    async fn step_output(&self, id: Uuid, step: &str, output: &str) -> Result<Value, String> {
        let r = self
            .http()
            .get(format!(
                "{}/api/v1/workflows/{id}/activities/{step}/output",
                self.url
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!("output API {status}: {body}"));
        }
        // `output` is a list of `{name, type, value}`
        body["output"]
            .as_array()
            .and_then(|list| list.iter().find(|o| o["name"] == output))
            .map(|o| o["value"].clone())
            .ok_or_else(|| format!("no output '{output}' in {}", body["output"]))
    }
}

// ---------------------------------------------------------------------------
// Running and judging
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Bench,
    Shop,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Bench => "bench",
            Mode::Shop => "shop",
        }
    }
}

/// A trap's outcome carries every issue the trap names
enum Outcome {
    Works,
    Broken,
    KnownBroken(Vec<u32>),
    Fixed(Vec<u32>),
}

impl Outcome {
    fn label(&self) -> String {
        match self {
            Outcome::Works => "works".to_string(),
            Outcome::Broken => "BROKEN".to_string(),
            Outcome::KnownBroken(n) => format!("known broken ({})", issue_list(n)),
            Outcome::Fixed(n) => format!("fixed ({})", issue_list(n)),
        }
    }

    /// The trap's issues, or none for a machine that isn't a trap
    fn trap_issues(&self) -> &[u32] {
        match self {
            Outcome::KnownBroken(n) | Outcome::Fixed(n) => n,
            _ => &[],
        }
    }

    /// The result's name in the results file
    fn key(&self) -> &'static str {
        match self {
            Outcome::Works => "works",
            Outcome::Broken => "broken",
            Outcome::KnownBroken(_) => "known_broken",
            Outcome::Fixed(_) => "fixed",
        }
    }
}

struct Row {
    machine: String,
    kind: Kind,
    run: String,
    outcome: Outcome,
    seconds: f64,
    notes: String,
    observed: Observed,
    /// What the run cost; none for a workflow that wasn't deployed
    footprint: Option<Footprint>,
    /// The run again, with the plug pulled partway through (plug mode)
    twin: Option<Twin>,
}

/// What a run did: as the results file records it, and as a twin is
/// compared with it
#[derive(Debug, Clone, Default)]
struct Observed {
    /// The workflow's final status
    status: String,
    /// The workflow ended before the run's deadline
    settled: bool,
    /// What each step did: `{status, runs}`
    steps: BTreeMap<String, Value>,
    /// Each output the card checks, as `step.output`, for the steps that
    /// completed: its value, or `{"error": ...}` when it couldn't be read
    outputs: BTreeMap<String, Value>,
    problems: Vec<String>,
    timeline: Option<Vec<Value>>,
}

/// What a run cost the engine, read from the test database when it ended
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Footprint {
    /// Rows in `workflow_events` for the workflow
    events: i64,
    /// Those events' payloads as stored (`pg_column_size` of the jsonb),
    /// without row headers or indexes
    payload_bytes: i64,
    /// Rows in `activity_queue` for the workflow: one per step and round,
    /// since a retry reuses its row
    queue_rows: i64,
    /// The most connections the engine held on the test database while the
    /// run went, sampled; bench mode only, where nothing else runs
    peak_connections: Option<i64>,
}

/// What the run did, and everything that differs from the card, as
/// readable lines
async fn check(engine: &Engine, id: Uuid, status: &str, spec: &RunSpec) -> Observed {
    let mut problems = Vec::new();
    let settled = is_settled(status);
    if status != spec.expect.status {
        problems.push(if settled {
            format!("workflow {status}, expected {}", spec.expect.status)
        } else {
            format!(
                "workflow still {status} after {}s, expected {}",
                spec.timeout_seconds, spec.expect.status
            )
        });
    }
    let statuses = engine.step_statuses(id).await;
    let runs = engine.step_runs(id).await;
    let steps: BTreeMap<String, Value> = statuses
        .keys()
        .chain(spec.expect.steps.keys())
        .map(|k| {
            let status = statuses.get(k).map(String::as_str).unwrap_or("absent");
            let n = runs.get(k).copied().unwrap_or(0);
            (k.clone(), json!({ "status": status, "runs": n }))
        })
        .collect();
    for (step, want) in &spec.expect.steps {
        let got = statuses.get(step).map(String::as_str).unwrap_or("absent");
        if got != want.status {
            problems.push(format!("{step} is {got}, expected {}", want.status));
        }
        if let Some(n) = want.runs {
            let got_runs = runs.get(step).copied().unwrap_or(0);
            if got_runs != n {
                problems.push(format!("{step} ran {got_runs} times, expected {n}"));
            }
        }
    }
    let mut outputs = BTreeMap::new();
    for o in &spec.expect.outputs {
        // A step that didn't complete is already reported above
        if statuses.get(&o.step).map(String::as_str) != Some("completed") {
            continue;
        }
        let key = format!("{}.{}", o.step, o.output);
        match engine.step_output(id, &o.step, &o.output).await {
            Ok(v) => {
                if v != o.equals {
                    problems.push(format!("{key} is {v}, expected {}", o.equals));
                }
                outputs.insert(key, v);
            }
            Err(e) => {
                problems.push(format!("{key}: {e}"));
                outputs.insert(key, json!({ "error": e }));
            }
        }
    }
    Observed {
        status: status.to_string(),
        settled,
        steps,
        outputs,
        problems,
        timeline: Some(engine.timeline(id).await),
    }
}

fn judge(machine: &Machine, run: &RunSpec, observed: Observed, seconds: f64) -> Row {
    let trap = machine
        .card
        .trap
        .as_ref()
        .filter(|t| t.covers(&run.name))
        .map(Trap::issues);
    let outcome = match (observed.problems.is_empty(), trap) {
        (true, None) => Outcome::Works,
        (true, Some(n)) => Outcome::Fixed(n),
        (false, None) => Outcome::Broken,
        (false, Some(n)) => Outcome::KnownBroken(n),
    };
    Row {
        machine: machine.dir.clone(),
        kind: machine.card.kind,
        run: run.name.clone(),
        outcome,
        seconds,
        notes: observed.problems.join("; "),
        observed,
        footprint: None,
        twin: None,
    }
}

/// Send the run's signals, then wait for the workflow to end or the
/// deadline to pass. The last status seen, and the signals that couldn't be
/// sent: after the first of those, the rest aren't sent.
async fn drive(
    engine: &Engine,
    id: Uuid,
    run: &RunSpec,
    deadline: Instant,
) -> (String, Vec<String>) {
    let mut problems = Vec::new();
    let mut signaled = BTreeSet::new();
    for s in &run.signals {
        let again = !signaled.insert(s.step.as_str());
        if let Err(p) = engine.signal(id, s, again, deadline).await {
            problems.push(p);
            break;
        }
    }
    (engine.wait(id, deadline).await, problems)
}

/// Samples how many connections the engine holds on the test database,
/// every 25 ms, and keeps the most seen: since the last `reset`, and overall
struct Sampler {
    peak: Arc<AtomicI64>,
    overall: Arc<AtomicI64>,
    /// Samples since the last `reset`; a peak needs at least one
    samples: Arc<AtomicI64>,
    stop: CancellationToken,
}

impl Sampler {
    const EVERY: Duration = Duration::from_millis(25);

    fn start(pool: PgPool) -> Sampler {
        let s = Sampler {
            peak: Arc::new(AtomicI64::new(0)),
            overall: Arc::new(AtomicI64::new(0)),
            samples: Arc::new(AtomicI64::new(0)),
            stop: CancellationToken::new(),
        };
        let (peak, overall, samples, stop) = (
            s.peak.clone(),
            s.overall.clone(),
            s.samples.clone(),
            s.stop.clone(),
        );
        tokio::spawn(async move {
            loop {
                if let Ok(n) = engine_connections(&pool).await {
                    peak.fetch_max(n, Ordering::Relaxed);
                    overall.fetch_max(n, Ordering::Relaxed);
                    samples.fetch_add(1, Ordering::Relaxed);
                }
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = tokio::time::sleep(Sampler::EVERY) => {}
                }
            }
        });
        s
    }

    fn reset(&self) {
        self.samples.store(0, Ordering::Relaxed);
        self.peak.store(0, Ordering::Relaxed);
    }

    /// The most since the last `reset`, if a sample was taken since
    fn peak(&self) -> Option<i64> {
        (self.samples.load(Ordering::Relaxed) > 0).then(|| self.peak.load(Ordering::Relaxed))
    }

    fn overall(&self) -> Option<i64> {
        Some(self.overall.load(Ordering::Relaxed)).filter(|n| *n > 0)
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn run_one(
    engine: &Engine,
    machine: &Machine,
    def: &Result<(String, String), String>,
    run: &RunSpec,
    sampler: Option<&Sampler>,
) -> Row {
    let def = match def {
        Ok(def) => def,
        // Nothing ran: every step is absent, and the rejection is the problem
        Err(e) => {
            let steps = run
                .expect
                .steps
                .keys()
                .map(|k| (k.clone(), json!({ "status": "absent", "runs": 0 })))
                .collect();
            let observed = Observed {
                status: "not_deployed".to_string(),
                settled: true,
                steps,
                outputs: BTreeMap::new(),
                problems: vec![e.clone()],
                timeline: None,
            };
            return judge(machine, run, observed, 0.0);
        }
    };
    let input = run.input.clone().unwrap_or_else(|| json!({}));
    if let Some(s) = sampler {
        s.reset();
    }
    let started = Instant::now();
    let deadline = started + Duration::from_secs(run.timeout_seconds);
    let id = engine.submit(&def.0, &def.1, &input).await;
    let (status, mut problems) = drive(engine, id, run, deadline).await;
    let seconds = started.elapsed().as_secs_f64();
    let peak_connections = sampler.and_then(Sampler::peak);
    let mut observed = check(engine, id, &status, run).await;
    problems.append(&mut observed.problems);
    observed.problems = problems;
    let footprint = Footprint {
        peak_connections,
        ..engine.footprint(id).await
    };
    Row {
        footprint: Some(footprint),
        ..judge(machine, run, observed, seconds)
    }
}

// ---------------------------------------------------------------------------
// Pull the plug
// ---------------------------------------------------------------------------

/// Plug mode, from `MACHINE_SHOP_PLUG`: each run gets a twin, and the
/// engine is stopped abruptly partway through it, then restarted
#[derive(Debug, Clone, Copy, PartialEq)]
struct Plug {
    /// Picks where the plug is pulled; the same seed picks the same points
    seed: u64,
    /// How much longer than its card's timeout a twin may take, when the
    /// uninterrupted run finished in time
    grace: Duration,
}

/// A step whose worker was stopped mid-activity is run again only once its
/// claim times out: the engine's default activity timeout is 60 s, and the
/// timeout checker runs every second
const DEFAULT_PLUG_GRACE_SECONDS: u64 = 75;

/// A twin of a run that never finished stops waiting for the workflow to
/// end this long past the card's timeout. It then has until the grace runs
/// out to reach the same state as the uninterrupted run.
const UNSETTLED_TWIN_SLACK: Duration = Duration::from_secs(5);

/// `MACHINE_SHOP_PLUG`: off when unset, empty, `0`, `off`, `false` or `no`;
/// `on`, `true` or `yes` is seed 1; a whole number is the seed
fn parse_plug(value: Option<&str>) -> Result<Option<u64>, String> {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    match v.to_ascii_lowercase().as_str() {
        "0" | "off" | "false" | "no" => Ok(None),
        "on" | "true" | "yes" => Ok(Some(1)),
        n => n
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("MACHINE_SHOP_PLUG={v}: use on, off, or a seed (a whole number)")),
    }
}

fn plug_config() -> Option<Plug> {
    let seed = parse_plug(std::env::var("MACHINE_SHOP_PLUG").ok().as_deref())
        .unwrap_or_else(|e| panic!("{e}"))?;
    let grace = std::env::var("MACHINE_SHOP_PLUG_GRACE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            s.trim().parse::<u64>().unwrap_or_else(|_| {
                panic!("MACHINE_SHOP_PLUG_GRACE={s}: a whole number of seconds")
            })
        })
        .unwrap_or(DEFAULT_PLUG_GRACE_SECONDS);
    Some(Plug {
        seed,
        grace: Duration::from_secs(grace),
    })
}

/// FNV-1a over the seed and each part: stable across platforms and Rust
/// versions, so a seed always picks the same points
fn fnv1a(seed: u64, parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    seed.to_le_bytes().into_iter().for_each(&mut eat);
    for part in parts {
        eat(0xff);
        part.bytes().for_each(&mut eat);
    }
    h
}

/// In bench mode, the plug is pulled once the twin has this many events:
/// somewhere from the first to the one before the last of the
/// uninterrupted run's `events`. None when it had fewer than two.
fn plug_point(seed: u64, machine: &str, run: &str, events: i64) -> Option<i64> {
    let span = u64::try_from(events - 1).ok().filter(|n| *n > 0)?;
    let k = 1 + fnv1a(seed, &[machine, run]) % span;
    i64::try_from(k).ok()
}

/// In shop mode, the plug is pulled once, for every twin, when the twins
/// together have this share of the uninterrupted runs' events: 25% to 75%
fn shop_plug_fraction(seed: u64) -> f64 {
    0.25 + (fnv1a(seed, &["shop"]) % 501) as f64 / 1000.0
}

/// How a twin came out against the uninterrupted run
#[derive(Debug, Clone, Copy, PartialEq)]
enum TwinOutcome {
    /// Same workflow status, step statuses and run counts, and outputs
    Identical,
    Different,
    /// The uninterrupted run finished; the twin didn't, even with the grace
    Hung,
    /// The plug wasn't pulled mid-run: the twin had finished first
    Missed,
}

impl TwinOutcome {
    fn key(self) -> &'static str {
        match self {
            TwinOutcome::Identical => "identical",
            TwinOutcome::Different => "different",
            TwinOutcome::Hung => "hung",
            TwinOutcome::Missed => "missed",
        }
    }
}

/// Where the plug was pulled on a twin
#[derive(Debug, Clone)]
struct PlugPoint {
    /// Bench mode: the plug was pulled once the twin had this many events.
    /// None in shop mode, where it's pulled once for every twin.
    after_event: Option<i64>,
    /// The twin's events in the database once the engine had stopped
    events_at_plug: i64,
    /// The uninterrupted run's events
    of_events: i64,
    /// From pulling the plug to the restarted API answering
    down_seconds: f64,
    /// The twin had already finished when the plug was pulled
    settled_before: bool,
    /// When the plug was pulled and when the API answered again, by the
    /// database's clock
    at: chrono::DateTime<chrono::Utc>,
    back: chrono::DateTime<chrono::Utc>,
}

struct Twin {
    outcome: TwinOutcome,
    /// How it differs from the uninterrupted run, as readable lines
    differences: Vec<String>,
    /// None when the plug was never pulled
    plug: Option<PlugPoint>,
    seconds: f64,
    observed: Observed,
    footprint: Footprint,
    /// When the twin's workflow was created, which its timeline counts from
    created_at: chrono::DateTime<chrono::Utc>,
}

/// How the twin differs from the uninterrupted run: the workflow's status,
/// each step's status and run count, and each output the card checks
fn differences(original: &Observed, twin: &Observed) -> Vec<String> {
    let mut out = Vec::new();
    if original.status != twin.status {
        out.push(format!(
            "workflow {}, uninterrupted {}",
            twin.status, original.status
        ));
    }
    let step = |o: &Observed, k: &str| -> (String, i64) {
        o.steps.get(k).map_or(("absent".to_string(), 0), |v| {
            (
                v["status"].as_str().unwrap_or("absent").to_string(),
                v["runs"].as_i64().unwrap_or(0),
            )
        })
    };
    let keys: BTreeSet<&String> = original.steps.keys().chain(twin.steps.keys()).collect();
    for k in keys {
        let ((was, was_runs), (now, now_runs)) = (step(original, k), step(twin, k));
        if was != now {
            out.push(format!("{k} {now}, uninterrupted {was}"));
        }
        if was_runs != now_runs {
            out.push(format!(
                "{k} ran {now_runs} times, uninterrupted {was_runs}"
            ));
        }
    }
    let keys: BTreeSet<&String> = original.outputs.keys().chain(twin.outputs.keys()).collect();
    let shown = |v: Option<&Value>| v.map_or("not read".to_string(), Value::to_string);
    for k in keys {
        let (was, now) = (original.outputs.get(k), twin.outputs.get(k));
        if was != now {
            out.push(format!(
                "{k} is {}, uninterrupted {}",
                shown(now),
                shown(was)
            ));
        }
    }
    out
}

fn twin_outcome(
    original: &Observed,
    twin: &Observed,
    plugged_mid_run: bool,
    differences: &[String],
) -> TwinOutcome {
    if !plugged_mid_run {
        TwinOutcome::Missed
    } else if original.settled && !twin.settled {
        TwinOutcome::Hung
    } else if differences.is_empty() {
        TwinOutcome::Identical
    } else {
        TwinOutcome::Different
    }
}

/// How long a twin may take: its card's timeout, plus the grace when the
/// uninterrupted run finished, or just enough for the restart when it
/// didn't
fn twin_timeout(run: &RunSpec, original: &Observed, plug: &Plug) -> Duration {
    let extra = if original.settled {
        plug.grace
    } else {
        UNSETTLED_TWIN_SLACK
    };
    Duration::from_secs(run.timeout_seconds) + extra
}

/// Read what the twin did and compare it with the uninterrupted run. A
/// twin still running that doesn't match yet is read again until it does,
/// it ends, or `settle_by` passes: a step whose worker was stopped
/// mid-activity runs again only once its claim times out.
#[allow(clippy::too_many_arguments)]
async fn finish_twin(
    engine: &Engine,
    id: Uuid,
    run: &RunSpec,
    (status, signal_problems): (String, Vec<String>),
    started: Instant,
    driven_at: Instant,
    settle_by: Instant,
    original: &Observed,
    plug: Option<PlugPoint>,
) -> Twin {
    let mut status = status;
    let mut caught_up = false;
    let (mut observed, mut differences) = loop {
        let observed = check(engine, id, &status, run).await;
        let differences = differences(original, &observed);
        if differences.is_empty() || observed.settled || Instant::now() >= settle_by {
            break (observed, differences);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        status = engine.stored_status(id).await;
        caught_up = true;
    };
    // The twin's own time: until it was driven to the end, or until it
    // caught up
    let ended = if caught_up { Instant::now() } else { driven_at };
    let seconds = ended.duration_since(started).as_secs_f64();
    let mut problems = signal_problems;
    problems.append(&mut observed.problems);
    observed.problems = problems;
    let footprint = engine.footprint(id).await;
    let mid_run = plug.as_ref().is_some_and(|p| !p.settled_before);
    let outcome = twin_outcome(original, &observed, mid_run, &differences);
    // What the queue holds is the first thing to look at in a finding
    if matches!(outcome, TwinOutcome::Different | TwinOutcome::Hung) {
        differences.extend(engine.queue_rows(id).await);
    }
    Twin {
        outcome,
        differences,
        plug,
        seconds,
        observed,
        footprint,
        created_at: engine.created_at(id).await,
    }
}

/// Bench mode: run a twin of `original` on its own, and pull the plug once
/// it has as many events as `plug_point` picks
async fn twin_bench(
    engine: &Engine,
    machine: &Machine,
    def: &Result<(String, String), String>,
    run: &RunSpec,
    original: &Row,
    plug: &Plug,
) -> Option<Twin> {
    let def = def.as_ref().ok()?;
    let of_events = original.footprint?.events;
    let k = plug_point(plug.seed, &machine.dir, &run.name, of_events)?;
    let input = run.input.clone().unwrap_or_else(|| json!({}));
    let started = Instant::now();
    let deadline = started + twin_timeout(run, &original.observed, plug);
    let id = engine.submit(&def.0, &def.1, &input).await;
    let done = Cell::new(false);
    let driving = async {
        let r = drive(engine, id, run, deadline).await;
        done.set(true);
        (r, Instant::now())
    };
    let watching = async {
        while !done.get() {
            let n = engine.event_counts(&[id]).await.get(&id).copied();
            if n.unwrap_or(0) >= k {
                return Some(engine.pull_plug(&[id]).await);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    };
    let ((driven, driven_at), pulled) = tokio::join!(driving, watching);
    let point = pulled.map(|p| PlugPoint {
        after_event: Some(k),
        events_at_plug: p.events.get(&id).copied().unwrap_or(0),
        of_events,
        down_seconds: p.down_seconds,
        settled_before: p.settled.contains(&id),
        at: p.at,
        back: p.back,
    });
    let settle_by = started + Duration::from_secs(run.timeout_seconds) + plug.grace;
    let twin = finish_twin(
        engine,
        id,
        run,
        driven,
        started,
        driven_at,
        settle_by,
        &original.observed,
        point,
    );
    Some(twin.await)
}

/// Shop mode: start a twin of every run at once, and pull the plug once,
/// when the twins together have `shop_plug_fraction` of the uninterrupted
/// runs' events. One twin per row, in order; none for a run that wasn't
/// deployed.
async fn twins_shop(
    engine: &Engine,
    machines: &[Machine],
    defs: &[Result<(String, String), String>],
    rows: &[Row],
    plug: &Plug,
) -> Vec<Option<Twin>> {
    struct Started<'a> {
        row: usize,
        run: &'a RunSpec,
        id: Uuid,
        started: Instant,
        deadline: Instant,
    }
    let specs = machines
        .iter()
        .zip(defs)
        .flat_map(|(m, def)| m.card.runs.iter().map(move |run| (def, run)));
    let mut twins = Vec::new();
    for (row, (def, run)) in specs.enumerate() {
        let Ok(def) = def else { continue };
        let input = run.input.clone().unwrap_or_else(|| json!({}));
        let started = Instant::now();
        let deadline = started + twin_timeout(run, &rows[row].observed, plug);
        let id = engine.submit(&def.0, &def.1, &input).await;
        twins.push(Started {
            row,
            run,
            id,
            started,
            deadline,
        });
    }
    let ids: Vec<Uuid> = twins.iter().map(|t| t.id).collect();
    let total: i64 = twins
        .iter()
        .filter_map(|t| rows[t.row].footprint.map(|f| f.events))
        .sum();
    let threshold = ((total as f64 * shop_plug_fraction(plug.seed)).ceil() as i64).max(1);

    let running = Cell::new(twins.len());
    let driving = futures::future::join_all(twins.iter().map(|t| {
        let running = &running;
        async move {
            let r = drive(engine, t.id, t.run, t.deadline).await;
            running.set(running.get() - 1);
            (r, Instant::now())
        }
    }));
    let watching = async {
        while running.get() > 0 {
            let n: i64 = engine.event_counts(&ids).await.values().sum();
            if n >= threshold {
                return Some(engine.pull_plug(&ids).await);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    };
    let (driven, pulled) = tokio::join!(driving, watching);

    // Read them all at once, so one twin catching up doesn't add to
    // another's time
    let finishing = twins.iter().zip(driven).map(|(t, (driven, driven_at))| {
        let original = &rows[t.row];
        let point = pulled.as_ref().map(|p| PlugPoint {
            after_event: None,
            events_at_plug: p.events.get(&t.id).copied().unwrap_or(0),
            of_events: original.footprint.map_or(0, |f| f.events),
            down_seconds: p.down_seconds,
            settled_before: p.settled.contains(&t.id),
            at: p.at,
            back: p.back,
        });
        let settle_by = t.started + Duration::from_secs(t.run.timeout_seconds) + plug.grace;
        finish_twin(
            engine,
            t.id,
            t.run,
            driven,
            t.started,
            driven_at,
            settle_by,
            &original.observed,
            point,
        )
    });
    let finished = futures::future::join_all(finishing).await;
    let mut out: Vec<Option<Twin>> = rows.iter().map(|_| None).collect();
    for (t, twin) in twins.iter().zip(finished) {
        out[t.row] = Some(twin);
    }
    out
}

// ---------------------------------------------------------------------------
// Footprints, the host, and the scoreboard
// ---------------------------------------------------------------------------

/// A machine's runs in one results set, added up
#[derive(Debug, Clone, Default, PartialEq)]
struct MachineFootprint {
    runs: usize,
    /// Runs with a footprint (not those whose workflow wasn't deployed)
    measured: usize,
    /// Wall time, all runs together, and the longest
    seconds: f64,
    seconds_max: f64,
    /// Over the measured runs
    events: i64,
    payload_bytes: i64,
    queue_rows: i64,
    /// The most over the runs that sampled it
    peak_connections: Option<i64>,
}

/// Each machine's footprint, from `(machine, seconds, footprint)` per run
fn machine_footprints<'a>(
    runs: impl IntoIterator<Item = (&'a str, f64, Option<&'a Footprint>)>,
) -> BTreeMap<String, MachineFootprint> {
    let mut out: BTreeMap<String, MachineFootprint> = BTreeMap::new();
    for (machine, seconds, footprint) in runs {
        let m = out.entry(machine.to_string()).or_default();
        m.runs += 1;
        m.seconds += seconds;
        m.seconds_max = m.seconds_max.max(seconds);
        if let Some(f) = footprint {
            m.measured += 1;
            m.events += f.events;
            m.payload_bytes += f.payload_bytes;
            m.queue_rows += f.queue_rows;
            if let Some(p) = f.peak_connections {
                m.peak_connections = Some(m.peak_connections.map_or(p, |q| q.max(p)));
            }
        }
    }
    out
}

/// Where the shop ran: `MACHINE_SHOP_HOST`, or `<os>/<arch>`. Never the
/// hostname or the user.
fn host_label(configured: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH))
}

fn host() -> String {
    host_label(std::env::var("MACHINE_SHOP_HOST").ok().as_deref())
}

/// What the plug does, and what it doesn't: said in the scoreboard, the
/// README and the Shop Floor's legend alike
const PLUG_SIMULATES: &str = "Pulling the plug aborts the engine's in-process tasks (API server, \
orchestrator and std worker) at once, which drops their connections and rolls back their open \
transactions. It is not a process kill and not a database power loss: PostgreSQL keeps running \
throughout.";

fn git(args: &[&str]) -> String {
    {
        std::process::Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// The tree under test: its label, branch and commit
fn tree() -> (String, String, String) {
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let commit = git(&["rev-parse", "--short", "HEAD"]);
    let label = std::env::var("MACHINE_SHOP_TREE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| branch.clone());
    (label, branch, commit)
}

fn git_describe() -> String {
    let (label, _, commit) = tree();
    format!("{label} @ {commit}")
}

/// A markdown table, its columns padded to line up
fn table(header: &[&str], cells: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for c in cells {
        for (i, v) in c.iter().enumerate() {
            widths[i] = widths[i].max(v.chars().count());
        }
    }
    let line = |vals: &[String]| {
        let padded: Vec<String> = vals
            .iter()
            .zip(&widths)
            .map(|(v, w)| format!("{v}{}", " ".repeat(w - v.chars().count())))
            .collect();
        format!("| {} |\n", padded.join(" | "))
    };
    let mut out = line(&header.iter().map(|h| h.to_string()).collect::<Vec<_>>());
    out.push_str(&format!(
        "|{}|\n",
        widths
            .iter()
            .map(|w| "-".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("|")
    ));
    for c in cells {
        out.push_str(&line(c));
    }
    out
}

/// Where the plug was pulled, in words
fn plug_text(p: &PlugPoint) -> String {
    let at = match p.after_event {
        Some(k) => format!("after event {k} of {}", p.of_events),
        None => format!("shop-wide, uninterrupted run had {} events", p.of_events),
    };
    format!(
        "{at}; {} committed when it stopped; down {:.2}s{}",
        p.events_at_plug,
        p.down_seconds,
        if p.settled_before {
            "; already finished"
        } else {
            ""
        }
    )
}

struct SetInfo {
    host: String,
    plug: Option<Plug>,
    /// The most connections the engine held at once over the set, sampled
    peak_connections: Option<i64>,
    /// The engine pool's size
    pool_max: u32,
}

fn scoreboard(
    mode: Mode,
    rows: &[Row],
    machines: &[Machine],
    total: f64,
    info: &SetInfo,
) -> String {
    let plugged = rows.iter().any(|r| r.twin.is_some());
    let mut header = vec![
        "Machine", "Kind", "Run", "Result", "Seconds", "Events", "Rows",
    ];
    if plugged {
        header.push("Twin");
    }
    header.push("Notes");
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let f = |get: fn(&Footprint) -> i64| {
                r.footprint.map_or("-".to_string(), |x| get(&x).to_string())
            };
            let mut c = vec![
                r.machine.clone(),
                match r.kind {
                    Kind::Simple => "simple".to_string(),
                    Kind::Compound => "compound".to_string(),
                },
                r.run.clone(),
                r.outcome.label(),
                format!("{:.2}", r.seconds),
                f(|x| x.events),
                f(|x| x.queue_rows),
            ];
            if plugged {
                c.push(
                    r.twin
                        .as_ref()
                        .map_or("-".to_string(), |t| match t.outcome {
                            TwinOutcome::Identical => "identical".to_string(),
                            other => other.key().to_uppercase(),
                        }),
                );
            }
            c.push(r.notes.clone());
            c
        })
        .collect();

    let count = |f: fn(&Outcome) -> bool| rows.iter().filter(|r| f(&r.outcome)).count();
    let mut out = format!(
        "# Machine shop scoreboard: {} mode\n\n**Tree:** {} · **Host:** {} · **Runs:** {} · **Total:** {:.2}s\n\n",
        mode.name(),
        git_describe(),
        info.host,
        rows.len(),
        total
    );
    out.push_str(&format!(
        "works {} · broken {} · known broken {} · fixed {}\n\n",
        count(|o| matches!(o, Outcome::Works)),
        count(|o| matches!(o, Outcome::Broken)),
        count(|o| matches!(o, Outcome::KnownBroken(_))),
        count(|o| matches!(o, Outcome::Fixed(_))),
    ));
    out.push_str(&table(&header, &cells));

    out.push_str("\n## Footprint by machine\n\n");
    out.push_str(&format!(
        "Events, payload bytes and queue rows are read from the test database when each run ends. \
         The engine's pool holds at most {} connections; the most it held at once over the set, \
         sampled every 25 ms, was {}.\n\n",
        info.pool_max,
        info.peak_connections
            .map_or("not sampled".to_string(), |n| n.to_string())
    ));
    let footprints = machine_footprints(
        rows.iter()
            .map(|r| (r.machine.as_str(), r.seconds, r.footprint.as_ref())),
    );
    let cells: Vec<Vec<String>> = footprints
        .iter()
        .map(|(m, f)| {
            vec![
                m.clone(),
                f.runs.to_string(),
                format!("{:.2}", f.seconds),
                format!("{:.2}", f.seconds_max),
                f.events.to_string(),
                f.payload_bytes.to_string(),
                f.queue_rows.to_string(),
                f.peak_connections
                    .map_or("-".to_string(), |n| n.to_string()),
            ]
        })
        .collect();
    out.push_str(&table(
        &[
            "Machine",
            "Runs",
            "Seconds",
            "Longest",
            "Events",
            "Payload bytes",
            "Queue rows",
            "Peak connections",
        ],
        &cells,
    ));

    if plugged {
        let twins: Vec<(&Row, &Twin)> = rows
            .iter()
            .filter_map(|r| r.twin.as_ref().map(|t| (r, t)))
            .collect();
        let n = |o: TwinOutcome| twins.iter().filter(|(_, t)| t.outcome == o).count();
        out.push_str("\n## Pull the plug\n\n");
        out.push_str(PLUG_SIMULATES);
        out.push_str(&format!(
            "\n\nidentical {} · different {} · hung {} · missed {}\n",
            n(TwinOutcome::Identical),
            n(TwinOutcome::Different),
            n(TwinOutcome::Hung),
            n(TwinOutcome::Missed),
        ));
        for (r, t) in twins
            .iter()
            .filter(|(_, t)| t.outcome != TwinOutcome::Identical)
        {
            out.push_str(&format!(
                "\n- **{} / {}**: {}, {}. {}",
                r.machine,
                r.run,
                t.outcome.key(),
                t.plug
                    .as_ref()
                    .map_or("the plug was never pulled".to_string(), plug_text),
                if t.differences.is_empty() {
                    "No differences.".to_string()
                } else {
                    format!("{}.", t.differences.join("; "))
                }
            ));
        }
        out.push('\n');
    }

    let traps: Vec<&Machine> = machines.iter().filter(|m| m.card.trap.is_some()).collect();
    if !traps.is_empty() {
        out.push_str("\n## Traps\n\n");
        for m in traps {
            let t = m.card.trap.as_ref().unwrap();
            out.push_str(&format!(
                "- **{}** ({}): broken looks like: {}\n",
                m.card.name,
                issue_list(&t.issues()),
                t.broken_looks_like.trim()
            ));
        }
    }
    out
}

async fn run_shop(pool: PgPool, mode: Mode) {
    let machines = load_machines();
    let plug = plug_config();
    let engine = Engine::start(pool.clone()).await;
    let sampler = Sampler::start(pool.clone());

    let mut defs = Vec::new();
    for m in &machines {
        defs.push(engine.deploy(&m.workflow_yaml).await);
    }

    let started_at = chrono::Utc::now();
    let started = Instant::now();
    let mut rows: Vec<Row> = match mode {
        Mode::Bench => {
            let mut rows = Vec::new();
            for (m, def) in machines.iter().zip(&defs) {
                for run in &m.card.runs {
                    rows.push(run_one(&engine, m, def, run, Some(&sampler)).await);
                }
            }
            rows
        }
        Mode::Shop => {
            let engine = &engine;
            let all = machines.iter().zip(&defs).flat_map(|(m, def)| {
                m.card
                    .runs
                    .iter()
                    .map(move |run| run_one(engine, m, def, run, None))
            });
            futures::future::join_all(all).await
        }
    };
    let total = started.elapsed().as_secs_f64();
    let info = SetInfo {
        host: host(),
        plug,
        peak_connections: sampler.overall(),
        pool_max: pool.options().get_max_connections(),
    };
    drop(sampler);
    engine.stop().await;

    // The twins run after the uninterrupted runs, on an engine of their
    // own: one engine at a time on the database
    if let Some(plug) = &plug {
        println!(
            "Pull the plug: running a twin of every run (seed {})",
            plug.seed
        );
        let engine = Engine::start(pool.clone()).await;
        let twins = match mode {
            Mode::Bench => {
                let mut twins = Vec::new();
                let specs = machines
                    .iter()
                    .zip(&defs)
                    .flat_map(|(m, def)| m.card.runs.iter().map(move |run| (m, def, run)));
                for ((m, def, run), row) in specs.zip(&rows) {
                    twins.push(twin_bench(&engine, m, def, run, row, plug).await);
                }
                twins
            }
            Mode::Shop => twins_shop(&engine, &machines, &defs, &rows, plug).await,
        };
        for (row, twin) in rows.iter_mut().zip(twins) {
            row.twin = twin;
        }
        engine.stop().await;
    }
    pool.close().await;

    let board = scoreboard(mode, &rows, &machines, total, &info);
    println!("\n{board}");
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("scoreboard dir");
    let path = dir.join(format!("{}.md", mode.name()));
    std::fs::write(&path, &board).expect("write scoreboard");
    println!("Scoreboard written to {}", path.display());
    let path = write_results(&dir, mode, &rows, started_at, total, &info);
    println!("Results written to {}", path.display());
    for page in bundle_views(&dir) {
        println!("View written to {}", page.display());
    }

    for r in &rows {
        if let Outcome::Fixed(n) = &r.outcome {
            println!(
                "NOTE: trap {} ({}) now passes: take the trap label off its card",
                r.machine,
                issue_list(n)
            );
        }
        if let Some(t) = &r.twin
            && matches!(t.outcome, TwinOutcome::Different | TwinOutcome::Hung)
        {
            println!(
                "PLUG: {} / {}: the twin came out {} ({}): {}",
                r.machine,
                r.run,
                t.outcome.key(),
                t.plug.as_ref().map_or(String::new(), plug_text),
                t.differences.join("; ")
            );
        }
    }
    let broken: Vec<String> = rows
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Broken))
        .map(|r| format!("{} / {}: {}", r.machine, r.run, r.notes))
        .collect();
    assert!(broken.is_empty(), "broken machines:\n{}", broken.join("\n"));
}

// ---------------------------------------------------------------------------
// Results files and views
// ---------------------------------------------------------------------------

/// The results format's version, written into every results file
const RESULTS_SCHEMA: &str = "machine-shop-results/2";

/// The versions the views read: /2 adds the host, footprints and twins to
/// /1, and the views render a /1 set without them
const RESULTS_SCHEMAS_READ: [&str; 2] = ["machine-shop-results/1", RESULTS_SCHEMA];

fn out_dir() -> PathBuf {
    std::env::var("MACHINE_SHOP_OUT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("machine-shop"))
}

/// Write the whole file, then rename it into place, so a reader never
/// sees half of it
fn write_atomic(path: &Path, contents: &str) {
    let tmp = path.with_extension(format!("tmp-{}", Uuid::now_v7()));
    std::fs::write(&tmp, contents).expect("write temporary file");
    std::fs::rename(&tmp, path).expect("move file into place");
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn footprint_json(f: &Footprint) -> Value {
    json!({
        "events": f.events,
        "payload_bytes": f.payload_bytes,
        "queue_rows": f.queue_rows,
        "peak_engine_connections": f.peak_connections,
    })
}

fn twin_json(t: &Twin) -> Value {
    json!({
        "outcome": t.outcome.key(),
        "plug": t.plug.as_ref().map(|p| json!({
            "after_event": p.after_event,
            "events_at_plug": p.events_at_plug,
            "of_events": p.of_events,
            "down_seconds": p.down_seconds,
            "settled_before": p.settled_before,
            "at_ms": machine_shop_stack::ms_after(p.at, t.created_at),
            "back_ms": machine_shop_stack::ms_after(p.back, t.created_at),
        })),
        "seconds": t.seconds,
        "status": t.observed.status,
        "steps": t.observed.steps,
        "differences": t.differences,
        "problems": t.observed.problems,
        "footprint": footprint_json(&t.footprint),
        "timeline": t.observed.timeline,
    })
}

fn write_results(
    dir: &Path,
    mode: Mode,
    rows: &[Row],
    started_at: chrono::DateTime<chrono::Utc>,
    total: f64,
    info: &SetInfo,
) -> PathBuf {
    let (label, branch, commit) = tree();
    let runs: Vec<Value> = rows
        .iter()
        .map(|r| {
            // `trap` is the first issue, as before; `trap_issues` is all
            let issues = r.outcome.trap_issues();
            let mut run = json!({
                "machine": r.machine,
                "run": r.run,
                "result": r.outcome.key(),
                "trap": issues.first(),
                "trap_issues": issues,
                "seconds": r.seconds,
                "status": r.observed.status,
                "steps": r.observed.steps,
                "problems": r.observed.problems,
                "footprint": r.footprint.as_ref().map(footprint_json),
                "twin": r.twin.as_ref().map(twin_json),
            });
            // Optional: older results files have no timeline
            if let Some(t) = &r.observed.timeline {
                run["timeline"] = json!(t);
            }
            run
        })
        .collect();
    let machine_footprints: BTreeMap<String, Value> = machine_footprints(
        rows.iter()
            .map(|r| (r.machine.as_str(), r.seconds, r.footprint.as_ref())),
    )
    .into_iter()
    .map(|(m, f)| {
        let v = json!({
            "runs": f.runs,
            "measured": f.measured,
            "seconds": f.seconds,
            "seconds_max": f.seconds_max,
            "events": f.events,
            "payload_bytes": f.payload_bytes,
            "queue_rows": f.queue_rows,
            "peak_engine_connections": f.peak_connections,
        });
        (m, v)
    })
    .collect();
    let results = json!({
        "schema": RESULTS_SCHEMA,
        "runner": "rust",
        "mode": mode.name(),
        "engine_version": env!("CARGO_PKG_VERSION"),
        "tree": { "label": label, "branch": branch, "commit": commit },
        "host": info.host,
        "started_at": started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "seconds": total,
        "footprint": {
            "peak_engine_connections": info.peak_connections,
            "engine_pool_max_connections": info.pool_max,
        },
        "plug": info.plug.map(|p| json!({
            "seed": p.seed,
            "grace_seconds": p.grace.as_secs(),
            "shop_fraction": (mode == Mode::Shop).then(|| shop_plug_fraction(p.seed)),
        })),
        "machine_footprints": machine_footprints,
        "runs": runs,
    });
    let dir = dir.join("results");
    std::fs::create_dir_all(&dir).expect("results dir");
    let path = dir.join(format!(
        "{}-{}-{}.json",
        slug(&label),
        mode.name(),
        started_at.format("%Y%m%dT%H%M%SZ")
    ));
    write_atomic(&path, &serde_json::to_string_pretty(&results).unwrap());
    path
}

/// A machine as the views see it: its card, plus its steps and what each
/// one depends on (with `dependency_of` folded into `depends_on`, as the
/// engine normalizes it), and for the Stack its drawing number, ladder
/// rungs, pins and ratings
fn machine_view(dir: &str) -> Value {
    let base = machines_dir().join(dir);
    let card: Value =
        serde_yaml::from_str(&std::fs::read_to_string(base.join("card.yaml")).expect("card.yaml"))
            .expect("card.yaml parses");
    let source = std::fs::read_to_string(base.join("workflow.yaml")).expect("workflow.yaml");
    let wf: Value = serde_yaml::from_str(&source).expect("workflow.yaml parses");
    let acts = wf["activities"].as_array().cloned().unwrap_or_default();

    // A dependency is a bare key or `{activity_key, conditions}`
    let dep = |d: &Value| -> Option<(String, Value)> {
        match d {
            Value::String(k) => Some((k.clone(), json!([]))),
            Value::Object(m) => Some((
                m.get("activity_key")?.as_str()?.to_string(),
                m.get("conditions").cloned().unwrap_or(json!([])),
            )),
            _ => None,
        }
    };
    let mut depends: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for a in &acts {
        let key = a["key"].as_str().unwrap_or_default().to_string();
        let list = depends.entry(key.clone()).or_default();
        for d in a["depends_on"].as_array().into_iter().flatten() {
            if let Some((k, c)) = dep(d) {
                list.push(json!({ "step": k, "conditions": c }));
            }
        }
        for d in a["dependency_of"].as_array().into_iter().flatten() {
            if let Some((target, c)) = dep(d) {
                depends
                    .entry(target)
                    .or_default()
                    .push(json!({ "step": key, "conditions": c }));
            }
        }
    }
    let steps: Vec<Value> = acts
        .iter()
        .map(|a| {
            let key = a["key"].as_str().unwrap_or_default();
            json!({
                "key": key,
                "activity": a["activity_name"],
                "worker": a["worker"],
                "depends_on": depends.get(key).cloned().unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "dir": dir,
        "card": card,
        "workflow": { "name": wf["name"], "steps": steps, "source": source },
        "drawing": machine_shop_stack::drawing_number(dir),
        "ladder": machine_shop_stack::ladder(&wf),
        "pins": machine_shop_stack::pins(&wf),
        "ratings": machine_shop_stack::ratings(&wf),
    })
}

/// Fill each page in `machines/views/` with every machine and every
/// results file, and write it next to the results, ready to open from disk
fn bundle_views(dir: &Path) -> Vec<PathBuf> {
    const MARKER: &str = "__MACHINE_SHOP_DATA__";
    const GLOSSARY_MARKER: &str = "__MACHINE_SHOP_GLOSSARY__";
    let mut machine_dirs: Vec<String> = std::fs::read_dir(machines_dir())
        .expect("machines/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("card.yaml").is_file())
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    machine_dirs.sort();
    let machines: Vec<Value> = machine_dirs.iter().map(|d| machine_view(d)).collect();

    let mut results: Vec<Value> = std::fs::read_dir(dir.join("results"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|v| {
            v["schema"]
                .as_str()
                .is_some_and(|s| RESULTS_SCHEMAS_READ.contains(&s))
        })
        .collect();
    results.sort_by(|a, b| {
        a["started_at"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["started_at"].as_str().unwrap_or_default())
    });

    let data = json!({ "machines": machines, "results": results });
    // Inside <script>, "</" would end the element; "\/" is the same JSON
    let data = serde_json::to_string(&data).unwrap().replace("</", "<\\/");
    let glossary = serde_json::to_string(&glossary())
        .unwrap()
        .replace("</", "<\\/");

    let mut pages = Vec::new();
    let views = machines_dir().join("views");
    for path in std::fs::read_dir(&views)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
    {
        if path.extension().is_none_or(|x| x != "html") {
            continue;
        }
        let template = std::fs::read_to_string(&path).expect("view template");
        if !template.contains(MARKER) {
            continue;
        }
        let out = dir.join(path.file_name().unwrap());
        let page = template
            .replacen(MARKER, &data, 1)
            .replacen(GLOSSARY_MARKER, &glossary, 1);
        write_atomic(&out, &page);
        pages.push(out);
    }
    pages.sort();
    pages
}

/// The views' glossary (`machines/views/glossary.json`): what their words mean in
/// Kruxia Flow, and the docs that explain them
fn glossary() -> Value {
    let path = machines_dir().join("views").join("glossary.json");
    let text = std::fs::read_to_string(&path).expect("machines/views/glossary.json");
    serde_json::from_str(&text).expect("machines/views/glossary.json is JSON")
}

/// A GitHub heading anchor: lower case, spaces to hyphens, other punctuation dropped
fn heading_anchor(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// Every glossary term is complete, and every docs link names a file and heading that exist
#[test]
fn glossary_is_valid() {
    let g = glossary();
    let root = machines_dir().join("..");
    let terms = g["terms"].as_array().expect("terms");
    assert!(!terms.is_empty(), "no terms");
    let mut ids = std::collections::HashSet::new();
    let mut phrases = std::collections::HashSet::new();
    for t in terms {
        let id = t["id"].as_str().expect("id");
        assert!(ids.insert(id), "{id}: id used twice");
        for field in ["name", "means"] {
            assert!(
                t[field].as_str().is_some_and(|s| !s.is_empty()),
                "{id}: no {field}"
            );
        }
        let kind = t["kind"].as_str().unwrap_or_default();
        assert!(kind == "kruxia" || kind == "analogy", "{id}: kind {kind}");
        if kind == "analogy" {
            assert!(
                t["analog"].as_str().is_some(),
                "{id}: an analogy needs analog"
            );
        }
        let pages = t["pages"].as_array().expect("pages");
        assert!(
            !pages.is_empty()
                && pages
                    .iter()
                    .all(|p| matches!(p.as_str(), Some("floor" | "stack"))),
            "{id}: pages"
        );
        let matches = t["match"].as_array().expect("match");
        assert!(!matches.is_empty(), "{id}: nothing to match");
        for m in matches {
            let m = m.as_str().expect("match phrase");
            for page in pages {
                assert!(
                    phrases.insert((page.as_str().unwrap().to_string(), m.to_lowercase())),
                    "{id}: {m} already names another term on {page}"
                );
            }
        }
        for doc in t["docs"].as_array().into_iter().flatten() {
            let path = doc[1].as_str().expect("docs path");
            let path = path.strip_prefix("shop:").unwrap_or(path);
            let (file, anchor) = path.split_once('#').unwrap_or((path, ""));
            let text = std::fs::read_to_string(root.join(file))
                .unwrap_or_else(|_| panic!("{id}: no file {file}"));
            if !anchor.is_empty() {
                let found = text
                    .lines()
                    .filter(|l| l.starts_with('#'))
                    .any(|l| heading_anchor(l.trim_start_matches('#')) == anchor);
                assert!(found, "{id}: no heading #{anchor} in {file}");
            }
        }
    }
}

/// Every machine, one at a time
#[sqlx::test(migrations = "../migrations")]
async fn machine_shop_bench(pool: PgPool) {
    run_shop(pool, Mode::Bench).await;
}

/// Every run of every machine, all at once in one engine
#[sqlx::test(migrations = "../migrations")]
async fn machine_shop_shop(pool: PgPool) {
    run_shop(pool, Mode::Shop).await;
}

/// Cards are read even when the engine isn't run
#[test]
fn machine_cards_are_valid() {
    let machines = load_machines();
    for m in &machines {
        assert!(!m.card.name.is_empty() && !m.card.picture.trim().is_empty());
        assert!(!m.card.functions.is_empty(), "{}: no functions", m.dir);
        assert!(
            m.card.edge.as_deref().is_some_and(|e| !e.trim().is_empty()),
            "{}: no edge reading",
            m.dir
        );
        for r in m.card.trap.iter().flat_map(|t| &t.runs) {
            assert!(
                m.card.runs.iter().any(|x| &x.name == r),
                "{}: the trap names a run the card doesn't have: {r}",
                m.dir
            );
        }
    }
}

/// A trap that names its runs covers only those
#[test]
fn trap_covers_only_the_runs_it_names() {
    let text = CARD_WITHOUT_EDGE.replace(
        "runs:\n",
        "trap:\n  issue: 7\n  runs: [once]\n  broken_looks_like: it breaks\nruns:\n",
    );
    let card: Card = serde_yaml::from_str(&text).unwrap();
    let trap = card.trap.as_ref().unwrap();
    assert!(trap.covers("once"));
    assert!(!trap.covers("twice"));
    let every: Trap = serde_yaml::from_str("issue: 7\nbroken_looks_like: it breaks").unwrap();
    assert!(every.covers("once") && every.covers("twice"));
}

const CARD_WITHOUT_EDGE: &str = r#"
name: Test bench
kind: simple
picture: A bench.
functions: [a step]
runs:
  - name: once
    expect:
      status: completed
      steps:
        a: { status: completed, runs: 1 }
"#;

/// `edge` is optional to the parser; `machine_cards_are_valid` requires it
/// of every card in `machines/`
#[test]
fn card_parses_without_edge() {
    let card: Card = serde_yaml::from_str(CARD_WITHOUT_EDGE).unwrap();
    assert!(card.edge.is_none());
    assert_eq!(card.runs.len(), 1);
}

#[test]
fn card_parses_with_edge() {
    let text = CARD_WITHOUT_EDGE.replace(
        "functions:",
        "edge: >\n  A till waits for a manager,\n  then falls back.\nfunctions:",
    );
    let card: Card = serde_yaml::from_str(&text).unwrap();
    assert_eq!(
        card.edge.as_deref(),
        Some("A till waits for a manager, then falls back.\n")
    );
}

#[test]
fn card_rejects_unknown_fields() {
    let text = CARD_WITHOUT_EDGE.replace("functions:", "edges: nope\nfunctions:");
    assert!(serde_yaml::from_str::<Card>(&text).is_err());
}

/// The views open from disk, so they must not load anything from a
/// network. Also rewrites the pages from the results already there, so a
/// view can be changed and looked at without running the machines.
#[test]
fn machine_shop_views() {
    let views = machines_dir().join("views");
    let mut seen = 0;
    for path in std::fs::read_dir(&views)
        .expect("machines/views/")
        .filter_map(|e| e.ok().map(|e| e.path()))
    {
        if path.extension().is_none_or(|x| x != "html") {
            continue;
        }
        seen += 1;
        let page = std::fs::read_to_string(&path).unwrap();
        let name = path.display();
        assert!(
            page.contains("__MACHINE_SHOP_DATA__"),
            "{name}: no data marker"
        );
        assert!(
            page.contains("__MACHINE_SHOP_GLOSSARY__"),
            "{name}: no glossary marker"
        );
        // Links out (to an issue) are fine; loads are not
        for banned in [
            "<script src",
            "<link",
            "@import",
            "url(http",
            "src=\"http",
            "fetch(",
        ] {
            assert!(!page.contains(banned), "{name}: contains {banned}");
        }
    }
    assert!(seen > 0, "no views in machines/views/");
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("output dir");
    for page in bundle_views(&dir) {
        println!("View written to {}", page.display());
    }
}

fn observed(status: &str, steps: &[(&str, &str, i64)], outputs: &[(&str, Value)]) -> Observed {
    Observed {
        status: status.to_string(),
        settled: is_settled(status),
        steps: steps
            .iter()
            .map(|(k, s, n)| (k.to_string(), json!({ "status": s, "runs": n })))
            .collect(),
        outputs: outputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        problems: Vec::new(),
        timeline: None,
    }
}

#[test]
fn twin_identical_when_status_steps_and_outputs_match() {
    let a = observed(
        "completed",
        &[("lever", "completed", 1), ("right", "skipped", 0)],
        &[("lever.echo", json!({ "track": "left" }))],
    );
    // Problems against the card don't count: only what happened
    let mut b = a.clone();
    b.problems
        .push("a signal that couldn't be sent".to_string());
    let diffs = differences(&a, &b);
    assert!(diffs.is_empty(), "{diffs:?}");
    assert_eq!(twin_outcome(&a, &b, true, &diffs), TwinOutcome::Identical);
}

#[test]
fn twin_different_lists_each_difference() {
    let a = observed(
        "completed",
        &[("pull", "completed", 1), ("mow", "completed", 1)],
        &[("mow.echo", json!({ "pulls": 3 }))],
    );
    let b = observed(
        "failed",
        &[
            ("pull", "failed", 0),
            ("mow", "skipped", 0),
            ("extra", "completed", 2),
        ],
        &[],
    );
    let diffs = differences(&a, &b);
    assert_eq!(
        diffs,
        vec![
            "workflow failed, uninterrupted completed",
            "extra completed, uninterrupted absent",
            "extra ran 2 times, uninterrupted 0",
            "mow skipped, uninterrupted completed",
            "mow ran 0 times, uninterrupted 1",
            "pull failed, uninterrupted completed",
            "pull ran 0 times, uninterrupted 1",
            "mow.echo is not read, uninterrupted {\"pulls\":3}",
        ]
    );
    assert_eq!(twin_outcome(&a, &b, true, &diffs), TwinOutcome::Different);
}

#[test]
fn twin_hung_only_when_the_uninterrupted_run_finished() {
    let done = observed("completed", &[("a", "completed", 1)], &[]);
    let stuck = observed("running", &[("a", "running", 0)], &[]);
    let diffs = differences(&done, &stuck);
    assert_eq!(twin_outcome(&done, &stuck, true, &diffs), TwinOutcome::Hung);
    // Both stuck the same way: the twin did what the uninterrupted run did
    let diffs = differences(&stuck, &stuck);
    assert_eq!(
        twin_outcome(&stuck, &stuck, true, &diffs),
        TwinOutcome::Identical
    );
}

#[test]
fn twin_missed_when_the_plug_was_not_pulled_mid_run() {
    let done = observed("completed", &[("a", "completed", 1)], &[]);
    let stuck = observed("running", &[("a", "running", 0)], &[]);
    let diffs = differences(&done, &stuck);
    assert_eq!(
        twin_outcome(&done, &stuck, false, &diffs),
        TwinOutcome::Missed
    );
}

#[test]
fn footprints_add_up_per_machine() {
    let f = |events, payload_bytes, queue_rows, peak| Footprint {
        events,
        payload_bytes,
        queue_rows,
        peak_connections: peak,
    };
    let (a, b, c) = (
        f(10, 1000, 3, Some(4)),
        f(14, 1500, 4, Some(6)),
        f(7, 700, 2, None),
    );
    let runs = [
        ("toaster", 1.5, Some(&a)),
        ("toaster", 2.5, Some(&b)),
        ("toaster", 0.0, None),
        ("galley", 0.25, Some(&c)),
    ];
    let m = machine_footprints(runs);
    assert_eq!(
        m["toaster"],
        MachineFootprint {
            runs: 3,
            measured: 2,
            seconds: 4.0,
            seconds_max: 2.5,
            events: 24,
            payload_bytes: 2500,
            queue_rows: 7,
            peak_connections: Some(6),
        }
    );
    assert_eq!(m["galley"].peak_connections, None);
    assert_eq!(m["galley"].events, 7);
    assert_eq!(m.len(), 2);
}

#[test]
fn plug_setting_parses() {
    for off in [
        None,
        Some(""),
        Some(" "),
        Some("0"),
        Some("off"),
        Some("False"),
        Some("no"),
    ] {
        assert_eq!(parse_plug(off), Ok(None), "{off:?}");
    }
    for on in ["1", "on", "TRUE", "yes"] {
        assert_eq!(parse_plug(Some(on)), Ok(Some(1)), "{on}");
    }
    assert_eq!(parse_plug(Some(" 42 ")), Ok(Some(42)));
    assert!(parse_plug(Some("sometimes")).is_err());
    assert!(parse_plug(Some("-3")).is_err());
}

#[test]
fn plug_points_fall_inside_the_run_and_repeat() {
    assert_eq!(plug_point(1, "toaster", "dark", 0), None);
    assert_eq!(plug_point(1, "toaster", "dark", 1), None);
    assert_eq!(plug_point(1, "toaster", "dark", 2), Some(1));
    for events in 2..60 {
        for seed in 0..20 {
            let k = plug_point(seed, "galley", "stroke 1", events).unwrap();
            assert!((1..events).contains(&k), "seed {seed}: {k} of {events}");
            assert_eq!(plug_point(seed, "galley", "stroke 1", events), Some(k));
        }
    }
    // Different seeds pick different points
    let picks: BTreeSet<i64> = (0..20)
        .filter_map(|seed| plug_point(seed, "galley", "stroke 1", 40))
        .collect();
    assert!(picks.len() > 5, "{picks:?}");
    for seed in 0..100 {
        let f = shop_plug_fraction(seed);
        assert!((0.25..=0.75).contains(&f), "seed {seed}: {f}");
    }
}

#[test]
fn host_label_is_the_setting_or_os_and_arch() {
    assert_eq!(host_label(Some(" pi-5 ")), "pi-5");
    let default = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    assert_eq!(host_label(None), default);
    assert_eq!(host_label(Some("  ")), default);
}
