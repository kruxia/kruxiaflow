//! `POST /api/v1/workflows/{id}/signal` against a real subscription row (#33).
//!
//! A wait takes one signal: once a signal is accepted, the API answers any
//! further signal for that wait `signaled: false`, with or without `data`,
//! and a wait whose timeout has expired takes no signal. The orchestrator is
//! not running, which holds open the window between a signal and the
//! orchestrator deleting the subscription (orchestrator lag in production).

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use bcrypt::hash;
use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::events::{
    EventCursor, EventError, EventSource, NewWorkflowEvent, PostgresEventSource, WorkflowEvent,
};
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_core::subscription::NewSubscription;
use kruxiaflow_core::workflow::OnTimeout;
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Api {
    server: TestServer,
    token: String,
    subscriptions: Arc<PostgresSubscriptionService>,
    pool: PgPool,
}

async fn start(pool: PgPool) -> Api {
    start_with(pool.clone(), Arc::new(PostgresEventSource::new(pool))).await
}

/// An event source whose first publish fails, as a database error would
struct FailFirstPublish {
    inner: PostgresEventSource,
    failed: AtomicBool,
}

#[async_trait::async_trait]
impl EventSource for FailFirstPublish {
    async fn publish(&self, event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
        if !self.failed.swap(true, Ordering::SeqCst) {
            return Err(EventError::Database(sqlx::Error::Protocol(
                "connection lost".to_string(),
            )));
        }
        self.inner.publish(event).await
    }

    async fn poll(&self, consumer_id: &str) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
        self.inner.poll(consumer_id).await
    }

    async fn update_position(
        &self,
        consumer_id: &str,
        cursor: EventCursor,
    ) -> kruxiaflow_core::events::Result<()> {
        self.inner.update_position(consumer_id, cursor).await
    }
}

async fn start_with(pool: PgPool, event_source: Arc<dyn EventSource>) -> Api {
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
    .expect("auth service");
    sqlx::query(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
    )
    .bind("test-client")
    .bind(hash("test-secret", 4).unwrap()) // min cost: hashing strength is not under test
    .bind("Test Client")
    .execute(&pool)
    .await
    .expect("test client");

    let subscriptions = Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default())),
        event_source,
        Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone())),
        Arc::new(kruxiaflow_core::cache::NoOpCache::new()),
        subscriptions.clone(),
        CancellationToken::new(),
    );
    let server = TestServer::new(app_router(state)).expect("test server");

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let token = response.json::<Value>()["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    Api {
        server,
        token,
        subscriptions,
        pool,
    }
}

