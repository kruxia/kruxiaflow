//! Integration tests for the worker wake stream
//! (`GET /api/v1/workers/wake`) and the `next_due_ms` hint on
//! `POST /api/v1/workers/poll`
//! (docs/implementation/2026-11-08-activity-wake-sse.md §2.4, §2.5).
//!
//! A real server on a local port, a real `LISTEN` connection, and a plain
//! HTTP client reading the event stream.

use bcrypt::hash;
use kruxiaflow_api::{AppState, AppStateBuild, app_router};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::{EventWakeMode, PostgresEventSource};
use kruxiaflow_core::queue::{Activity, ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serde_json::json;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ============================================================================
// Test Infrastructure
// ============================================================================

fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Build application state. With `with_wake`, the event source holds a real
/// LISTEN connection and the state carries its activity wake; without, the
/// server has no wake signal (as with `KRUXIAFLOW_EVENT_WAKE=poll`).
async fn setup_state(pool: PgPool, shutdown_token: CancellationToken, with_wake: bool) -> AppState {
    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };
    let auth_service = PostgresAuthService::new(pool.clone(), auth_config)
        .expect("Failed to create test auth service");

    sqlx::query(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
    )
    .bind("wake-test-client")
    .bind(hash("wake-test-secret", 4).unwrap()) // min cost: hashing strength is not under test
    .bind("Wake Test Client")
    .execute(&pool)
    .await
    .expect("Failed to create test client");

    let mode = if with_wake {
        EventWakeMode::Notify
    } else {
        EventWakeMode::Poll
    };
    let event_source = PostgresEventSource::connect(pool.clone(), mode)
        .await
        .expect("event source");
    let activity_wake = event_source.activity_wake();
    assert_eq!(activity_wake.is_some(), with_wake);

    AppState::with_metadata(
        pool.clone(),
        Arc::new(auth_service),
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default())),
        Arc::new(event_source),
        Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone())),
        Arc::new(kruxiaflow_core::cache::NoOpCache::new()),
        Arc::new(PostgresSubscriptionService::new(pool)),
        shutdown_token,
        "0.1.0-test".to_string(),
        AppStateBuild {
            timestamp: "2026-11-08T00:00:00Z".to_string(),
            git_hash: "waketest".to_string(),
        },
        vec!["workers".to_string()],
    )
    .with_activity_wake(activity_wake)
}

/// A running test server. Dropping it stops the server task, which releases
/// the state and with it the LISTEN connection; otherwise the test database
/// cannot be torn down until that connection times out.
struct TestServer {
    addr: SocketAddr,
    token: String,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}

async fn start_server(pool: PgPool, with_wake: bool) -> TestServer {
    let shutdown = CancellationToken::new();
    let state = setup_state(pool, shutdown.clone(), with_wake).await;
    let app = app_router(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("local address");
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let token = get_token(addr).await;
    TestServer {
        addr,
        token,
        shutdown,
        task,
    }
}

async fn get_token(addr: SocketAddr) -> String {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/api/v1/oauth/token"))
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "wake-test-client",
            "client_secret": "wake-test-secret"
        }))
        .send()
        .await
        .expect("token request");
    let body: serde_json::Value = response.json().await.expect("token response");
    body["access_token"]
        .as_str()
        .expect("access_token")
        .to_string()
}

async fn open_stream(addr: SocketAddr, token: Option<&str>, worker: &str) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .get(format!("http://{addr}/api/v1/workers/wake"))
        .query(&[("worker", worker)])
        .header("Accept", "text/event-stream");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.expect("wake stream request")
}

/// What reading the stream produced
#[derive(Debug, PartialEq)]
enum Read {
    /// An event with this name
    Event(String),
    /// Nothing but (possibly) keepalive comments within the time allowed
    Quiet,
    /// The server ended the stream
    Ended,
}

/// A minimal server-sent-event reader over a response body
struct EventStream {
    response: reqwest::Response,
    buffer: String,
}

