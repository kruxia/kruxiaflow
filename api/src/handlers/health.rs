use crate::health::{
    HealthCheckStatus, LivenessResponse, PoolMetricsResponse, ReadinessResponse, ServiceInfo,
    check_activity_queue_health, check_database_health, check_event_source_health,
    get_pool_metrics,
};
use crate::state::AppState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

/// Liveness probe handler
///
/// Returns 200 OK if the server is running and can accept HTTP requests.
/// This is a simple check with minimal processing.
///
/// # Response
/// - 200 OK: Server is alive
/// - Response body: `{"status": "ok"}`
///
/// # Performance
/// Target: <1ms P99 latency
#[utoipa::path(
    get,
    path = "/health",
    tag = "Health",
    responses(
        (status = 200, description = "Server is alive", body = LivenessResponse)
    )
)]
pub async fn liveness_handler() -> impl IntoResponse {
    // If this handler runs, server is alive
    (StatusCode::OK, Json(LivenessResponse { status: "ok" }))
}

/// Readiness probe handler
///
/// Returns 200 OK if the server can handle requests and all dependencies are healthy.
/// Checks database, event source, and activity queue in parallel using tokio::join!
///
/// # Response
/// - 200 OK: All dependencies are healthy
/// - 503 Service Unavailable: One or more dependencies are unhealthy
///
/// Response body includes detailed status for each check:
/// ```json
/// {
///   "status": "ready",
///   "checks": {
///     "database": "ok",
///     "event_source": "ok",
///     "queue": "ok"
///   }
/// }
/// ```
///
/// # Performance
/// Target: <100ms P99 latency
/// Uses parallel execution to minimize total latency
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "Health",
    responses(
        (status = 200, description = "Server is ready", body = ReadinessResponse),
        (status = 503, description = "Server is not ready", body = ReadinessResponse)
    )
)]
pub async fn readiness_handler(State(app_state): State<AppState>) -> impl IntoResponse {
    // Run all health checks in parallel for optimal performance
    let (db_result, event_source_result, queue_result) = tokio::join!(
        check_database_health(&app_state.db_pool),
        check_event_source_health(&app_state.db_pool),
        check_activity_queue_health(&app_state.db_pool)
    );

    // Use static strings to avoid allocations
    let database_status = match db_result {
        Ok(_) => "ok",
        Err(e) => {
            tracing::warn!("Database health check failed: {}", e);
            "unhealthy"
        }
    };

    let event_source_status = match event_source_result {
        Ok(_) => "ok",
        Err(e) => {
            tracing::warn!("Event source health check failed: {}", e);
            "unhealthy"
        }
    };

    let queue_status = match queue_result {
        Ok(_) => "ok",
        Err(e) => {
            tracing::warn!("Activity queue health check failed: {}", e);
            "unhealthy"
        }
    };

    let all_healthy =
        database_status == "ok" && event_source_status == "ok" && queue_status == "ok";

    let status_code = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(ReadinessResponse {
            status: if all_healthy { "ready" } else { "not_ready" },
            checks: HealthCheckStatus {
                database: database_status,
                event_source: event_source_status,
                queue: queue_status,
            },
        }),
    )
}

/// Service information handler
///
/// Returns service metadata for discovery and debugging.
/// Does not require authentication.
///
/// # Response
/// - 200 OK always
/// - Response body:
/// ```json
/// {
///   "version": "0.2.0",
///   "build_timestamp": "2025-10-30T12:34:56Z",
///   "build_git_hash": "abc1234",
///   "api_version": "v1",
///   "features": ["workflows", "workers", "websockets"]
/// }
/// ```
///
/// # Performance
/// Target: <1ms P99 latency
#[utoipa::path(
    get,
    path = "/api/v1/info",
    tag = "Service",
    responses(
        (status = 200, description = "Service information", body = ServiceInfo)
    )
)]
pub async fn service_info_handler(State(app_state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(ServiceInfo {
            version: app_state.version.clone(),
            build_timestamp: app_state.build.timestamp.clone(),
            build_git_hash: Some(app_state.build.git_hash.clone()),
            api_version: "v1".to_string(),
            features: app_state.features.clone(),
        }),
    )
}

/// Connection pool metrics handler
///
/// Returns current connection pool statistics for performance monitoring.
/// Useful for profiling and capacity planning.
///
/// # Response
/// - 200 OK always
/// - Response body includes pool size, active/idle connections, and utilization
///
/// # Performance
/// Target: <1ms P99 latency (no database query required)
#[utoipa::path(
    get,
    path = "/health/pool",
    tag = "Health",
    responses(
        (status = 200, description = "Connection pool metrics", body = PoolMetricsResponse)
    )
)]
pub async fn pool_metrics_handler(State(app_state): State<AppState>) -> impl IntoResponse {
    let metrics = get_pool_metrics(&app_state.db_pool);
    (StatusCode::OK, Json(metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_liveness_returns_200_ok() {
        let response = liveness_handler().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_liveness_response_format() {
        let response = liveness_handler().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&body_str).unwrap();

        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_service_info_handler_returns_version_and_features() {
        use crate::state::tests::*;
        use crate::state::{AppState, AppStateBuild};
        use kruxiaflow_core::cache::NoOpCache;
        use sqlx::PgPool;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });
        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        let build = AppStateBuild {
            timestamp: "2025-11-15T10:00:00Z".to_string(),
            git_hash: "abc123".to_string(),
        };
        let features = vec!["workflows".to_string(), "workers".to_string()];

        let state = AppState::with_metadata(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
            "1.0.0-test".to_string(),
            build,
            features,
        );

        let response = service_info_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], "1.0.0-test");
        assert_eq!(json["api_version"], "v1");
        assert_eq!(json["build_git_hash"], "abc123");
        assert_eq!(json["features"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_pool_metrics_handler_returns_200() {
        use crate::state::AppState;
        use crate::state::tests::*;
        use kruxiaflow_core::cache::NoOpCache;
        use sqlx::PgPool;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });
        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        let state = AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
        );

        let response = pool_metrics_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["max_connections"].as_u64().unwrap() > 0);
        assert!(json.get("utilization_percent").is_some());
        assert!(json.get("status").is_some());
    }

    // Integration tests for readiness_handler
    // are in tests/health_integration_tests.rs (require database connection)
}
