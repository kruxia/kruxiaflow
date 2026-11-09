//! End to end: the built-in worker, a real API server and a real database,
//! with every timer set far longer than the test runs
//! (docs/implementation/2026-11-08-activity-wake-sse.md).
//!
//! The worker's poll interval and its wake-stream idle interval are both
//! 30 s. Work that is picked up within a few seconds can therefore only have
//! been announced: by the trigger, through the API server's listener and the
//! wake stream, or by the next-due hint on an empty poll.

use kruxiaflow_api::{routes::app_router, state::AppState};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::{EventWakeMode, PostgresEventSource};
use kruxiaflow_core::queue::{Activity, ActivityQueue as _, PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_std_worker::{ActivityRegistry, EchoActivity, WorkerConfig, WorkerManager};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const LONG_TIMER: Duration = Duration::from_secs(30);

fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// A real API server whose state carries the activity wake of a real LISTEN
/// connection
async fn start_server_with_wake(pool: PgPool) -> (String, tokio::task::JoinHandle<()>) {
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
    .bind("wake_e2e_client")
    .bind(bcrypt::hash("wake_e2e_secret", 4).unwrap()) // min cost: hashing strength is not under test
    .bind("Wake E2E Client")
    .execute(&pool)
    .await
    .expect("Failed to create test OAuth client");

    let event_source = PostgresEventSource::connect(pool.clone(), EventWakeMode::Notify)
        .await
        .expect("event source");
    let activity_wake = event_source.activity_wake();
    assert!(activity_wake.is_some(), "LISTEN should succeed in tests");

    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default())),
        Arc::new(event_source),
        Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone())),
        Arc::new(kruxiaflow_core::cache::NoOpCache::new()),
        Arc::new(PostgresSubscriptionService::new(pool)),
        CancellationToken::new(),
    )
    .with_activity_wake(activity_wake);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to port");
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app_router(state))
            .await
            .expect("Server failed to start");
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    (server_url, handle)
}

fn echo(key: &str, scheduled_for: Option<chrono::DateTime<chrono::Utc>>) -> Activity {
    Activity {
        key: key.to_string(),
        worker: "std".to_string(),
        activity_name: "echo".to_string(),
        parameters: json!({"message": key}),
        settings: None,
        scheduled_for,
        output_definitions: None,
        iteration: None,
        signal_data: None,
    }
}

/// How long until the workflow's activity is reported complete, or `None`
/// if that does not happen within `within`
async fn time_to_completion(
    pool: &PgPool,
    workflow_id: Uuid,
    within: Duration,
) -> Option<Duration> {
    let started = Instant::now();
    while started.elapsed() < within {
        let completed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_events \
             WHERE workflow_id = $1 AND event_type = 'ActivityCompleted'",
        )
        .bind(workflow_id)
        .fetch_one(pool)
        .await
        .expect("count events");
        if completed > 0 {
            return Some(started.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}

#[sqlx::test(migrations = "../migrations")]
async fn test_worker_is_woken_for_new_work_and_returns_on_time_for_delayed_work(pool: PgPool) {
    let (server_url, server_handle) = start_server_with_wake(pool.clone()).await;
    let queue = PostgresQueue::new(pool.clone(), QueueConfig::default());

    let config = WorkerConfig {
        api_url: server_url,
        worker_id: "wake_e2e_worker".to_string(),
        worker: "std".to_string(),
        // Neither timer can explain work being picked up within seconds
        poll_interval: LONG_TIMER,
        wake_poll_interval: LONG_TIMER,
        client_id: Some("wake_e2e_client".to_string()),
        client_secret: Some("wake_e2e_secret".to_string()),
        ..WorkerConfig::default()
    };
    let mut registry = ActivityRegistry::new(Arc::new(kruxiaflow_core::cache::NoOpCache::new()));
    registry.register(Arc::new(EchoActivity));
    let manager = WorkerManager::new(
        config,
        registry,
        Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone())),
    );
    let handles = manager.start().await.expect("worker should start");

    // Let the worker authenticate, poll once and open its stream; from here
    // on it is waiting up to 30 s
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // 1. Work scheduled for now is announced and picked up at once
    let workflow_now = Uuid::now_v7();
    queue
        .schedule(workflow_now, vec![echo("now", None)])
        .await
        .unwrap();
    let took = time_to_completion(&pool, workflow_now, Duration::from_secs(10))
        .await
        .expect("work scheduled for now was not picked up; the wake did not reach the worker");
    assert!(took < Duration::from_secs(5), "took {took:?}");

    // 2. Work scheduled for 2 s from now (by the database's clock) starts
    //    when it is due: the wake makes the worker poll, the empty answer
    //    says when to come back
    let due: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT now() + interval '2 seconds'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let workflow_later = Uuid::now_v7();
    queue
        .schedule(workflow_later, vec![echo("later", Some(due))])
        .await
        .unwrap();
    let took = time_to_completion(&pool, workflow_later, Duration::from_secs(15))
        .await
        .expect("delayed work was not picked up when due; the next-due hint did not work");
    assert!(
        took >= Duration::from_millis(1800),
        "ran {took:?} after scheduling, before it was due"
    );
    assert!(took < Duration::from_secs(8), "took {took:?}");

    manager.stop(handles).await;
    server_handle.abort();
}
