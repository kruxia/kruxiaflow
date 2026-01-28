use kruxiaflow_api::{routes::app_router, state::AppState};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{Activity, ActivityQueue as _, PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_worker::{ActivityRegistry, EchoActivity, WorkerConfig, WorkerManager};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
    });

    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database")
}

/// Generate test RSA private key
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

/// Generate test RSA public key
fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Create and start a real API server on a port
async fn create_real_server() -> (String, PgPool, tokio::task::JoinHandle<()>) {
    let pool = setup_test_pool().await;

    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };

    let auth_service = PostgresAuthService::new(pool.clone(), auth_config)
        .expect("Failed to create test auth service");

    // Create test OAuth client
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "test_worker_client",
        bcrypt::hash("test_worker_secret", bcrypt::DEFAULT_COST).unwrap(),
        "Test Worker Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test OAuth client");

    let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let shutdown_token = CancellationToken::new();

    let subscription_service = Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        activity_queue,
        event_source,
        workflow_storage.clone(),
        cache_service,
        subscription_service,
        shutdown_token,
    );
    let app = app_router(state);

    // Bind to a random available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to port");
    let addr = listener.local_addr().expect("Failed to get local address");
    let server_url = format!("http://{}", addr);

    // Spawn the server in the background
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Server failed to start");
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    (server_url, pool, handle)
}

/// Helper to schedule test activities
async fn schedule_test_activities(pool: &PgPool, workflow_id: Uuid, count: usize) {
    let queue = PostgresQueue::new(pool.clone(), QueueConfig::default());
    let activities: Vec<Activity> = (0..count)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            worker: "builtin".to_string(),
            activity_name: "echo".to_string(),
            parameters: json!({"test": format!("value_{}", i)}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");
}

#[tokio::test]
#[serial]
async fn test_worker_poll_and_execute_echo() {
    // Setup: Start real API server
    let (server_url, pool, server_handle) = create_real_server().await;
    let workflow_id = Uuid::now_v7();

    // Schedule echo activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    // Verify activity was scheduled
    let count_after_schedule: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_one(&pool)
            .await
            .expect("Should count activities");
    println!("Activities scheduled: {}", count_after_schedule.0);

    // Configure worker
    #[allow(deprecated)]
    let config = WorkerConfig {
        api_url: server_url,
        worker_id: "test_worker".to_string(),
        worker: "builtin".to_string(),
        poll_max_activities: 10,
        poll_interval: Duration::from_millis(100),
        max_concurrent_activities: 16,
        concurrency: 1,
        activity_timeout: Duration::from_secs(30),
        heartbeat_interval: Duration::from_secs(30),
        client_id: "test_worker_client".to_string(),
        client_secret: "test_worker_secret".to_string(),
    };

    // Create worker with EchoActivity
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let mut registry = ActivityRegistry::new(cache_service);
    registry.register(Arc::new(EchoActivity));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let manager = WorkerManager::new(config, registry, workflow_storage);

    // Start worker
    let handles = manager.start().await.expect("Worker should start");

    // Wait for activity to complete
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Check if completion event was published to workflow_events
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workflow_events WHERE workflow_id = $1 AND event_type = 'ActivityCompleted'"
    )
    .bind(workflow_id)
    .fetch_one(&pool)
    .await
    .expect("Should count events");

    println!("ActivityCompleted events: {}", event_count.0);

    // Check activity_queue state
    let queue_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_one(&pool)
            .await
            .expect("Should count activities");

    println!("Activities in queue: {}", queue_count.0);

    // The worker should have completed the activity and published an event
    assert!(
        event_count.0 >= 1,
        "At least one ActivityCompleted event should have been published"
    );

    // Stop worker and server
    manager.stop(handles).await;
    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_worker_concurrency() {
    // Setup: Start real API server
    let (server_url, pool, server_handle) = create_real_server().await;
    let workflow_id = Uuid::now_v7();

    // Schedule 10 echo activities
    schedule_test_activities(&pool, workflow_id, 10).await;

    // Configure worker with max_concurrent_activities=16
    #[allow(deprecated)]
    let config = WorkerConfig {
        api_url: server_url,
        worker_id: "test_worker_concurrent".to_string(),
        worker: "builtin".to_string(),
        poll_max_activities: 10,
        poll_interval: Duration::from_millis(100),
        max_concurrent_activities: 16,
        concurrency: 4,
        activity_timeout: Duration::from_secs(30),
        heartbeat_interval: Duration::from_secs(30),
        client_id: "test_worker_client".to_string(),
        client_secret: "test_worker_secret".to_string(),
    };

    // Create worker with EchoActivity
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let mut registry = ActivityRegistry::new(cache_service);
    registry.register(Arc::new(EchoActivity));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let manager = WorkerManager::new(config, registry, workflow_storage);

    // Start worker
    let handles = manager.start().await.expect("Worker should start");

    // Wait for all activities to complete
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Verify all activities completed by checking workflow_events
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM workflow_events WHERE workflow_id = $1 AND event_type = 'ActivityCompleted'"
    )
    .bind(workflow_id)
    .fetch_one(&pool)
    .await
    .expect("Should count events");

    println!("ActivityCompleted events: {}", event_count.0);

    assert_eq!(
        event_count.0, 10,
        "All 10 activities should have been completed"
    );

    // Stop worker and server
    manager.stop(handles).await;
    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_worker_authentication_failure() {
    // Setup: Start real API server
    let (server_url, pool, server_handle) = create_real_server().await;

    // Configure worker with invalid credentials
    #[allow(deprecated)]
    let config = WorkerConfig {
        api_url: server_url,
        worker_id: "test_worker_badauth".to_string(),
        worker: "default".to_string(),
        poll_max_activities: 10,
        poll_interval: Duration::from_millis(100),
        max_concurrent_activities: 16,
        concurrency: 1,
        activity_timeout: Duration::from_secs(30),
        heartbeat_interval: Duration::from_secs(30),
        client_id: "test_worker_client".to_string(),
        client_secret: "wrong_secret".to_string(),
    };

    // Create worker with EchoActivity
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let mut registry = ActivityRegistry::new(cache_service);
    registry.register(Arc::new(EchoActivity));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let manager = WorkerManager::new(config, registry, workflow_storage);

    // Start worker - it should fail to authenticate but not crash
    let handles = manager.start().await.expect("Worker should start");

    // Wait briefly - worker will error but keep retrying
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Worker should still be running (retrying with backoff)
    // Just verify we can stop it without panic
    manager.stop(handles).await;
    server_handle.abort();
}
