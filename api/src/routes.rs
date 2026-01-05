use crate::error::AppError;
use crate::state::AppState;
use crate::{handlers, middleware, openapi};
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    middleware as axum_middleware,
    response::IntoResponse,
    routing::{delete, get, post},
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
/// - GET /health/pool - Connection pool metrics
/// - GET /api/v1/info - Service information
/// - POST /api/v1/oauth/token - OAuth 2.0 token issuance
/// - GET /api/v1/activities/{id}/ws - WebSocket for activity streaming (auth via query param)
///
/// These routes are accessible without HTTP header authentication.
///
/// Note: The token endpoint accepts both application/json and
/// application/x-www-form-urlencoded per OAuth 2.0 spec using
/// Axum's Either extractor to handle both content types.
///
/// Note: The WebSocket endpoint handles authentication via query parameter
/// (?token=<jwt>) since WebSocket upgrade bypasses HTTP middleware.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(handlers::liveness_handler))
        .route("/health/ready", get(handlers::readiness_handler))
        .route("/health/pool", get(handlers::pool_metrics_handler))
        .route("/api/v1/info", get(handlers::service_info_handler))
        .route("/api/v1/oauth/token", post(handlers::token_handler))
        // WebSocket endpoint - auth handled in handler via query param
        .route(
            "/api/v1/activities/:activity_id/ws",
            get(handlers::activity_stream_handler),
        )
}

/// Protected API routes (require authentication)
///
/// Routes:
/// - POST /api/v1/workflow_definitions - Deploy workflow definition
/// - GET /api/v1/workflow_definitions - List workflow definitions
/// - GET /api/v1/workflow_definitions/{name} - Get workflow definition
/// - POST /api/v1/workflows - Submit workflow
/// - GET /api/v1/workflows - List workflows with filters
/// - GET /api/v1/workflows/{id} - Get workflow by ID
/// - GET /api/v1/workflows/{workflow_id}/output - Get workflow output
/// - GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output - Get activity output
/// - GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename} - Download file
/// - POST /api/v1/workers/poll - Poll for activities
/// - POST /api/v1/activities/{activity_id}/heartbeat - Send heartbeat
/// - POST /api/v1/activities/{activity_id}/complete - Complete activity
/// - POST /api/v1/activities/{activity_id}/fail - Fail activity
/// - GET /api/v1/llm/providers - List all LLM providers
/// - POST /api/v1/llm/models/search - Search for LLM models
/// - GET /api/v1/workflows/{workflow_id}/cost - Get workflow cost summary
/// - GET /api/v1/workflows/{workflow_id}/cost/history - Get workflow cost history
/// - GET /api/v1/cost/analytics - Get cost analytics
/// - DELETE /api/v1/cache/{key} - Invalidate specific cache entry
/// - POST /api/v1/cache/invalidate - Invalidate cache entries by pattern
///
/// All routes in this group require valid JWT Bearer token.
/// Authentication middleware is applied in app_router() after with_state().
pub fn protected_routes() -> Router<AppState> {
    Router::new()
        // Workflow Definition Management
        .route(
            "/api/v1/workflow_definitions",
            post(handlers::deploy_workflow_definition).get(handlers::list_workflow_definitions),
        )
        .route(
            "/api/v1/workflow_definitions/:name",
            get(handlers::get_workflow_definition),
        )
        // Workflow Submission and Query
        .route(
            "/api/v1/workflows",
            post(handlers::submit_workflow).get(handlers::list_workflows),
        )
        .route(
            "/api/v1/workflows/:workflow_id",
            get(handlers::get_workflow),
        )
        // Workflow Cost Tracking
        .route(
            "/api/v1/workflows/:workflow_id/cost",
            get(handlers::get_workflow_cost),
        )
        .route(
            "/api/v1/workflows/:workflow_id/cost/history",
            get(handlers::get_workflow_cost_history),
        )
        .route("/api/v1/cost/analytics", get(handlers::get_cost_analytics))
        // Output Retrieval APIs
        .route(
            "/api/v1/workflows/:workflow_id/output",
            get(handlers::get_workflow_output),
        )
        .route(
            "/api/v1/workflows/:workflow_id/activities/:activity_key/output",
            get(handlers::get_activity_output),
        )
        .route(
            "/api/v1/workflows/:workflow_id/activities/:activity_key/files/:filename",
            get(handlers::download_activity_file),
        )
        // LLM Provider Catalog
        .route("/api/v1/llm/providers", get(handlers::list_providers))
        .route("/api/v1/llm/models/search", post(handlers::search_models))
        // Cache Invalidation
        .route("/api/v1/cache/:key", delete(handlers::invalidate_cache_key))
        .route(
            "/api/v1/cache/invalidate",
            post(handlers::invalidate_cache_pattern),
        )
        // Worker Activity APIs
        .route("/api/v1/workers/poll", post(handlers::poll_activities))
        .route(
            "/api/v1/activities/:activity_id/heartbeat",
            post(handlers::heartbeat_activity),
        )
        .route(
            "/api/v1/activities/:activity_id/complete",
            post(handlers::complete_activity),
        )
        .route(
            "/api/v1/activities/:activity_id/fail",
            post(handlers::fail_activity),
        )
        // Streaming APIs (for workers to publish tokens)
        .route(
            "/api/v1/activities/:activity_id/ws/token",
            post(handlers::publish_stream_token),
        )
        .route(
            "/api/v1/activities/:activity_id/ws/complete",
            post(handlers::publish_stream_complete),
        )
        .route(
            "/api/v1/activities/:activity_id/ws/error",
            post(handlers::publish_stream_error),
        )
        .route(
            "/api/v1/activities/:activity_id/ws/subscribers",
            get(handlers::get_subscriber_count),
        )
}

