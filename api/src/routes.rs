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
    use super::*;
    use axum_test::TestServer;
    use serial_test::serial;
    use sqlx::PgPool;
    use std::sync::Arc;
    use streamflow_oauth::{AuthConfig, PostgresAuthService};

    /// Helper to create test database pool
    async fn setup_test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
        });

        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    /// Generate test RSA private key
    fn test_rsa_private_key() -> String {
        include_str!("../../oauth/tests/private.pem").to_string()
    }

    /// Generate test RSA public key
    fn test_rsa_public_key() -> String {
        include_str!("../../oauth/tests/public.pem").to_string()
    }

    #[tokio::test]
    #[serial]
    async fn test_public_routes_creation() {
        // Test that public_routes() creates a router successfully
        let router = public_routes();
        // Just verifying it compiles and creates a Router<AppState>
        assert!(std::mem::size_of_val(&router) > 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_protected_routes_creation() {
        // Test that protected_routes() creates a router successfully
        let router = protected_routes();
        // Just verifying it compiles and creates a Router<AppState>
        assert!(std::mem::size_of_val(&router) > 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_app_router_creation() {
        // Test that app_router() creates a router with state
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

        let state = AppState::new(pool, Arc::new(auth_service));
        let router = app_router(state);

        // Just verifying it compiles and creates a Router
        assert!(std::mem::size_of_val(&router) > 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_fallback_404_handler() {
        // Test that fallback_404 returns NotFound error
        let response = fallback_404().await;
        let body_bytes = axum::body::to_bytes(response.into_response().into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let body_str = String::from_utf8(body_bytes.to_vec()).expect("Invalid UTF-8");
        assert!(body_str.contains("not found") || body_str.contains("Not Found"));
    }

    #[tokio::test]
    #[serial]
    async fn test_404_for_unknown_route() {
        // Test that unknown routes return 404
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

        let state = AppState::new(pool, Arc::new(auth_service));
        let app = app_router(state);
        let server = TestServer::new(app).expect("Failed to create test server");

        let response = server.get("/nonexistent-route").await;
        assert_eq!(response.status_code(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[serial]
    async fn test_openapi_json_endpoint() {
        // Test that /api/v1/openapi.json returns OpenAPI spec
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

        let state = AppState::new(pool, Arc::new(auth_service));
        let app = app_router(state);
        let server = TestServer::new(app).expect("Failed to create test server");

        let response = server.get("/api/v1/openapi.json").await;
        assert_eq!(response.status_code(), axum::http::StatusCode::OK);

        let body: serde_json::Value = response.json();
        assert!(body.get("openapi").is_some());
        assert!(body.get("info").is_some());
    }

    // Integration tests for routes are in:
    // tests/health_integration_tests.rs
    // tests/error_handling_test.rs
}
