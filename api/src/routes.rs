use crate::handlers;
use crate::state::AppState;
use axum::{Router, routing::get};

/// Create health check routes
///
/// Routes:
/// - GET /health - Liveness probe
/// - GET /health/ready - Readiness probe
///
/// These routes do not require authentication and should be available
/// outside any authentication middleware.
pub fn health_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(handlers::liveness_handler))
        .route("/health/ready", get(handlers::readiness_handler))
}

/// Create API routes
///
/// Routes:
/// - GET /api/v1/info - Service information
///
/// These routes may be subject to rate limiting in the future,
/// but do not require authentication for MVP.
pub fn api_routes() -> Router<AppState> {
    Router::new().route("/api/v1/info", get(handlers::service_info_handler))
}

/// Create the complete application router
///
/// Combines all route groups and configures middleware.
/// Health checks are outside authentication to allow load balancers
/// and orchestrators to probe the service.
///
/// # Arguments
/// * `state` - Application state to share across handlers
///
/// # Returns
/// Configured Axum router ready to serve requests
pub fn app_router(state: AppState) -> Router {
    Router::new()
        .merge(health_routes())
        .merge(api_routes())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    // Helper to create test state (would need actual DB pool for real tests)
    #[ignore]
    #[tokio::test]
    async fn test_health_route_registered() {
        // This would require a real database connection for integration testing
        // Placeholder to show structure
    }

    #[ignore]
    #[tokio::test]
    async fn test_api_info_route_registered() {
        // This would require a real database connection for integration testing
        // Placeholder to show structure
    }
}
