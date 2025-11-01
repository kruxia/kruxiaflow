use crate::error::AppError;
use crate::state::AppState;
use crate::{handlers, middleware, openapi};
use axum::{Json, Router, middleware as axum_middleware, response::IntoResponse, routing::get};
use utoipa::OpenApi;
use utoipa_redoc::{Redoc, Servable};

/// Fallback handler for 404 errors
async fn fallback_404() -> impl IntoResponse {
    AppError::NotFound("The requested resource was not found".to_string())
}

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
///
/// Middleware stack (applied in order):
/// 1. CORS - Cross-origin resource sharing
/// 2. Request ID - Unique ID for request tracing
/// 3. (Future: Rate limiting, authentication)
///
/// Documentation:
/// - ReDoc UI served at /api/v1/docs
/// - OpenAPI spec served at /api/v1/openapi.json
///
/// # Arguments
/// * `state` - Application state to share across handlers
///
/// # Returns
/// Configured Axum router ready to serve requests
pub fn app_router(state: AppState) -> Router {
    // Generate OpenAPI specification from annotated handlers
    let openapi = openapi::ApiDoc::openapi();

    Router::new()
        .merge(health_routes())
        .merge(api_routes())
        // Serve ReDoc documentation UI at /api/v1/docs
        .merge(Redoc::with_url("/api/v1/docs", openapi.clone()))
        // Serve OpenAPI JSON spec at /api/v1/openapi.json
        .route("/api/v1/openapi.json", get(|| async move { Json(openapi) }))
        // Fallback handler for 404 errors
        .fallback(fallback_404)
        .with_state(state)
        // Apply middleware (in reverse order of execution)
        .layer(axum_middleware::from_fn(middleware::request_id_middleware))
        .layer(middleware::cors_layer())
}

#[cfg(test)]
mod tests {
    // Unit tests would go here
    // For proper unit tests, we'd need to mock dependencies
    //
    // Integration tests for routes are in:
    // tests/health_integration_tests.rs
    // tests/error_handling_test.rs
}
