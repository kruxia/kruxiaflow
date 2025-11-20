// api/tests/shutdown_middleware_tests.rs
//! Integration tests for shutdown middleware
//!
//! Tests that the shutdown middleware properly rejects requests during shutdown.

use axum::Router;
use axum::http::StatusCode;
use axum::middleware as axum_middleware;
use axum::routing::get;
use axum_test::TestServer;
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_api::{AppState, AppStateBuild, middleware::shutdown::shutdown_check};
use streamflow_core::events::PostgresEventSource;
use streamflow_core::queue::{PostgresQueue, QueueConfig};
use streamflow_oauth::{AuthConfig, PostgresAuthService};
use tokio_util::sync::CancellationToken;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
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
async fn setup_test_state(shutdown_token: CancellationToken) -> AppState {
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

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(streamflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(streamflow_core::cache::NoOpCache::new());

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        cache_service,
        shutdown_token,
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-10-30T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec!["workflows".to_string()],
    )
}

/// Helper to create test server with shutdown middleware
async fn setup_test_server(shutdown_token: CancellationToken) -> TestServer {
    let state = setup_test_state(shutdown_token).await;

    // Create a simple test endpoint
    let app = Router::new()
        .route("/test", get(|| async { "OK" }))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            shutdown_check,
        ))
        .with_state(state);

    TestServer::new(app).expect("Failed to create test server")
}

#[tokio::test]
#[serial]
async fn test_shutdown_middleware_allows_requests_when_not_shutting_down() {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(shutdown_token.clone()).await;

    // Send a request - should succeed
    let response = server.get("/test").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(response.text(), "OK");
}

#[tokio::test]
#[serial]
async fn test_shutdown_middleware_rejects_requests_during_shutdown() {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(shutdown_token.clone()).await;

    // Trigger shutdown
    shutdown_token.cancel();

    // Send a request - should be rejected with 503
    let response = server.get("/test").await;

    assert_eq!(response.status_code(), StatusCode::SERVICE_UNAVAILABLE);

    let body: serde_json::Value = response.json();
    assert_eq!(
        body,
        json!({
            "error": {
                "code": "service_unavailable",
                "message": "Server is shutting down, please retry later"
            }
        })
    );
}

#[tokio::test]
#[serial]
async fn test_shutdown_middleware_transition() {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(shutdown_token.clone()).await;

    // First request should succeed
    let response1 = server.get("/test").await;
    assert_eq!(response1.status_code(), StatusCode::OK);

    // Trigger shutdown
    shutdown_token.cancel();

    // Second request should be rejected
    let response2 = server.get("/test").await;
    assert_eq!(response2.status_code(), StatusCode::SERVICE_UNAVAILABLE);

    // Third request should still be rejected
    let response3 = server.get("/test").await;
    assert_eq!(response3.status_code(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
#[serial]
async fn test_shutdown_middleware_response_format() {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(shutdown_token.clone()).await;

    // Trigger shutdown
    shutdown_token.cancel();

    // Send a request
    let response = server.get("/test").await;

    // Verify response format
    assert_eq!(response.status_code(), StatusCode::SERVICE_UNAVAILABLE);

    let body: serde_json::Value = response.json();

    // Verify structure
    assert!(body.get("error").is_some());
    assert!(body["error"].get("code").is_some());
    assert!(body["error"].get("message").is_some());

    // Verify values
    assert_eq!(body["error"]["code"], "service_unavailable");
    assert_eq!(
        body["error"]["message"],
        "Server is shutting down, please retry later"
    );
}

#[tokio::test]
#[serial]
async fn test_shutdown_middleware_multiple_concurrent_requests() {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(shutdown_token.clone()).await;

    // Trigger shutdown
    shutdown_token.cancel();

    // Send multiple requests sequentially
    let response1 = server.get("/test").await;
    let response2 = server.get("/test").await;
    let response3 = server.get("/test").await;

    // All should be rejected
    for response in [response1, response2, response3] {
        assert_eq!(response.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = response.json();
        assert_eq!(body["error"]["code"], "service_unavailable");
    }
}
