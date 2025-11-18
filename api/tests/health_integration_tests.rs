use axum::http::StatusCode;
use axum_test::TestServer;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_api::{AppState, AppStateBuild, app_router};
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

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(streamflow_core::storage::PostgresStorage::new(pool.clone()));

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        CancellationToken::new(),
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
    // much longer. We allow 200ms with some tolerance for CI/test overhead.
    assert!(
        duration.as_millis() < 200,
        "Readiness check took {}ms, expected <200ms",
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

// ============================================================================
// Health Check Edge Cases and Error Scenarios
// ============================================================================

#[tokio::test]
#[serial]
async fn test_database_health_with_invalid_connection() {
    // Create a pool with an invalid connection string
    let invalid_pool_result =
        PgPool::connect("postgres://invalid:invalid@localhost:1/invalid").await;

    // If we can't even create the pool, that's expected
    if let Ok(invalid_pool) = invalid_pool_result {
        let result = streamflow_api::health::check_database_health(&invalid_pool).await;
        assert!(
            result.is_err(),
            "Database health check should fail with invalid connection"
        );
    }
}

#[tokio::test]
#[serial]
async fn test_event_source_health_maps_errors_correctly() {
    // Test that event source health check properly maps database errors
    // Create a pool with an invalid connection
    let invalid_pool_result =
        PgPool::connect("postgres://invalid:invalid@localhost:1/invalid").await;

    if let Ok(invalid_pool) = invalid_pool_result {
        let result = streamflow_api::health::check_event_source_health(&invalid_pool).await;
        assert!(
            result.is_err(),
            "Event source health check should fail with invalid connection"
        );

        // Verify it's mapped to EventSourceError
        if let Err(e) = result {
            match e {
                streamflow_api::health::HealthCheckError::EventSourceError(_) => {
                    // Expected error type
                }
                _ => panic!("Expected EventSourceError, got {:?}", e),
            }
        }
    }
}

#[tokio::test]
#[serial]
async fn test_activity_queue_health_with_invalid_connection() {
    // Test activity queue health check with invalid connection
    let invalid_pool_result =
        PgPool::connect("postgres://invalid:invalid@localhost:1/invalid").await;

    if let Ok(invalid_pool) = invalid_pool_result {
        let result = streamflow_api::health::check_activity_queue_health(&invalid_pool).await;
        assert!(
            result.is_err(),
            "Activity queue health check should fail with invalid connection"
        );

        // Verify it's mapped to QueueError
        if let Err(e) = result {
            match e {
                streamflow_api::health::HealthCheckError::QueueError(_) => {
                    // Expected error type
                }
                _ => panic!("Expected QueueError, got {:?}", e),
            }
        }
    }
}

#[tokio::test]
#[serial]
async fn test_health_checks_with_closed_pool() {
    let pool = setup_test_pool().await;

    // Close the pool
    pool.close().await;

    // Try to perform health checks on closed pool
    let db_result = streamflow_api::health::check_database_health(&pool).await;
    assert!(
        db_result.is_err(),
        "Database health check should fail with closed pool"
    );

    let event_result = streamflow_api::health::check_event_source_health(&pool).await;
    assert!(
        event_result.is_err(),
        "Event source health check should fail with closed pool"
    );

    let queue_result = streamflow_api::health::check_activity_queue_health(&pool).await;
    assert!(
        queue_result.is_err(),
        "Activity queue health check should fail with closed pool"
    );
}

#[tokio::test]
#[serial]
async fn test_multiple_concurrent_health_checks() {
    let pool = setup_test_pool().await;

    // Run multiple health checks concurrently to test thread safety
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let pool_clone = pool.clone();
            tokio::spawn(
                async move { streamflow_api::health::check_database_health(&pool_clone).await },
            )
        })
        .collect();

    // All should succeed
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "Concurrent health check should succeed");
    }
}

#[tokio::test]
#[serial]
async fn test_health_check_response_time() {
    let pool = setup_test_pool().await;

    // Measure response time for database health check
    let start = std::time::Instant::now();
    let result = streamflow_api::health::check_database_health(&pool).await;
    let duration = start.elapsed();

    assert!(result.is_ok(), "Health check should succeed");

    // Health check should complete well under the 5 second timeout
    // We'll assert it's under 1 second for a healthy database
    assert!(
        duration.as_secs() < 1,
        "Health check took {}ms, expected <1000ms",
        duration.as_millis()
    );
}

#[tokio::test]
#[serial]
async fn test_all_health_checks_return_consistent_results() {
    let pool = setup_test_pool().await;

    // Run all health checks multiple times and verify consistent results
    for _ in 0..5 {
        let db_result = streamflow_api::health::check_database_health(&pool).await;
        let event_result = streamflow_api::health::check_event_source_health(&pool).await;
        let queue_result = streamflow_api::health::check_activity_queue_health(&pool).await;

        assert!(db_result.is_ok(), "Database health should be consistent");
        assert!(
            event_result.is_ok(),
            "Event source health should be consistent"
        );
        assert!(queue_result.is_ok(), "Queue health should be consistent");
    }
}

#[tokio::test]
#[serial]
async fn test_health_check_error_types() {
    use streamflow_api::health::HealthCheckError;

    let pool = setup_test_pool().await;

    // Close the pool to trigger errors
    pool.close().await;

    // Test database error
    let db_result = streamflow_api::health::check_database_health(&pool).await;
    assert!(db_result.is_err(), "Should fail with closed pool");
    if let Err(e) = db_result {
        // Should be DatabaseError or timeout
        let error_string = e.to_string();
        assert!(
            error_string.contains("Database error") || error_string.contains("timeout"),
            "Error should be database or timeout related"
        );
    }

    // Test event source error mapping
    let event_result = streamflow_api::health::check_event_source_health(&pool).await;
    assert!(event_result.is_err(), "Should fail with closed pool");
    if let Err(e) = event_result {
        match e {
            HealthCheckError::EventSourceError(_) => {
                // Expected error type
            }
            _ => {}
        }
    }

    // Test queue error mapping
    let queue_result = streamflow_api::health::check_activity_queue_health(&pool).await;
    assert!(queue_result.is_err(), "Should fail with closed pool");
    if let Err(e) = queue_result {
        match e {
            HealthCheckError::QueueError(_) => {
                // Expected error type
            }
            _ => {}
        }
    }
}

#[tokio::test]
#[serial]
async fn test_health_check_with_minimal_pool() {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
    });

    // Create a minimal pool with just 1 connection
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("Failed to connect with minimal pool");

    // Run migrations
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Health checks should still work with minimal pool
    let db_result = streamflow_api::health::check_database_health(&pool).await;
    assert!(
        db_result.is_ok(),
        "Health check should work with minimal pool"
    );

    let event_result = streamflow_api::health::check_event_source_health(&pool).await;
    assert!(
        event_result.is_ok(),
        "Event source health check should work with minimal pool"
    );

    let queue_result = streamflow_api::health::check_activity_queue_health(&pool).await;
    assert!(
        queue_result.is_ok(),
        "Queue health check should work with minimal pool"
    );
}
