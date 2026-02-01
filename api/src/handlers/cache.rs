//! Cache invalidation API handlers
//!
//! Provides endpoints for manual cache invalidation:
//! - DELETE /api/v1/cache/:key - Invalidate specific cache entry by cache key
//! - POST /api/v1/cache/invalidate - Invalidate multiple cache entries by pattern
//!
//! All endpoints require JWT authentication.

use crate::error::AppError;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Response from cache invalidation operations
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvalidateResponse {
    /// Whether the operation succeeded
    pub success: bool,
    /// Number of cache entries invalidated
    pub count: usize,
}

/// Request body for pattern-based cache invalidation
#[derive(Debug, Deserialize, ToSchema)]
pub struct InvalidatePatternRequest {
    /// Redis pattern to match cache keys (e.g., "builtin.llm_prompt:*")
    ///
    /// Supports Redis glob-style patterns:
    /// - `*` matches any characters
    /// - `?` matches exactly one character
    /// - `[abc]` matches one character from set
    ///
    /// Examples:
    /// - `builtin.llm_prompt:*` - All LLM prompt activity caches
    /// - `builtin.http_request:*` - All HTTP request activity caches
    /// - `*` - All cache entries (use with caution)
    #[schema(example = "builtin.llm_prompt:*")]
    pub pattern: String,
}

