// api/tests/shutdown_middleware_tests.rs
//! Integration tests for shutdown middleware
//!
//! Tests that the shutdown middleware properly rejects requests during shutdown.

use axum::Router;
use axum::http::StatusCode;
use axum::middleware as axum_middleware;
use axum::routing::get;
use axum_test::TestServer;
use kruxiaflow_api::{AppState, AppStateBuild, middleware::shutdown::shutdown_check};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Load test RSA keys
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to create test AppState
async fn setup_test_state(pool: PgPool, shutdown_token: CancellationToken) -> AppState {
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
async fn setup_test_server(pool: PgPool, shutdown_token: CancellationToken) -> TestServer {
    let state = setup_test_state(pool, shutdown_token).await;

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

#[sqlx::test(migrations = "../migrations")]
async fn test_shutdown_middleware_allows_requests_when_not_shutting_down(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(pool, shutdown_token.clone()).await;

    // Send a request - should succeed
    let response = server.get("/test").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(response.text(), "OK");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_shutdown_middleware_rejects_requests_during_shutdown(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(pool, shutdown_token.clone()).await;

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

#[sqlx::test(migrations = "../migrations")]
async fn test_shutdown_middleware_transition(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(pool, shutdown_token.clone()).await;

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

#[sqlx::test(migrations = "../migrations")]
async fn test_shutdown_middleware_response_format(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(pool, shutdown_token.clone()).await;

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

#[sqlx::test(migrations = "../migrations")]
async fn test_shutdown_middleware_multiple_concurrent_requests(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let server = setup_test_server(pool, shutdown_token.clone()).await;

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
