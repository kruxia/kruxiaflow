use axum::http::StatusCode;
use axum_test::TestServer;
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

    // Run migrations from workspace root
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Generate test RSA private key
fn test_rsa_private_key() -> String {
    // Test RSA private key (2048-bit) - for testing only!
    include_str!("../../oauth/tests/private.pem").to_string()
}

/// Generate test RSA public key
fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to create test AppState
async fn setup_test_state() -> AppState {
    let pool = setup_test_pool().await;

    // Create auth service for testing
    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };

    let auth_service = PostgresAuthService::new(pool.clone(), auth_config)
        .expect("Failed to create test auth service");

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-10-30T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec![
            "workflows".to_string(),
            "workers".to_string(),
            "websockets".to_string(),
        ],
    )
}

/// Helper to create test server
async fn setup_test_server() -> TestServer {
    let state = setup_test_state().await;
    let router = app_router(state);
    TestServer::new(router).expect("Failed to create test server")
}

#[tokio::test]
#[serial]
async fn test_liveness_endpoint() {
    let server = setup_test_server().await;

    let response = server.get("/health").await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
#[serial]
async fn test_liveness_endpoint_multiple_calls() {
    let server = setup_test_server().await;

    // Simulate repeated health checks (like Kubernetes)
    for _ in 0..10 {
        let response = server.get("/health").await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }
}

#[tokio::test]
#[serial]
async fn test_readiness_endpoint_healthy() {
    let server = setup_test_server().await;

    let response = server.get("/health/ready").await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "ready");

    // Verify all checks passed
    assert_eq!(body["checks"]["database"], "ok");
    assert_eq!(body["checks"]["event_source"], "ok");
    assert_eq!(body["checks"]["queue"], "ok");
}

#[tokio::test]
#[serial]
async fn test_readiness_endpoint_multiple_calls() {
    let server = setup_test_server().await;

    // Simulate repeated readiness checks
    for _ in 0..10 {
        let response = server.get("/health/ready").await;
        assert_eq!(response.status_code(), StatusCode::OK);

        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "ready");
    }
}

#[tokio::test]
#[serial]
async fn test_service_info_endpoint() {
    let server = setup_test_server().await;

    let response = server.get("/api/v1/info").await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();

    // Verify all required fields are present
    assert_eq!(body["version"], "0.2.0-test");
    assert_eq!(body["build_timestamp"], "2025-10-30T00:00:00Z");
    assert_eq!(body["build_git_hash"], "test123");
    assert_eq!(body["api_version"], "v1");

    // Verify features array
    let features = body["features"]
        .as_array()
        .expect("features should be an array");
    assert_eq!(features.len(), 3);
    assert!(features.iter().any(|f| f == "workflows"));
    assert!(features.iter().any(|f| f == "workers"));
    assert!(features.iter().any(|f| f == "websockets"));
}

#[tokio::test]
#[serial]
async fn test_service_info_no_auth_required() {
    let server = setup_test_server().await;

    // Verify endpoint doesn't require authentication
    let response = server.get("/api/v1/info").await;
    assert_eq!(response.status_code(), StatusCode::OK);
}

#[tokio::test]
#[serial]
async fn test_health_checks_run_in_parallel() {
    let server = setup_test_server().await;

    // Measure readiness check latency
    let start = std::time::Instant::now();
    let response = server.get("/health/ready").await;
    let duration = start.elapsed();

    assert_eq!(response.status_code(), StatusCode::OK);

    // With parallel execution (tokio::join!), readiness check should be fast
    // even though it runs 3 checks. If checks were sequential, this would take
    // much longer. We allow 100ms P99 per requirements.
    assert!(
        duration.as_millis() < 100,
        "Readiness check took {}ms, expected <100ms",
        duration.as_millis()
    );
}

#[tokio::test]
#[serial]
async fn test_liveness_latency() {
    let server = setup_test_server().await;

    // Measure liveness check latency
    let start = std::time::Instant::now();
    let response = server.get("/health").await;
    let duration = start.elapsed();

    assert_eq!(response.status_code(), StatusCode::OK);

    // Liveness should be very fast (<1ms P99 per requirements)
    // Allow some tolerance for test overhead
    assert!(
        duration.as_millis() < 10,
        "Liveness check took {}ms, expected <10ms",
        duration.as_millis()
    );
}

#[tokio::test]
#[serial]
async fn test_all_health_endpoints_available() {
    let server = setup_test_server().await;

    // Test that all health endpoints are registered and accessible
    let endpoints = vec!["/health", "/health/ready", "/api/v1/info"];

    for endpoint in endpoints {
        let response = server.get(endpoint).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "Endpoint {} should return 200 OK",
            endpoint
        );
    }
}

#[tokio::test]
#[serial]
async fn test_readiness_includes_all_checks() {
    let server = setup_test_server().await;

    let response = server.get("/health/ready").await;
    let body: serde_json::Value = response.json();

    // Verify all three health checks are included
    assert!(
        body["checks"]["database"].is_string(),
        "database check should be present"
    );
    assert!(
        body["checks"]["event_source"].is_string(),
        "event_source check should be present"
    );
    assert!(
        body["checks"]["queue"].is_string(),
        "queue check should be present"
    );
}

#[tokio::test]
#[serial]
async fn test_kubernetes_simulation() {
    let server = setup_test_server().await;

    // Simulate Kubernetes probes running sequentially (since TestServer can't be cloned)
    // In production, these would run on separate threads/pods

    // Liveness probes (every 10s in production)
    for _ in 0..5 {
        let response = server.get("/health").await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }

    // Readiness probes (every 5s in production)
    for _ in 0..5 {
        let response = server.get("/health/ready").await;
        assert_eq!(response.status_code(), StatusCode::OK);
    }
}

// Tests for individual health check functions
// These test the health check functions directly, not through HTTP endpoints

#[tokio::test]
#[serial]
async fn test_check_database_health_success() {
    let pool = setup_test_pool().await;

    // Test that database health check passes with a working database
    let result = streamflow_api::health::check_database_health(&pool).await;
    assert!(
        result.is_ok(),
        "Database health check should succeed with working database"
    );
}

#[tokio::test]
#[serial]
async fn test_check_event_source_health_success() {
    let pool = setup_test_pool().await;

    // Test that event source health check passes
    // Event source is PostgreSQL-based, so this tests the event_source_commands table
    let result = streamflow_api::health::check_event_source_health(&pool).await;
    assert!(
        result.is_ok(),
        "Event source health check should succeed with working database"
    );
}

#[tokio::test]
#[serial]
async fn test_check_activity_queue_health_success() {
    let pool = setup_test_pool().await;

    // Test that activity queue health check passes
    // Activity queue is PostgreSQL-based, so this tests the activity_tasks table
    let result = streamflow_api::health::check_activity_queue_health(&pool).await;
    assert!(
        result.is_ok(),
        "Activity queue health check should succeed with working database"
    );
}
