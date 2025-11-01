use crate::health::{
    LivenessResponse, ReadinessResponse, ServiceInfo, check_activity_queue_health,
    check_database_health, check_event_source_health,
};
use crate::state::AppState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use std::collections::HashMap;

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
    (
        StatusCode::OK,
        Json(LivenessResponse {
            status: "ok".to_string(),
        }),
    )
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

    let mut checks = HashMap::new();
    let mut all_healthy = true;

    // Process database check result
    match db_result {
        Ok(_) => {
            checks.insert("database".to_string(), "ok".to_string());
        }
        Err(e) => {
            checks.insert("database".to_string(), "unhealthy".to_string());
            all_healthy = false;
            tracing::warn!("Database health check failed: {}", e);
        }
    }

    // Process event source check result
    match event_source_result {
        Ok(_) => {
            checks.insert("event_source".to_string(), "ok".to_string());
        }
        Err(e) => {
            checks.insert("event_source".to_string(), "unhealthy".to_string());
            all_healthy = false;
            tracing::warn!("Event source health check failed: {}", e);
        }
    }

    // Process activity queue check result
    match queue_result {
        Ok(_) => {
            checks.insert("queue".to_string(), "ok".to_string());
        }
        Err(e) => {
            checks.insert("queue".to_string(), "unhealthy".to_string());
            all_healthy = false;
            tracing::warn!("Activity queue health check failed: {}", e);
        }
    }

    let status_code = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(ReadinessResponse {
            status: if all_healthy {
                "ready".to_string()
            } else {
                "not_ready".to_string()
            },
            checks,
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

    // Integration tests for readiness_handler and service_info_handler
    // are in tests/health_integration_tests.rs (require database connection)
}