/// Create the complete application router
///
/// Combines public and protected route groups with appropriate middleware.
///
/// Middleware stack (outer to inner):
/// 1. CORS - Cross-origin resource sharing
/// 2. Request ID - Unique ID for request tracing
/// 3. Shutdown check - Return 503 during graceful shutdown (protected routes only)
/// 4. Authentication - Protected routes only (applied per-group)
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
    let shutdown_state = state.clone();

    Router::new()
        .merge(public_routes())
        .merge(
            protected_routes()
                // Apply shutdown check middleware first (returns 503 during shutdown)
                .layer(axum_middleware::from_fn(move |req, next| {
                    let state = shutdown_state.clone();
                    async move {
                        middleware::shutdown_check(axum::extract::State(state), req, next).await
                    }
                }))
                // Apply authentication middleware to protected routes
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
        // Apply global middleware (request ID, CORS, body limit)
        .layer(axum_middleware::from_fn(middleware::request_id_middleware))
        .layer(middleware::cors_layer())
        // Set body size limit to 50MB for large embedding payloads
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;
    use kruxiaflow_core::events::PostgresEventSource;
    use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
    use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
    use serial_test::serial;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    /// Helper to create test database pool
    async fn setup_test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
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

        let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
        let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
        let workflow_storage =
            Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
        let cache_service = Arc::new(kruxiaflow_core::NoOpCache::new());
        let shutdown_token = CancellationToken::new();

        let state = AppState::new(
            pool,
            Arc::new(auth_service),
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );
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

        let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
        let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
        let workflow_storage =
            Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
        let cache_service = Arc::new(kruxiaflow_core::NoOpCache::new());
        let shutdown_token = CancellationToken::new();

        let state = AppState::new(
            pool,
            Arc::new(auth_service),
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );
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

        let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
        let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
        let workflow_storage =
            Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
        let cache_service = Arc::new(kruxiaflow_core::NoOpCache::new());
        let shutdown_token = CancellationToken::new();

        let state = AppState::new(
            pool,
            Arc::new(auth_service),
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );
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