/// Invalidate a specific cache entry by key
///
/// Deletes a single cache entry identified by its cache key. The cache key is
/// returned in the `metadata.cache_key` field of workflow activity results.
///
/// # Authentication
/// Requires valid JWT Bearer token.
///
/// # Example
/// ```bash
/// # Get cache_key from workflow status
/// CACHE_KEY=$(curl http://localhost:8080/api/v1/workflows/wf_123 \
///   -H "Authorization: Bearer $TOKEN" \
///   | jq -r '.activities.analyze_sentiment.metadata.cache_key')
///
/// # Invalidate the cache entry
/// curl -X DELETE "http://localhost:8080/api/v1/cache/$CACHE_KEY" \
///   -H "Authorization: Bearer $TOKEN"
/// ```
#[utoipa::path(
    delete,
    path = "/api/v1/cache/{key}",
    tag = "Cache",
    params(
        ("key" = String, Path, description = "Cache key (SHA256 hash from activity metadata)")
    ),
    responses(
        (status = 200, description = "Cache entry invalidated successfully", body = InvalidateResponse),
        (status = 401, description = "Unauthorized - Invalid or missing JWT token"),
        (status = 500, description = "Internal server error - Cache invalidation failed")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn invalidate_cache_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<InvalidateResponse>, AppError> {
    // Invalidate single cache entry
    state.cache_service.invalidate(&key).await.map_err(|e| {
        tracing::error!(
            cache_key = %key,
            error = %e,
            "Failed to invalidate cache entry"
        );
        AppError::InternalError(anyhow::anyhow!("Failed to invalidate cache: {}", e))
    })?;

    tracing::info!(
        cache_key = %key,
        "Successfully invalidated cache entry"
    );

    Ok(Json(InvalidateResponse {
        success: true,
        count: 1,
    }))
}

/// Invalidate cache entries matching a pattern
///
/// Deletes all cache entries matching the specified Redis glob pattern.
/// Uses Redis SCAN for safe pattern matching (does not block the server).
///
/// # Authentication
/// Requires valid JWT Bearer token.
///
/// # Pattern Examples
/// - `builtin.llm_prompt:*` - Invalidate all LLM prompt activity caches
/// - `builtin.http_request:*` - Invalidate all HTTP request activity caches
/// - `builtin.*:*` - Invalidate all built-in activity caches
/// - `*` - Invalidate all cache entries (use with caution)
///
/// # Example
/// ```bash
/// # Invalidate all LLM prompt caches
/// curl -X POST http://localhost:8080/api/v1/cache/invalidate \
///   -H "Authorization: Bearer $TOKEN" \
///   -H "Content-Type: application/json" \
///   -d '{"pattern": "builtin.llm_prompt:*"}'
///
/// # Invalidate all HTTP request caches
/// curl -X POST http://localhost:8080/api/v1/cache/invalidate \
///   -H "Authorization: Bearer $TOKEN" \
///   -H "Content-Type: application/json" \
///   -d '{"pattern": "builtin.http_request:*"}'
/// ```
#[utoipa::path(
    post,
    path = "/api/v1/cache/invalidate",
    tag = "Cache",
    request_body = InvalidatePatternRequest,
    responses(
        (status = 200, description = "Cache entries invalidated successfully", body = InvalidateResponse),
        (status = 401, description = "Unauthorized - Invalid or missing JWT token"),
        (status = 500, description = "Internal server error - Cache invalidation failed")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn invalidate_cache_pattern(
    State(state): State<AppState>,
    Json(payload): Json<InvalidatePatternRequest>,
) -> Result<Json<InvalidateResponse>, AppError> {
    // Invalidate all cache entries matching pattern
    let count = state
        .cache_service
        .invalidate_pattern(&payload.pattern)
        .await
        .map_err(|e| {
            tracing::error!(
                pattern = %payload.pattern,
                error = %e,
                "Failed to invalidate cache pattern"
            );
            AppError::InternalError(anyhow::anyhow!("Failed to invalidate cache pattern: {}", e))
        })?;

    tracing::info!(
        pattern = %payload.pattern,
        count = count,
        "Successfully invalidated cache entries matching pattern"
    );

    Ok(Json(InvalidateResponse {
        success: true,
        count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::app_router;
    use crate::state::AppState;
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use bcrypt;
    use kruxiaflow_core::PostgresSubscriptionService;
    use kruxiaflow_core::cache::NoOpCache;
    use kruxiaflow_core::events::PostgresEventSource;
    use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
    use kruxiaflow_core::storage::PostgresStorage;
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
        include_str!("../../../oauth/tests/private.pem").to_string()
    }

    /// Generate test RSA public key
    fn test_rsa_public_key() -> String {
        include_str!("../../../oauth/tests/public.pem").to_string()
    }

    /// Create OAuth client for testing
    async fn create_test_oauth_client(pool: &PgPool, client_id: &str, client_secret: &str) {
        let secret_hash = bcrypt::hash(client_secret, bcrypt::DEFAULT_COST).unwrap();
        sqlx::query(
            "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (client_id) DO NOTHING",
        )
        .bind(client_id)
        .bind(&secret_hash)
        .bind("Test Client")
        .execute(pool)
        .await
        .expect("Failed to create test OAuth client");
    }

    /// Helper to create authenticated test client
    async fn setup_auth_client() -> (TestServer, String) {
        let pool = setup_test_pool().await;

        // Create OAuth client in database
        create_test_oauth_client(&pool, "test_client", "test_secret").await;

        let auth_config = AuthConfig {
            rsa_private_key_pem: test_rsa_private_key(),
            rsa_public_key_pem: Some(test_rsa_public_key()),
            jwt_issuer: "test".to_string(),
            jwt_audience: "test".to_string(),
            token_ttl: 3600,
        };

        let auth_service = Arc::new(
            PostgresAuthService::new(pool.clone(), auth_config)
                .expect("Failed to create auth service"),
        );

        let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
        let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
        let workflow_storage = Arc::new(PostgresStorage::new(pool.clone()));
        let cache_service = Arc::new(NoOpCache::new());
        let shutdown_token = CancellationToken::new();

        let subscription_service = Arc::new(PostgresSubscriptionService::new(pool.clone()));
        let state = AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            subscription_service,
            shutdown_token,
        );

        let app = app_router(state);
        let server = TestServer::new(app).expect("Failed to create test server");

        // Get auth token
        let token_response = server
            .post("/api/v1/oauth/token")
            .json(&serde_json::json!({
                "grant_type": "client_credentials",
                "client_id": "test_client",
                "client_secret": "test_secret"
            }))
            .await;

        let token: serde_json::Value = token_response.json();
        let access_token = token["access_token"].as_str().unwrap().to_string();

        (server, access_token)
    }

    #[tokio::test]
    #[serial]
    async fn test_invalidate_cache_key_requires_auth() {
        let (server, _token) = setup_auth_client().await;

        // Request without auth should fail
        let response = server.delete("/api/v1/cache/test_key_123").await;
        assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_invalidate_cache_pattern_requires_auth() {
        let (server, _token) = setup_auth_client().await;

        // Request without auth should fail
        let response = server
            .post("/api/v1/cache/invalidate")
            .json(&serde_json::json!({
                "pattern": "test:*"
            }))
            .await;

        assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_invalidate_cache_key_with_auth() {
        let (server, token) = setup_auth_client().await;

        // Request with auth should succeed (NoOpCache always succeeds)
        let response = server
            .delete("/api/v1/cache/test_key_456")
            .add_header(
                axum::http::HeaderName::from_static("authorization"),
                axum::http::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            )
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);

        let body: InvalidateResponse = response.json();
        assert!(body.success);
        assert_eq!(body.count, 1);
    }

    #[tokio::test]
    #[serial]
    async fn test_invalidate_cache_pattern_with_auth() {
        let (server, token) = setup_auth_client().await;

        // Request with auth should succeed (NoOpCache returns 0 invalidated)
        let response = server
            .post("/api/v1/cache/invalidate")
            .add_header(
                axum::http::HeaderName::from_static("authorization"),
                axum::http::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            )
            .json(&serde_json::json!({
                "pattern": "builtin.llm_prompt:*"
            }))
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);

        let body: InvalidateResponse = response.json();
        assert!(body.success);
        // NoOpCache returns 0 for pattern invalidation
        assert_eq!(body.count, 0);
    }
}
