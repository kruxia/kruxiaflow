use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_api::{routes::app_router, state::AppState};
use streamflow_core::events::PostgresEventSource;
use streamflow_core::queue::{Activity, ActivityQueue, PostgresQueue, QueueConfig};
use streamflow_oauth::{AuthConfig, PostgresAuthService};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
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

/// Create test server with authentication
async fn create_test_server() -> (TestServer, PgPool) {
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
        "test_client",
        bcrypt::hash("test_secret", bcrypt::DEFAULT_COST).unwrap(),
        "Test Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test OAuth client");

    let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let shutdown_token = CancellationToken::new();

    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        activity_queue,
        event_source,
        shutdown_token,
    );
    let app = app_router(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, pool)
}

/// Get test access token
async fn get_test_token(server: &TestServer) -> String {
    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test_client",
            "client_secret": "test_secret"
        }))
        .await;

    let body: serde_json::Value = response.json();
    body["access_token"].as_str().unwrap().to_string()
}

/// Helper to schedule test activities
async fn schedule_test_activities(pool: &PgPool, workflow_id: Uuid, count: usize) {
    let queue = PostgresQueue::new(pool.clone(), QueueConfig::default());
    let activities: Vec<Activity> = (0..count)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            namespace: "payments".to_string(),
            name: "authorize".to_string(),
            parameters: json!({"amount": 100.0}),
            settings: None,
            scheduled_for: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, workflow_id: Uuid) {
    sqlx::query!(
        "DELETE FROM activity_queue WHERE workflow_id = $1",
        workflow_id
    )
    .execute(pool)
    .await
    .ok();

    sqlx::query!(
        "DELETE FROM workflow_events WHERE workflow_id = $1",
        workflow_id
    )
    .execute(pool)
    .await
    .ok();
}

#[tokio::test]
#[serial]
async fn test_poll_activities_success() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule test activities
    schedule_test_activities(&pool, workflow_id, 3).await;

    // Poll for activities
    let response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["count"], 3);
    assert_eq!(body["activities"].as_array().unwrap().len(), 3);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_poll_activities_empty() {
    let (server, _pool) = create_test_server().await;
    let token = get_test_token(&server).await;

    // Poll for non-existent activity type
    let response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["nonexistent.type"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["count"], 0);
    assert!(body["activities"].as_array().unwrap().is_empty());
}

#[tokio::test]
#[serial]
async fn test_poll_activities_validation_error() {
    let (server, _pool) = create_test_server().await;
    let token = get_test_token(&server).await;

    // Invalid activity type format (missing namespace)
    let response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["invalid_format"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[serial]
async fn test_poll_activities_unauthorized() {
    let (server, _pool) = create_test_server().await;

    // No authentication token
    let response = server
        .post("/api/v1/workers/poll")
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn test_heartbeat_activity_success() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Send heartbeat
    let response = server
        .post(&format!("/api/v1/activities/{}/heartbeat", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["acknowledged"], true);
    assert_eq!(body["next_heartbeat_seconds"], 30);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_heartbeat_activity_not_found() {
    let (server, _pool) = create_test_server().await;
    let token = get_test_token(&server).await;

    let activity_id = Uuid::now_v7();

    let response = server
        .post(&format!("/api/v1/activities/{}/heartbeat", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[serial]
async fn test_heartbeat_wrong_worker() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim with worker_01
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Try heartbeat from worker_02
    let response = server
        .post(&format!("/api/v1/activities/{}/heartbeat", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_02"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::CONFLICT);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_activity_success() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Complete the activity
    let response = server
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": {"authorization_id": "auth_123", "approved": true},
            "cost_usd": 0.015
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["acknowledged"], true);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_activity_idempotency() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Complete first time
    let response1 = server
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": {"result": "success"}
        }))
        .await;

    assert_eq!(response1.status_code(), StatusCode::OK);

    // Try to complete again (should fail with conflict)
    let response2 = server
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": {"result": "success"}
        }))
        .await;

    assert_eq!(response2.status_code(), StatusCode::NOT_FOUND);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_activity_validation_error() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Invalid output (not an object)
    let response = server
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": "not an object"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_fail_activity_success() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Fail the activity
    let response = server
        .post(&format!("/api/v1/activities/{}/fail", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01",
            "error": {
                "code": "PAYMENT_DECLINED",
                "message": "Card was declined by the bank",
                "retryable": false
            }
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["acknowledged"], true);
    assert_eq!(body["will_retry"], false);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_fail_activity_validation_error() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activities(&pool, workflow_id, 1).await;

    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 1
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activity_id = poll_body["activities"][0]["activity_id"].as_str().unwrap();

    // Empty error code
    let response = server
        .post(&format!("/api/v1/activities/{}/fail", activity_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "worker_id": "worker_test_01",
            "error": {
                "code": "",
                "message": "Some error",
                "retryable": false
            }
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    cleanup_test_data(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_workflow_end_to_end() {
    let (server, pool) = create_test_server().await;
    let token = get_test_token(&server).await;
    let workflow_id = Uuid::now_v7();

    // Schedule multiple activities
    schedule_test_activities(&pool, workflow_id, 3).await;

    // Poll and claim all activities
    let poll_response = server
        .post("/api/v1/workers/poll")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    let poll_body: serde_json::Value = poll_response.json();
    let activities = poll_body["activities"].as_array().unwrap();

    // Complete all activities
    for activity in activities {
        let activity_id = activity["activity_id"].as_str().unwrap();

        let response = server
            .post(&format!("/api/v1/activities/{}/complete", activity_id))
            .add_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            )
            .json(&json!({
                "worker_id": "worker_test_01",
                "output": {"result": "success"}
            }))
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    // Verify all activities are removed from queue
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, Some(0));

    // Verify all events were published
    let event_count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM workflow_events WHERE workflow_id = $1",
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(event_count, Some(3));

    cleanup_test_data(&pool, workflow_id).await;
}
