// api/tests/oauth_handler_tests.rs
//! Integration tests for OAuth 2.0 token handlers
//!
//! Tests OAuth 2.0 compliant token endpoint per RFC 6749.

use axum::http::StatusCode;
use axum_test::TestServer;
use bcrypt::hash;
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_api::{AppState, AppStateBuild, app_router};
use streamflow_oauth::{AuthConfig, PostgresAuthService};

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
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

    // Create test user for password flow
    sqlx::query!(
        "INSERT INTO oauth_users (username, email, password_hash, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (username) DO NOTHING",
        "testuser",
        "testuser@example.com",
        hash("testpass", bcrypt::DEFAULT_COST).unwrap()
    )
    .execute(&pool)
    .await
    .expect("Failed to create test user");

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
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

// ============================================================================
// JsonOrForm Extractor Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_token_endpoint_accepts_json() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .content_type("application/json")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
}

#[tokio::test]
#[serial]
async fn test_token_endpoint_accepts_form_urlencoded() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .content_type("application/x-www-form-urlencoded")
        .form(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
}

#[tokio::test]
#[serial]
async fn test_token_endpoint_rejects_unsupported_content_type() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .content_type("text/plain")
        .text("grant_type=client_credentials")
        .await;

    assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let body = response.text();
    assert!(
        body.contains("Content-Type must be application/json or application/x-www-form-urlencoded")
    );
}

#[tokio::test]
#[serial]
async fn test_token_endpoint_rejects_invalid_json() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .add_header("content-type", "application/json")
        .bytes("{invalid json}".into())
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body = response.text();
    assert!(body.contains("Invalid JSON"));
}

// ============================================================================
// Client Credentials Grant Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_client_credentials_grant_success() {
    let server = setup_test_server().await;

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
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    assert!(body["refresh_token"].is_null());
}

#[tokio::test]
#[serial]
async fn test_client_credentials_missing_client_id() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body = response.text();
    assert!(body.contains("client_id is required"));
}

#[tokio::test]
#[serial]
async fn test_client_credentials_missing_client_secret() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body = response.text();
    assert!(body.contains("client_secret is required"));
}

#[tokio::test]
#[serial]
async fn test_client_credentials_invalid_credentials() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "wrong-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);

    let body = response.text();
    assert!(body.contains("Invalid client credentials"));
}

#[tokio::test]
#[serial]
async fn test_client_credentials_nonexistent_client() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "nonexistent-client",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Password Grant Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_password_grant_success() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": "testuser",
            "password": "testpass"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    assert!(body["refresh_token"].is_string());
}

#[tokio::test]
#[serial]
async fn test_password_grant_missing_username() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "password": "testpass"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body = response.text();
    assert!(body.contains("username is required"));
}

#[tokio::test]
#[serial]
async fn test_password_grant_missing_password() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": "testuser"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body = response.text();
    assert!(body.contains("password is required"));
}

#[tokio::test]
#[serial]
async fn test_password_grant_invalid_password() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": "testuser",
            "password": "wrongpass"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);

    let body = response.text();
    assert!(body.contains("Invalid username or password"));
}

#[tokio::test]
#[serial]
async fn test_password_grant_nonexistent_user() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": "nonexistent",
            "password": "testpass"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Refresh Token Grant Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_refresh_token_grant_success() {
    let server = setup_test_server().await;

    // First, get a refresh token using password grant
    let initial_response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "password",
            "username": "testuser",
            "password": "testpass"
        }))
        .await;

    assert_eq!(initial_response.status_code(), StatusCode::OK);
    let initial_body: serde_json::Value = initial_response.json();
    let refresh_token = initial_body["refresh_token"]
        .as_str()
        .expect("refresh_token should be present");

    // Use refresh token to get a new access token
    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    assert!(body["refresh_token"].is_string());
}

#[tokio::test]
#[serial]
async fn test_refresh_token_grant_missing_refresh_token() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "refresh_token"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body = response.text();
    assert!(body.contains("refresh_token is required"));
}

#[tokio::test]
#[serial]
async fn test_refresh_token_grant_invalid_token() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "refresh_token",
            "refresh_token": "invalid-refresh-token"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);

    let body = response.text();
    assert!(body.contains("Invalid or expired refresh token"));
}

// ============================================================================
// Token Validation Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_issued_token_is_valid_jwt() {
    let server = setup_test_server().await;

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
    let token = body["access_token"].as_str().unwrap();

    // Token should be a valid JWT (3 base64url parts separated by dots)
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");
}

#[tokio::test]
#[serial]
async fn test_token_response_structure() {
    let server = setup_test_server().await;

    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    let body: serde_json::Value = response.json();

    // Verify all required fields are present
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert!(body["expires_in"].is_number());

    // client_credentials should not return refresh_token
    assert!(body["refresh_token"].is_null());

    // scope is optional and should be null in MVP
    assert!(body["scope"].is_null() || !body.get("scope").is_some());
}

// ============================================================================
// Multiple Requests Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multiple_token_requests_generate_different_tokens() {
    let server = setup_test_server().await;

    let response1 = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    let body1: serde_json::Value = response1.json();
    let token1 = body1["access_token"].as_str().unwrap();

    let response2 = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    let body2: serde_json::Value = response2.json();
    let token2 = body2["access_token"].as_str().unwrap();

    // Each token request should generate a unique token (different iat)
    assert_ne!(token1, token2);
}

#[tokio::test]
#[serial]
async fn test_concurrent_token_requests() {
    let server = setup_test_server().await;

    // Issue multiple token requests (sequentially in TestServer)
    let mut tokens = Vec::new();
    for _ in 0..5 {
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
        tokens.push(body["access_token"].as_str().unwrap().to_string());
    }

    // All tokens should be valid but unique
    assert_eq!(tokens.len(), 5);
    for i in 0..tokens.len() {
        for j in (i + 1)..tokens.len() {
            assert_ne!(tokens[i], tokens[j]);
        }
    }
}
