use crate::error::AppError;
use crate::state::AppState;
use crate::{handlers, middleware, openapi};
use axum::{
    Json, Router, middleware as axum_middleware,
    response::IntoResponse,
    routing::{get, post},
};
use utoipa::OpenApi;
use utoipa_redoc::{Redoc, Servable};

/// Fallback handler for 404 errors
async fn fallback_404() -> impl IntoResponse {
    AppError::NotFound("The requested resource was not found".to_string())
}

/// Public routes (no authentication required)
///
/// Routes:
/// - GET /health - Liveness probe
/// - GET /health/ready - Readiness probe
/// - GET /api/v1/info - Service information
/// - POST /api/v1/oauth/token - OAuth 2.0 token issuance
///
/// These routes are accessible without authentication.
///
/// Note: The token endpoint accepts both application/json and
/// application/x-www-form-urlencoded per OAuth 2.0 spec using
/// Axum's Either extractor to handle both content types.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(handlers::liveness_handler))
        .route("/health/ready", get(handlers::readiness_handler))
        .route("/api/v1/info", get(handlers::service_info_handler))
        .route("/api/v1/oauth/token", post(handlers::token_handler))
}

/// Protected API routes (require authentication)
///
/// Routes:
/// - (Future) POST /api/v1/workflows - Submit workflow
/// - (Future) GET /api/v1/workflows/{id} - Query workflow
///
/// All routes in this group require valid JWT Bearer token.
/// Authentication middleware is applied in app_router() after with_state().
pub fn protected_routes() -> Router<AppState> {
    Router::new()
    // Future routes will be added here
}

/// Create the complete application router
///
/// Combines public and protected route groups with appropriate middleware.
///
/// Middleware stack (outer to inner):
/// 1. CORS - Cross-origin resource sharing
/// 2. Request ID - Unique ID for request tracing
/// 3. Authentication - Protected routes only (applied per-group)
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

    // Clone state for middleware use
    let auth_state = state.clone();

    Router::new()
        .merge(public_routes())
        .merge(
            protected_routes()
                // Apply authentication middleware to protected routes only
                .layer(axum_middleware::from_fn(move |req, next| {
                    let state = auth_state.clone();
                    async move {
                        middleware::auth_middleware(axum::extract::State(state), req, next).await
                    }
                })),
        )
        // Serve ReDoc documentation UI at /api/v1/docs
        .merge(Redoc::with_url("/api/v1/docs", openapi.clone()))
        // Serve OpenAPI JSON spec at /api/v1/openapi.json
        .route("/api/v1/openapi.json", get(|| async move { Json(openapi) }))
        // Fallback handler for 404 errors
        .fallback(fallback_404)
        .with_state(state)
        // Apply global middleware (request ID, CORS)
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
