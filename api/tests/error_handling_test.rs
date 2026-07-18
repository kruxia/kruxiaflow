use axum::http::{HeaderName, Method, StatusCode};
use axum_test::TestServer;
use kruxiaflow_api::{ApiErrorResponse, AppState, AppStateBuild, app_router};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use sqlx::PgPool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
async fn setup_test_state(pool: PgPool) -> AppState {
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
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());

    let subscription_service = Arc::new(PostgresSubscriptionService::new(pool.clone()));
    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        cache_service,
        subscription_service,
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
async fn setup_test_server(pool: PgPool) -> TestServer {
    let state = setup_test_state(pool).await;
    let router = app_router(state);
    TestServer::new(router).expect("Failed to create test server")
}

// ============================================================================
// Request ID Middleware Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_request_id_generated(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/health").await;

    assert!(response.headers().contains_key("x-request-id"));
    let request_id = response
        .headers()
        .get("x-request-id")
        .expect("X-Request-ID header should be present")
        .to_str()
        .expect("Request ID should be valid UTF-8");

    // Verify it's a valid UUID
    assert!(
        Uuid::parse_str(request_id).is_ok(),
        "Request ID should be a valid UUID"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_request_id_preserved_from_client(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let client_request_id = Uuid::now_v7().to_string();

    let response = server
        .get("/health")
        .add_header(
            HeaderName::from_static("x-request-id"),
            client_request_id.clone(),
        )
        .await;

    let response_request_id = response
        .headers()
        .get("x-request-id")
        .expect("X-Request-ID header should be present")
        .to_str()
        .expect("Request ID should be valid UTF-8");

    assert_eq!(response_request_id, client_request_id);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_request_id_on_all_endpoints(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let endpoints = vec!["/health", "/health/ready", "/api/v1/info"];

    for endpoint in endpoints {
        let response = server.get(endpoint).await;
        assert!(
            response.headers().contains_key("x-request-id"),
            "Endpoint {} should have X-Request-ID header",
            endpoint
        );
    }
}

// ============================================================================
// CORS Headers Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_cors_headers_present(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let response = server
        .method(Method::OPTIONS, "/api/v1/info")
        .add_header(HeaderName::from_static("origin"), "https://example.com")
        .add_header(
            HeaderName::from_static("access-control-request-method"),
            "GET",
        )
        .await;

    // CORS middleware should respond to OPTIONS requests
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-methods")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_cors_allows_all_origins(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let response = server
        .get("/health")
        .add_header(HeaderName::from_static("origin"), "https://example.com")
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-origin")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_cors_exposes_request_id_header(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let response = server
        .method(Method::OPTIONS, "/health")
        .add_header(HeaderName::from_static("origin"), "https://example.com")
        .add_header(
            HeaderName::from_static("access-control-request-method"),
            "GET",
        )
        .await;

    // Verify that X-Request-ID is exposed to JavaScript
    if let Some(expose_headers) = response.headers().get("access-control-expose-headers") {
        let expose_headers_str = expose_headers
            .to_str()
            .expect("Header should be valid UTF-8")
            .to_lowercase();
        // The header should include x-request-id (exact format may vary)
        assert!(
            expose_headers_str.contains("x-request-id") || expose_headers_str.contains("*"),
            "CORS should expose X-Request-ID header"
        );
    }
}

// ============================================================================
// Error Response Format Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_404_error_format(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/nonexistent").await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);

    let body: ApiErrorResponse = response.json();
    // ErrorCode::NotFound serializes to "NOT_FOUND"
    let serialized = serde_json::to_string(&body.error.code).unwrap();
    assert!(serialized.contains("NOT_FOUND"));
    assert!(!body.error.message.is_empty());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_error_code_serialization_format(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/nonexistent").await;

    // Get raw JSON text to verify serialization format
    let text = response.text();

    // Verify error code is serialized as SCREAMING_SNAKE_CASE
    assert!(
        text.contains("\"code\":\"NOT_FOUND\""),
        "Error code should be in SCREAMING_SNAKE_CASE format"
    );
}

// ============================================================================
// OpenAPI Endpoint Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_openapi_spec_accessible(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/openapi.json").await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let spec: serde_json::Value = response.json();
    assert_eq!(spec["openapi"], "3.1.0");
    assert!(spec["paths"].is_object());
    assert!(spec["components"].is_object());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_openapi_spec_includes_health_endpoints(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/openapi.json").await;

    let spec: serde_json::Value = response.json();
    let paths = &spec["paths"];

    // Verify health endpoints are documented
    assert!(paths["/health"].is_object(), "/health should be documented");
    assert!(
        paths["/health/ready"].is_object(),
        "/health/ready should be documented"
    );
    assert!(
        paths["/api/v1/info"].is_object(),
        "/api/v1/info should be documented"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_openapi_spec_includes_error_schemas(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/openapi.json").await;

    let spec: serde_json::Value = response.json();
    let schemas = &spec["components"]["schemas"];

    // Verify error schemas are included
    assert!(
        schemas["ApiErrorResponse"].is_object(),
        "ApiErrorResponse schema should be present"
    );
    assert!(
        schemas["ApiError"].is_object(),
        "ApiError schema should be present"
    );
    assert!(
        schemas["ErrorCode"].is_object(),
        "ErrorCode schema should be present"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_redoc_ui_accessible(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/docs").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .expect("Content-Type header should be present")
        .to_str()
        .expect("Content-Type should be valid UTF-8")
        .to_lowercase();
    assert!(content_type.contains("text/html"));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_redoc_ui_contains_api_title(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server.get("/api/v1/docs").await;

    let body = response.text();

    // Verify ReDoc UI contains our API title
    assert!(
        body.contains("Kruxia Flow") || body.contains("kruxiaflow"),
        "ReDoc UI should contain API title"
    );
}

// ============================================================================
// Middleware Stack Integration Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_middleware_stack_applied_correctly(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let response = server
        .get("/health")
        .add_header(HeaderName::from_static("origin"), "https://example.com")
        .await;

    // Verify both middleware are applied
    assert!(
        response.headers().contains_key("x-request-id"),
        "Request ID middleware should be applied"
    );
    assert!(
        response
            .headers()
            .contains_key("access-control-allow-origin"),
        "CORS middleware should be applied"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_request_id_different_on_each_request(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let response1 = server.get("/health").await;
    let response2 = server.get("/health").await;

    let request_id1 = response1
        .headers()
        .get("x-request-id")
        .expect("First request should have X-Request-ID")
        .to_str()
        .expect("Request ID should be valid UTF-8");
    let request_id2 = response2
        .headers()
        .get("x-request-id")
        .expect("Second request should have X-Request-ID")
        .to_str()
        .expect("Request ID should be valid UTF-8");

    assert_ne!(
        request_id1, request_id2,
        "Each request should have a different request ID"
    );
}