impl Api {
    /// A running workflow whose `approve` step waits for `go`;
    /// `timeout_seconds: 0` is already past its deadline
    async fn waiting_workflow(&self, timeout_seconds: u64) -> Uuid {
        let def_id: Uuid = sqlx::query_scalar(
            "INSERT INTO workflow_definitions (name, activities) \
             VALUES ('signal_api_test', '[]'::jsonb) RETURNING id",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap();
        let workflow_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data) \
             VALUES ($1, 'signal_api_test', $2, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)",
        )
        .bind(workflow_id)
        .bind(def_id)
        .execute(&self.pool)
        .await
        .unwrap();
        self.subscriptions
            .create_subscription(NewSubscription {
                workflow_id,
                activity_key: "approve".to_string(),
                event_name: "go".to_string(),
                on_timeout: OnTimeout::Fail,
                timeout_seconds,
            })
            .await
            .unwrap();
        workflow_id
    }

    /// Send a signal; `None` leaves `data` out of the request body entirely
    async fn signal(&self, workflow_id: Uuid, data: Option<Value>) -> Value {
        let (status, body) = self.signal_status(workflow_id, data).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn signal_status(&self, workflow_id: Uuid, data: Option<Value>) -> (StatusCode, Value) {
        let mut body = json!({"activity_key": "approve", "event_name": "go"});
        if let Some(data) = data {
            body["data"] = data;
        }
        let response = self
            .server
            .post(&format!("/api/v1/workflows/{workflow_id}/signal"))
            .add_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap(),
            )
            .json(&body)
            .await;
        (
            response.status_code(),
            response.json::<Option<Value>>().unwrap_or_default(),
        )
    }

    /// The subscription row: (signaled_at is set, signal_data, expired_at is set)
    async fn row(&self, workflow_id: Uuid) -> (bool, Option<Value>, bool) {
        sqlx::query_as(
            "SELECT signaled_at IS NOT NULL, signal_data, expired_at IS NOT NULL \
             FROM activity_event_subscriptions WHERE workflow_id = $1 AND activity_key = 'approve'",
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn signaled_events(&self, workflow_id: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_events \
             WHERE workflow_id = $1 AND event_type = 'ActivitySignaled'",
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

/// Two signals for one wait: the first is accepted, the second rejected, and
/// the row keeps the first signal's data
async fn duplicate_signal(pool: PgPool, first: Option<Value>, second: Option<Value>) {
    let api = start(pool).await;
    let id = api.waiting_workflow(600).await;

    let r1 = api.signal(id, first.clone()).await;
    let r2 = api.signal(id, second).await;

    assert_eq!(r1["signaled"], true, "r1={r1}");
    assert_eq!(
        r2["signaled"], false,
        "a second signal for a wait already signaled was accepted: r2={r2}"
    );
    assert_eq!(api.row(id).await, (true, first, false));
    assert_eq!(api.signaled_events(id).await, 1);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_without_data(pool: PgPool) {
    duplicate_signal(pool, None, None).await;
}

/// The second signal's data would be lost: it must be rejected, not
/// acknowledged
#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_without_then_with_data(pool: PgPool) {
    duplicate_signal(pool, None, Some(json!({"approved": false}))).await;
}

/// Both signals carry data
#[sqlx::test(migrations = "../migrations")]
async fn test_duplicate_signal_with_data(pool: PgPool) {
    duplicate_signal(pool, Some(json!({"n": 1})), Some(json!({"n": 2}))).await;
}

/// Concurrent signals without data: exactly one is accepted
#[sqlx::test(migrations = "../migrations")]
async fn test_concurrent_signals_without_data(pool: PgPool) {
    let api = start(pool).await;
    let id = api.waiting_workflow(600).await;

    let responses = futures::future::join_all((0..4).map(|_| api.signal(id, None))).await;
    let accepted = responses.iter().filter(|r| r["signaled"] == true).count();
    assert_eq!(accepted, 1, "{responses:?}");
    assert_eq!(api.signaled_events(id).await, 1);
}

/// A wait signaled before its deadline is not expired by the checker while
/// the orchestrator has yet to take the signal
async fn signaled_wait_not_expired(pool: PgPool, data: Option<Value>) {
    let api = start(pool).await;
    let id = api.waiting_workflow(0).await;

    let r = api.signal(id, data.clone()).await;
    assert_eq!(r["signaled"], true, "{r}");
    let expired = api.subscriptions.expire_subscriptions(100).await.unwrap();
    assert!(
        expired.is_empty(),
        "a wait signaled before its deadline was expired: {expired:?}"
    );
    assert_eq!(api.row(id).await, (true, data, false));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signaled_wait_not_expired_without_data(pool: PgPool) {
    signaled_wait_not_expired(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signaled_wait_not_expired_with_data(pool: PgPool) {
    signaled_wait_not_expired(pool, Some(json!({"n": 1}))).await;
}

/// A signal for a wait the expiry checker has already expired is rejected
async fn signal_after_expiry(pool: PgPool, data: Option<Value>) {
    let api = start(pool).await;
    let id = api.waiting_workflow(0).await;
    let expired = api.subscriptions.expire_subscriptions(100).await.unwrap();
    assert_eq!(expired.len(), 1);

    let r = api.signal(id, data).await;
    assert_eq!(r["signaled"], false, "signal accepted after expiry: {r}");
    assert_eq!(api.row(id).await, (false, None, true));
    assert_eq!(api.signaled_events(id).await, 0);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_after_expiry_without_data(pool: PgPool) {
    signal_after_expiry(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_after_expiry_with_data(pool: PgPool) {
    signal_after_expiry(pool, Some(json!({"n": 1}))).await;
}

/// A signal whose event fails to publish gets an error and leaves the wait
/// open: a retry is accepted and publishes the event
async fn failed_publish_then_retry(pool: PgPool, data: Option<Value>) {
    let api = start_with(
        pool.clone(),
        Arc::new(FailFirstPublish {
            inner: PostgresEventSource::new(pool),
            failed: AtomicBool::new(false),
        }),
    )
    .await;
    let id = api.waiting_workflow(600).await;

    let (status, body) = api.signal_status(id, data.clone()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(api.row(id).await, (false, None, false));

    let r = api.signal(id, data.clone()).await;
    assert_eq!(r["signaled"], true, "retry after a failed publish: {r}");
    assert_eq!(api.row(id).await, (true, data, false));
    assert_eq!(api.signaled_events(id).await, 1);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_failed_publish_then_retry_without_data(pool: PgPool) {
    failed_publish_then_retry(pool, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn test_failed_publish_then_retry_with_data(pool: PgPool) {
    failed_publish_then_retry(pool, Some(json!({"n": 1}))).await;
}

/// A wait whose signal failed to publish still times out
#[sqlx::test(migrations = "../migrations")]
async fn test_failed_publish_wait_still_expires(pool: PgPool) {
    let api = start_with(
        pool.clone(),
        Arc::new(FailFirstPublish {
            inner: PostgresEventSource::new(pool),
            failed: AtomicBool::new(false),
        }),
    )
    .await;
    let id = api.waiting_workflow(0).await;

    let (status, body) = api.signal_status(id, Some(json!({"n": 1}))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let expired = api.subscriptions.expire_subscriptions(100).await.unwrap();
    assert_eq!(
        expired.len(),
        1,
        "a wait whose signal failed was not expired"
    );
}
