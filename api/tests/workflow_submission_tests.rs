//! Integration tests for workflow submission API
//!
//! Tests workflow submission, idempotency, validation, and error handling.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use bcrypt::hash;
use kruxiaflow_api::handlers::workflows::SubmitWorkflowResponse;
use kruxiaflow_api::{AppState, AppStateBuild, app_router};
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Load test RSA keys
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to create test AppState
async fn setup_test_state() -> AppState {
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

    // Create test client for client_credentials flow
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "test-client",
        hash("test-secret", bcrypt::DEFAULT_COST).unwrap(),
        "Test Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test client");

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        cache_service,
        CancellationToken::new(),
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-10-30T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec!["workflows".to_string(), "testing".to_string()],
    )
}

/// Helper to create test server
async fn setup_test_server() -> TestServer {
    let state = setup_test_state().await;
    let app = app_router(state);
    TestServer::new(app).expect("Failed to create test server")
}

/// Helper to get OAuth token
async fn get_test_token(server: &TestServer) -> String {
    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    body["access_token"].as_str().unwrap().to_string()
}

/// Helper to deploy a test workflow definition
async fn deploy_test_workflow(server: &TestServer, token: &str, name: &str) -> String {
    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "name": name,
            "activities": [
                {
                    "key": "step1",
                    "worker": "test",
                    "name": "echo",
                    "parameters": {}
                }
            ]
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    body["version"].as_str().unwrap().to_string()
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_success() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy workflow definition first
    let def_name = "test_submit_workflow";
    deploy_test_workflow(&server, &token, def_name).await;

    // Submit workflow
    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: SubmitWorkflowResponse = response.json();
    assert_eq!(body.definition_name, def_name);
    assert!(!body.definition_version.is_empty());
    assert_eq!(body.status, "created");
    assert!(!body.workflow_id.to_string().is_empty());
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_with_specific_version() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    let def_name = "test_submit_workflow_versioned";
    let version = deploy_test_workflow(&server, &token, def_name).await;

    // Submit with specific version
    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "version": version,
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: SubmitWorkflowResponse = response.json();
    assert_eq!(body.definition_version, version);
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_definition_not_found() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": "nonexistent",
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);

    let body: serde_json::Value = response.json();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_invalid_input() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    let def_name = "test_invalid_input";
    deploy_test_workflow(&server, &token, def_name).await;

    // Submit with array instead of object
    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "input": ["invalid", "array"]
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_idempotency() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    let def_name = "test_idempotency";
    deploy_test_workflow(&server, &token, def_name).await;

    let unique_key = format!("test_unique_key_{}", uuid::Uuid::now_v7());

    // First submission
    let response1 = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"},
            "unique_key": unique_key
        }))
        .await;

    assert_eq!(response1.status_code(), StatusCode::CREATED);
    let _body1: SubmitWorkflowResponse = response1.json();

    // Second submission with same unique_key
    let response2 = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"},
            "unique_key": unique_key
        }))
        .await;

    assert_eq!(response2.status_code(), StatusCode::CONFLICT);

    let body2: serde_json::Value = response2.json();
    assert_eq!(body2["error"]["code"], "CONFLICT");
    assert!(
        body2["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&unique_key)
    );
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_requires_authentication() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/workflows")
        .json(&json!({
            "definition_name": "test",
            "input": {}
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_empty_definition_name() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": "",
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[serial]
async fn test_submit_workflow_event_published() {
    let server = setup_test_server().await;
    let state = setup_test_state().await;
    let token = get_test_token(&server).await;

    let def_name = "test_event_published";
    deploy_test_workflow(&server, &token, def_name).await;

    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
    let body: SubmitWorkflowResponse = response.json();

    // Verify event was published
    let event_count = sqlx::query!(
        r#"
        SELECT COUNT(*) as count
        FROM workflow_events
        WHERE workflow_id = $1 AND event_type = 'WorkflowCreated'
        "#,
        body.workflow_id
    )
    .fetch_one(&state.db_pool)
    .await
    .expect("Should be able to query events");

    assert!(
        event_count.count.unwrap_or(0) > 0,
        "WorkflowCreated event should exist"
    );
}