impl EventStream {
    fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: String::new(),
        }
    }

    /// The next event, or what happened instead, within `within`
    async fn next(&mut self, within: Duration) -> Read {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            // A complete block ends with a blank line
            while let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                let name = block
                    .lines()
                    .find_map(|line| line.strip_prefix("event:"))
                    .map(|name| name.trim().to_string());
                if let Some(name) = name {
                    return Read::Event(name);
                }
                // A comment-only block (keepalive): keep reading
            }

            match tokio::time::timeout_at(deadline, self.response.chunk()).await {
                Ok(Ok(Some(chunk))) => self.buffer.push_str(&String::from_utf8_lossy(&chunk)),
                Ok(Ok(None)) | Ok(Err(_)) => return Read::Ended,
                Err(_) => return Read::Quiet,
            }
        }
    }
}

fn wake() -> Read {
    Read::Event("wake".to_string())
}

fn activity(worker: &str, key: &str) -> Activity {
    Activity {
        key: key.to_string(),
        worker: worker.to_string(),
        activity_name: "echo".to_string(),
        parameters: json!({}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
        iteration: None,
        signal_data: None,
    }
}

/// A time `seconds` from now by the database's clock, which is the clock the
/// queue compares `scheduled_for` against
async fn db_time_in(pool: &PgPool, seconds: i32) -> chrono::DateTime<chrono::Utc> {
    sqlx::query_scalar("SELECT now() + make_interval(secs => $1)")
        .bind(seconds as f64)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn schedule(pool: &PgPool, activity: Activity) {
    PostgresQueue::new(pool.clone(), QueueConfig::default())
        .schedule(Uuid::now_v7(), vec![activity])
        .await
        .expect("schedule");
}

async fn poll(addr: SocketAddr, token: &str, worker: &str) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/api/v1/workers/poll"))
        .bearer_auth(token)
        .json(&json!({"worker": worker, "worker_id": "wake-test-worker", "max_activities": 1}))
        .send()
        .await
        .expect("poll request");
    assert_eq!(response.status(), 200);
    response.json().await.expect("poll response")
}

const SOON: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(400);

// ============================================================================
// Opening the stream
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_stream_requires_a_token(pool: PgPool) {
    let server = start_server(pool, true).await;
    let addr = server.addr;

    let response = open_stream(addr, None, "std").await;
    assert_eq!(response.status(), 401);

    let response = open_stream(addr, Some("not-a-token"), "std").await;
    assert_eq!(response.status(), 401);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_stream_rejects_an_empty_worker_type(pool: PgPool) {
    let server = start_server(pool, true).await;
    let (addr, token) = (server.addr, server.token.clone());

    let response = open_stream(addr, Some(&token), "  ").await;
    assert_eq!(response.status(), 400);
}

/// A server that polls only has nothing to stream: it says so, and the
/// worker stays on its timer
#[sqlx::test(migrations = "../migrations")]
async fn test_wake_stream_answers_503_without_a_wake_signal(pool: PgPool) {
    let server = start_server(pool, false).await;
    let (addr, token) = (server.addr, server.token.clone());

    let response = open_stream(addr, Some(&token), "std").await;
    assert_eq!(response.status(), 503);
}

// ============================================================================
// What the stream sends
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_stream_wakes_on_connect_and_when_work_is_scheduled(pool: PgPool) {
    let server = start_server(pool.clone(), true).await;
    let (addr, token) = (server.addr, server.token.clone());

    let response = open_stream(addr, Some(&token), "std").await;
    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"))
    );
    let mut stream = EventStream::new(response);

    // On connect: covers anything committed before the subscription
    assert_eq!(stream.next(SOON).await, wake());
    // Then nothing while nothing happens
    assert_eq!(stream.next(QUIET).await, Read::Quiet);

    schedule(&pool, activity("std", "a")).await;
    assert_eq!(stream.next(SOON).await, wake());
    assert_eq!(stream.next(QUIET).await, Read::Quiet);

    // A delayed activity announces itself when written
    let mut delayed = activity("std", "later");
    delayed.scheduled_for = Some(db_time_in(&pool, 3600).await);
    schedule(&pool, delayed).await;
    assert_eq!(stream.next(SOON).await, wake());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_wake_stream_is_per_worker_type(pool: PgPool) {
    let server = start_server(pool.clone(), true).await;
    let (addr, token) = (server.addr, server.token.clone());

    let mut std_stream = EventStream::new(open_stream(addr, Some(&token), "std").await);
    let mut other_stream = EventStream::new(open_stream(addr, Some(&token), "other").await);
    assert_eq!(std_stream.next(SOON).await, wake());
    assert_eq!(other_stream.next(SOON).await, wake());

    schedule(&pool, activity("std", "a")).await;
    assert_eq!(std_stream.next(SOON).await, wake());
    assert_eq!(other_stream.next(QUIET).await, Read::Quiet);
}

/// A wake says "poll now" and nothing else: the event has a name and an
/// empty data field, so nothing about the work can be read from the stream
#[sqlx::test(migrations = "../migrations")]
async fn test_wake_events_carry_no_data(pool: PgPool) {
    let server = start_server(pool.clone(), true).await;
    let (addr, token) = (server.addr, server.token.clone());

    let mut response = open_stream(addr, Some(&token), "std").await;
    let first = tokio::time::timeout(SOON, response.chunk())
        .await
        .expect("first chunk in time")
        .expect("readable")
        .expect("a chunk");
    let text = String::from_utf8_lossy(&first);
    assert!(text.contains("event: wake"), "got {text:?}");
    for line in text.lines().filter(|line| line.starts_with("data:")) {
        assert_eq!(line.trim_start_matches("data:").trim(), "", "got {text:?}");
    }
}

/// Graceful shutdown must not be held open by idle streams
#[sqlx::test(migrations = "../migrations")]
async fn test_wake_stream_ends_when_the_server_shuts_down(pool: PgPool) {
    let server = start_server(pool, true).await;
    let (addr, token) = (server.addr, server.token.clone());

    let mut stream = EventStream::new(open_stream(addr, Some(&token), "std").await);
    assert_eq!(stream.next(SOON).await, wake());

    server.shutdown.cancel();
    assert_eq!(stream.next(SOON).await, Read::Ended);
}

// ============================================================================
// next_due_ms on the poll response
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_reports_next_due_only_when_nothing_was_claimed(pool: PgPool) {
    let server = start_server(pool.clone(), true).await;
    let (addr, token) = (server.addr, server.token.clone());

    // Nothing pending: no hint
    let response = poll(addr, &token, "std").await;
    assert_eq!(response["count"], 0);
    assert!(response.get("next_due_ms").is_none(), "got {response}");

    // A delayed activity: an empty answer that says when to come back
    let mut delayed = activity("std", "later");
    delayed.scheduled_for = Some(db_time_in(&pool, 30).await);
    schedule(&pool, delayed).await;

    let response = poll(addr, &token, "std").await;
    assert_eq!(response["count"], 0);
    let due = response["next_due_ms"]
        .as_u64()
        .unwrap_or_else(|| panic!("next_due_ms missing in {response}"));
    assert!(
        (25_000..=30_001).contains(&due),
        "expected just under 30 s, got {due} ms"
    );

    // Another worker type is told nothing
    let response = poll(addr, &token, "other").await;
    assert!(response.get("next_due_ms").is_none(), "got {response}");

    // Something claimable now: it is returned, with no hint
    schedule(&pool, activity("std", "now")).await;
    let response = poll(addr, &token, "std").await;
    assert_eq!(response["count"], 1);
    assert_eq!(response["activities"][0]["activity_key"], "now");
    assert!(response.get("next_due_ms").is_none(), "got {response}");
}
