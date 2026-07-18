// api/tests/user_creation_tests.rs
//! Integration tests for user creation endpoint (POST /api/v1/oauth/users)
//!
//! Tests the full HTTP stack including auth middleware.

use axum::http::StatusCode;
use axum_test::TestServer;
use bcrypt::hash;
use kruxiaflow_api::{AppState, AppStateBuild, app_router};
use kruxiaflow_core::PostgresSubscriptionService;
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

    // Run migrations
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Load test RSA private key
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

/// Load test RSA public key
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

    // Create test client for obtaining auth tokens
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "test-client-users",
        hash("test-secret", bcrypt::DEFAULT_COST).unwrap(),
        "Test Client for User Tests"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test client");

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());

    let subscription_service = Arc::new(PostgresSubscriptionService::new(pool.clone()));
    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        cache_service,
        subscription_service,
        CancellationToken::new(),
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-10-30T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec!["workflows".to_string()],
    )
}

/// Helper to create test server
async fn setup_test_server() -> TestServer {
    let state = setup_test_state().await;
    let router = app_router(state);
    TestServer::new(router).expect("Failed to create test server")
}

/// Helper to get a valid auth token via client_credentials
async fn get_auth_token(server: &TestServer) -> String {
    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client-users",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    body["access_token"].as_str().unwrap().to_string()
}

/// Cleanup test user
async fn cleanup_test_user(username: &str) {
    let pool = setup_test_pool().await;
    sqlx::query!("DELETE FROM oauth_refresh_tokens WHERE user_id IN (SELECT id FROM oauth_users WHERE username = $1)", username)
        .execute(&pool)
        .await
        .ok();
    sqlx::query!("DELETE FROM oauth_users WHERE username = $1", username)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Authentication requirement tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_user_requires_auth() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/users")
        .json(&json!({
            "username": "noauth-user",
            "email": "noauth@example.com",
            "password": "password"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Successful creation tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_user_success() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;
    let username = "test-create-user-success";

    // Pre-cleanup
    cleanup_test_user(username).await;

    let response = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": username,
            "email": "created@example.com",
            "password": "secure-pass"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    assert_eq!(body["username"], username);
    assert_eq!(body["email"], "created@example.com");
    assert_eq!(body["is_active"], true);
    assert!(body["id"].is_string());
    assert!(body["created_at"].is_string());

    cleanup_test_user(username).await;
}

#[tokio::test]
#[serial]
async fn test_created_user_can_login() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;
    let username = "test-created-can-login";

    // Pre-cleanup
    cleanup_test_user(username).await;

    // Create user via API
    let create_response = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": username,
            "email": "canlogin@example.com",
            "password": "login-password"
        }))
        .await;

    assert_eq!(create_response.status_code(), StatusCode::CREATED);

    // Login with the created user via password grant
    let login_response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": username,
            "password": "login-password"
        }))
        .await;

    assert_eq!(login_response.status_code(), StatusCode::OK);

    let login_body: serde_json::Value = login_response.json();
    assert!(login_body["access_token"].is_string());
    assert_eq!(login_body["token_type"], "Bearer");
    assert!(login_body["refresh_token"].is_string());

    cleanup_test_user(username).await;
}

// ============================================================================
// Idempotent creation tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_user_idempotent() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;
    let username = "test-idempotent-user";

    // Pre-cleanup
    cleanup_test_user(username).await;

    // First creation
    let response1 = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": username,
            "email": "idem@example.com",
            "password": "password-1"
        }))
        .await;

    assert_eq!(response1.status_code(), StatusCode::CREATED);
    let body1: serde_json::Value = response1.json();

    // Second creation with same username
    let response2 = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": username,
            "email": "different@example.com",
            "password": "password-2"
        }))
        .await;

    assert_eq!(response2.status_code(), StatusCode::CREATED);
    let body2: serde_json::Value = response2.json();

    // Same user ID returned
    assert_eq!(body1["id"], body2["id"]);

    // Original password should still work (not overwritten)
    let login_response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": username,
            "password": "password-1"
        }))
        .await;

    assert_eq!(login_response.status_code(), StatusCode::OK);

    cleanup_test_user(username).await;
}

// ============================================================================
// Validation error tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_user_empty_username() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;

    let response = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": "",
            "email": "empty@example.com",
            "password": "password"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let body = response.text();
    assert!(body.contains("username is required"));
}

#[tokio::test]
#[serial]
async fn test_create_user_empty_email() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;

    let response = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": "validuser",
            "email": "",
            "password": "password"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let body = response.text();
    assert!(body.contains("email is required"));
}

#[tokio::test]
#[serial]
async fn test_create_user_empty_password() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;

    let response = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": "validuser",
            "email": "valid@example.com",
            "password": ""
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let body = response.text();
    assert!(body.contains("password is required"));
}

#[tokio::test]
#[serial]
async fn test_create_user_whitespace_username() {
    let server = setup_test_server().await;
    let token = get_auth_token(&server).await;

    let response = server
        .post("/api/v1/oauth/users")
        .add_header("authorization", format!("Bearer {}", token))
        .json(&json!({
            "username": "   ",
            "email": "ws@example.com",
            "password": "password"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let body = response.text();
    assert!(body.contains("username is required"));
}
