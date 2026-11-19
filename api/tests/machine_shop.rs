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
//! A scoreboard is printed and written to
//! `$CARGO_TARGET_TMPDIR/machine-shop/<mode>.md`.
//!
//! Set `MACHINE_SHOP_ONLY=domino-run,level-crossing` to run some machines.

use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Trap {
    issue: u32,
    broken_looks_like: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSpec {
    name: String,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    expect: Expect,
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
    let keys: BTreeSet<String> = wf["activities"]
        .as_sequence()
        .unwrap_or_else(|| panic!("machines/{dir}/workflow.yaml: no activities"))
        .iter()
        .filter_map(|a| a["key"].as_str().map(str::to_string))
        .collect();
    assert!(!card.runs.is_empty(), "machines/{dir}: card has no runs");
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
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
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
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("API server");
        });

        {
            let (events, queue, subscriptions, pool, shutdown) = (
                events.clone(),
                queue.clone(),
                subscriptions.clone(),
                pool.clone(),
                shutdown.clone(),
            );
            tokio::spawn(async move {
                let config = OrchestratorConfig::new(pool)
                    .with_poll_interval(Duration::from_millis(1), Duration::from_millis(10));
                run_orchestrator(events, queue, subscriptions, config, Some(shutdown))
                    .await
                    .expect("orchestrator");
            });
        }

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
        // The handles are dropped: the worker runs until the test ends
        let _handles = worker.start().await.expect("start worker");

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
        }
    }

    async fn deploy(&self, dir: &str, yaml: &str) -> (String, String) {
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
        assert_eq!(status, 201, "machines/{dir}: deploy failed: {body}");
        (
            body["name"].as_str().unwrap().to_string(),
            body["version"].as_str().unwrap().to_string(),
        )
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

    async fn workflow_status(&self, id: Uuid) -> String {
        let body: Value = self
            .http
            .get(format!("{}/api/v1/workflows/{id}", self.url))
            .bearer_auth(&self.token)
            .send()
            .await
            .expect("status request")
            .json()
            .await
            .unwrap_or(Value::Null);
        body["status"].as_str().unwrap_or("unknown").to_string()
    }

    /// Poll until the workflow is completed, failed or cancelled, or the
    /// timeout passes. Returns the last status seen.
    async fn wait(&self, id: Uuid, timeout: Duration) -> String {
        let started = Instant::now();
        loop {
            let status = self.workflow_status(id).await;
            if matches!(status.as_str(), "completed" | "failed" | "cancelled")
                || started.elapsed() >= timeout
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

enum Outcome {
    Works,
    Broken,
    KnownBroken(u32),
    Fixed(u32),
}

impl Outcome {
    fn label(&self) -> String {
        match self {
            Outcome::Works => "works".to_string(),
            Outcome::Broken => "BROKEN".to_string(),
            Outcome::KnownBroken(n) => format!("known broken (#{n})"),
            Outcome::Fixed(n) => format!("fixed (#{n})"),
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
}

/// Everything that differs from the card, as readable lines
async fn check(engine: &Engine, id: Uuid, status: &str, spec: &RunSpec) -> Vec<String> {
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
    problems
}

fn judge(machine: &Machine, run: &RunSpec, problems: Vec<String>, seconds: f64) -> Row {
    let trap = machine.card.trap.as_ref().map(|t| t.issue);
    let outcome = match (problems.is_empty(), trap) {
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
        notes: problems.join("; "),
    }
}

async fn run_one(engine: &Engine, machine: &Machine, def: &(String, String), run: &RunSpec) -> Row {
    let input = run.input.clone().unwrap_or_else(|| json!({}));
    let started = Instant::now();
    let id = engine.submit(&def.0, &def.1, &input).await;
    let status = engine
        .wait(id, Duration::from_secs(run.timeout_seconds))
        .await;
    let seconds = started.elapsed().as_secs_f64();
    let problems = check(engine, id, &status, run).await;
    judge(machine, run, problems, seconds)
}

fn git_describe() -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };
    format!(
        "{} @ {}",
        git(&["rev-parse", "--abbrev-ref", "HEAD"]),
        git(&["rev-parse", "--short", "HEAD"])
    )
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
                "- **{}** (#{}): broken looks like: {}\n",
                m.card.name,
                t.issue,
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
        defs.push(engine.deploy(&m.dir, &m.workflow_yaml).await);
    }

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

    let board = scoreboard(mode, &rows, &machines, total);
    println!("\n{board}");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("machine-shop");
    std::fs::create_dir_all(&dir).expect("scoreboard dir");
    let path = dir.join(format!("{}.md", mode.name()));
    std::fs::write(&path, &board).expect("write scoreboard");
    println!("Scoreboard written to {}", path.display());

    for r in &rows {
        if let Outcome::Fixed(n) = r.outcome {
            println!(
                "NOTE: trap {} (#{n}) now passes: take the trap label off its card",
                r.machine
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
    }
}
