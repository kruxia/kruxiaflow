//! The machine shop: small workflows inspired by physical machines, run
//! against a real engine (API server, orchestrator and std worker).
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
//! and `MACHINE_SHOP_TREE` to label the tree in the results (default: the
//! branch name).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../machines")
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

struct Engine {
    url: String,
    token: String,
    pool: PgPool,
    http: reqwest::Client,
    shutdown: CancellationToken,
    /// The orchestrator, stopped by `shutdown`
    orchestrator: tokio::task::JoinHandle<()>,
    /// The API server and the worker's tasks, which are aborted
    others: Vec<tokio::task::JoinHandle<()>>,
}

async fn create_oauth_client(pool: &PgPool, client_id: &str, client_secret: &str) {
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

impl Engine {
    async fn start(pool: PgPool) -> Engine {
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
        let shutdown = CancellationToken::new();

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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind API");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = app_router(state);
        let mut others = vec![tokio::spawn(async move {
            axum::serve(listener, app).await.expect("API server");
        })];

        // {{SECRET.shop_db_url}} is the test's own database, for machines
        // that keep state across attempts in it (postgres_query)
        let secrets = HashMap::from([("shop_db_url".to_string(), test_db_url(&pool))]);
        let orchestrator = {
            let (events, queue, subscriptions, pool, shutdown) = (
                events.clone(),
                queue.clone(),
                subscriptions.clone(),
                pool.clone(),
                shutdown.clone(),
            );
            tokio::spawn(async move {
                // The timeout checker also ends expired signal waits; at the
                // default 30s, a run that waits out a short signal timeout
                // would take half a minute
                let config = OrchestratorConfig::new(pool)
                    .with_poll_interval(Duration::from_millis(1), Duration::from_millis(10))
                    .with_timeout_check_interval(Duration::from_secs(1))
                    .with_secrets(secrets);
                run_orchestrator(events, queue, subscriptions, config, Some(shutdown))
                    .await
                    .expect("orchestrator");
            })
        };

        let http = reqwest::Client::new();
        let started = Instant::now();
        loop {
            if let Ok(r) = http.get(format!("{url}/health")).send().await
                && r.status().is_success()
            {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "API not ready");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let registry = register_std_activities(Arc::new(kruxiaflow_core::NoOpCache::new()));
        let worker = WorkerManager::new(
            WorkerConfig {
                api_url: url.clone(),
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
        others.extend(worker.start().await.expect("start worker"));

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
            http,
            shutdown,
            orchestrator,
            others,
        }
    }

    /// Stop everything, so no connection outlives the test and its
    /// database can be dropped
    async fn stop(self) {
        self.shutdown.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.orchestrator).await;
        for task in self.others {
            task.abort();
            let _ = task.await;
        }
        self.pool.close().await;
    }

    /// Deploy a machine's workflow. A definition the engine rejects is the
    /// engine's answer, not the runner's failure: each of the machine's runs
    /// reports it, and the other machines still run.
    async fn deploy(&self, yaml: &str) -> Result<(String, String), String> {
        let r = self
            .http
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
            .http
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
        self.http
            .get(format!("{}/api/v1/workflows/{id}", self.url))
            .bearer_auth(&self.token)
            .send()
            .await
            .expect("status request")
            .json()
            .await
            .unwrap_or(Value::Null)
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
    async fn signal(
        &self,
        id: Uuid,
        s: &SignalSpec,
        again: bool,
        deadline: Instant,
    ) -> Result<(), String> {
        if s.early {
            return self.early_signal(id, s).await;
        }
        if s.late {
            return self.late_signal(id, s, deadline).await;
        }
        let what = format!("signal '{}' to {}", s.event, s.step);
        loop {
            let wf = self.workflow(id).await;
            let workflow = wf["status"].as_str().unwrap_or("unknown");
            let step = wf["activities"]
                .as_array()
                .and_then(|list| list.iter().find(|a| a["activity_key"] == s.step.as_str()))
                .and_then(|a| a["status"].as_str())
                .unwrap_or("absent");
            // What the step was doing, for the message if this goes wrong
            let mut last = step.to_string();
            match step {
                "waiting" => {
                    let reply = self.post_signal(id, s, &what).await?;
                    if reply["signaled"] == true {
                        return Ok(());
                    }
                    // Not delivered: the wait may not be visible yet, so try
                    // again while the step still shows waiting
                    last = format!(
                        "waiting, but the signal wasn't delivered: {}",
                        reply["message"]
                    );
                }
                "not_scheduled" | "absent" => {}
                "pending" | "running" | "completed" if again => {}
                other => {
                    return Err(format!(
                        "{what} not sent: {} is {other} and never waited",
                        s.step
                    ));
                }
            }
            if matches!(workflow, "completed" | "failed" | "cancelled") {
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

    /// `POST /api/v1/workflows/{id}/signal`; the reply, when the API
    /// answers with a success status
    async fn post_signal(&self, id: Uuid, s: &SignalSpec, what: &str) -> Result<Value, String> {
        let mut body = json!({ "activity_key": s.step, "event_name": s.event });
        if let Some(data) = &s.data {
            body["data"] = data.clone();
        }
        let r = self
            .http
            .post(format!("{}/api/v1/workflows/{id}/signal", self.url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("{what}: {e}"))?;
        let status = r.status();
        let reply: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!("{what}: signal API {status}: {reply}"));
        }
        Ok(reply)
    }

    /// Send the signal at once, while the step isn't waiting yet. The API
    /// must refuse it (`signaled: false`): a signal isn't kept for a wait
    /// that hasn't begun.
    async fn early_signal(&self, id: Uuid, s: &SignalSpec) -> Result<(), String> {
        let what = format!("early signal '{}' to {}", s.event, s.step);
        let wf = self.workflow(id).await;
        let step = wf["activities"]
            .as_array()
            .and_then(|list| list.iter().find(|a| a["activity_key"] == s.step.as_str()))
            .and_then(|a| a["status"].as_str())
            .unwrap_or("absent");
        if !matches!(step, "not_scheduled" | "absent") {
            return Err(format!("{what} not sent: {} is already {step}", s.step));
        }
        let reply = self.post_signal(id, s, &what).await?;
        if reply["signaled"] == false {
            return Ok(());
        }
        Err(format!(
            "{what} was accepted with {} {step}: {reply}",
            s.step
        ))
    }

    /// Wait until the step has waited and stopped waiting without a signal,
    /// or the workflow has ended, then send the signal once. The API must
    /// refuse it (`signaled: false`): a wait that timed out takes no signal.
    async fn late_signal(&self, id: Uuid, s: &SignalSpec, deadline: Instant) -> Result<(), String> {
        let what = format!("late signal '{}' to {}", s.event, s.step);
        loop {
            let wf = self.workflow(id).await;
            let workflow = wf["status"].as_str().unwrap_or("unknown");
            let step = wf["activities"]
                .as_array()
                .and_then(|list| list.iter().find(|a| a["activity_key"] == s.step.as_str()))
                .and_then(|a| a["status"].as_str())
                .unwrap_or("absent");
            let past_waiting = !matches!(step, "waiting" | "not_scheduled" | "absent");
            if past_waiting || matches!(workflow, "completed" | "failed" | "cancelled") {
                let reply = self.post_signal(id, s, &what).await?;
                if reply["signaled"] == false {
                    return Ok(());
                }
                return Err(format!(
                    "{what} was accepted with {} {step}: {reply}",
                    s.step
                ));
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
            if matches!(status.as_str(), "completed" | "failed" | "cancelled")
                || Instant::now() >= deadline
            {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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

    /// One named output of a step, through the public output API
    async fn step_output(&self, id: Uuid, step: &str, output: &str) -> Result<Value, String> {
        let r = self
            .http
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
}

struct Row {
    machine: String,
    kind: Kind,
    run: String,
    outcome: Outcome,
    seconds: f64,
    notes: String,
    /// The workflow's final status
    status: String,
    /// What each step did: `{status, runs}`
    steps: BTreeMap<String, Value>,
    problems: Vec<String>,
}

impl Outcome {
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

/// What a run did, as the results file records it
struct Observed {
    status: String,
    steps: BTreeMap<String, Value>,
    problems: Vec<String>,
}

/// What the run did, and everything that differs from the card, as
/// readable lines
async fn check(engine: &Engine, id: Uuid, status: &str, spec: &RunSpec) -> Observed {
    let mut problems = Vec::new();
    if status != spec.expect.status {
        let settled = matches!(status, "completed" | "failed" | "cancelled");
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
    for o in &spec.expect.outputs {
        // A step that didn't complete is already reported above
        if statuses.get(&o.step).map(String::as_str) != Some("completed") {
            continue;
        }
        match engine.step_output(id, &o.step, &o.output).await {
            Ok(v) if v == o.equals => {}
            Ok(v) => problems.push(format!(
                "{}.{} is {v}, expected {}",
                o.step, o.output, o.equals
            )),
            Err(e) => problems.push(format!("{}.{}: {e}", o.step, o.output)),
        }
    }
    Observed {
        status: status.to_string(),
        steps,
        problems,
    }
}

fn judge(machine: &Machine, run: &RunSpec, observed: Observed, seconds: f64) -> Row {
    let trap = machine.card.trap.as_ref().map(Trap::issues);
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
        status: observed.status,
        steps: observed.steps,
        problems: observed.problems,
    }
}

async fn run_one(
    engine: &Engine,
    machine: &Machine,
    def: &Result<(String, String), String>,
    run: &RunSpec,
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
                steps,
                problems: vec![e.clone()],
            };
            return judge(machine, run, observed, 0.0);
        }
    };
    let input = run.input.clone().unwrap_or_else(|| json!({}));
    let started = Instant::now();
    let deadline = started + Duration::from_secs(run.timeout_seconds);
    let id = engine.submit(&def.0, &def.1, &input).await;
    // A signal that can't be sent is a problem; the ones after it aren't sent
    let mut signal_problems = Vec::new();
    let mut signaled = BTreeSet::new();
    for s in &run.signals {
        let again = !signaled.insert(s.step.as_str());
        if let Err(p) = engine.signal(id, s, again, deadline).await {
            signal_problems.push(p);
            break;
        }
    }
    let status = engine.wait(id, deadline).await;
    let seconds = started.elapsed().as_secs_f64();
    let mut observed = check(engine, id, &status, run).await;
    signal_problems.append(&mut observed.problems);
    observed.problems = signal_problems;
    judge(machine, run, observed, seconds)
}

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

fn scoreboard(mode: Mode, rows: &[Row], machines: &[Machine], total: f64) -> String {
    let header = ["Machine", "Kind", "Run", "Result", "Seconds", "Notes"];
    let cells: Vec<[String; 6]> = rows
        .iter()
        .map(|r| {
            [
                r.machine.clone(),
                match r.kind {
                    Kind::Simple => "simple".to_string(),
                    Kind::Compound => "compound".to_string(),
                },
                r.run.clone(),
                r.outcome.label(),
                format!("{:.2}", r.seconds),
                r.notes.clone(),
            ]
        })
        .collect();
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for c in &cells {
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

    let count = |f: fn(&Outcome) -> bool| rows.iter().filter(|r| f(&r.outcome)).count();
    let mut out = format!(
        "# Machine shop scoreboard: {} mode\n\n**Tree:** {} · **Runs:** {} · **Total:** {:.2}s\n\n",
        mode.name(),
        git_describe(),
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
    out.push_str(&line(&header.map(String::from)));
    out.push_str(&format!(
        "|{}|\n",
        widths
            .iter()
            .map(|w| "-".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("|")
    ));
    for c in &cells {
        out.push_str(&line(c));
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
    let engine = Engine::start(pool).await;

    let mut defs = Vec::new();
    for m in &machines {
        defs.push(engine.deploy(&m.workflow_yaml).await);
    }

    let started_at = chrono::Utc::now();
    let started = Instant::now();
    let rows: Vec<Row> = match mode {
        Mode::Bench => {
            let mut rows = Vec::new();
            for (m, def) in machines.iter().zip(&defs) {
                for run in &m.card.runs {
                    rows.push(run_one(&engine, m, def, run).await);
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
                    .map(move |run| run_one(engine, m, def, run))
            });
            futures::future::join_all(all).await
        }
    };
    let total = started.elapsed().as_secs_f64();
    engine.stop().await;

    let board = scoreboard(mode, &rows, &machines, total);
    println!("\n{board}");
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("scoreboard dir");
    let path = dir.join(format!("{}.md", mode.name()));
    std::fs::write(&path, &board).expect("write scoreboard");
    println!("Scoreboard written to {}", path.display());
    let path = write_results(&dir, mode, &rows, started_at, total);
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
const RESULTS_SCHEMA: &str = "machine-shop-results/1";

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

fn write_results(
    dir: &Path,
    mode: Mode,
    rows: &[Row],
    started_at: chrono::DateTime<chrono::Utc>,
    total: f64,
) -> PathBuf {
    let (label, branch, commit) = tree();
    let runs: Vec<Value> = rows
        .iter()
        .map(|r| {
            // `trap` is the first issue, as before; `trap_issues` is all
            let issues = r.outcome.trap_issues();
            json!({
                "machine": r.machine,
                "run": r.run,
                "result": r.outcome.key(),
                "trap": issues.first(),
                "trap_issues": issues,
                "seconds": r.seconds,
                "status": r.status,
                "steps": r.steps,
                "problems": r.problems,
            })
        })
        .collect();
    let results = json!({
        "schema": RESULTS_SCHEMA,
        "runner": "rust",
        "mode": mode.name(),
        "engine_version": env!("CARGO_PKG_VERSION"),
        "tree": { "label": label, "branch": branch, "commit": commit },
        "started_at": started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "seconds": total,
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
/// engine normalizes it)
fn machine_view(dir: &str) -> Value {
    let base = machines_dir().join(dir);
    let card: Value =
        serde_yaml::from_str(&std::fs::read_to_string(base.join("card.yaml")).expect("card.yaml"))
            .expect("card.yaml parses");
    let wf: Value = serde_yaml::from_str(
        &std::fs::read_to_string(base.join("workflow.yaml")).expect("workflow.yaml"),
    )
    .expect("workflow.yaml parses");
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
        "workflow": { "name": wf["name"], "steps": steps },
    })
}

/// Fill each page in `machines/views/` with every machine and every
/// results file, and write it next to the results, ready to open from disk
fn bundle_views(dir: &Path) -> Vec<PathBuf> {
    const MARKER: &str = "__MACHINE_SHOP_DATA__";
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
        .filter(|v| v["schema"] == RESULTS_SCHEMA)
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
        write_atomic(&out, &template.replacen(MARKER, &data, 1));
        pages.push(out);
    }
    pages.sort();
    pages
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
    }
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
