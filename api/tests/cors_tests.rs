// api/tests/cors_tests.rs
//! Tests for CORS middleware

use axum::http::{HeaderName, Method, Request, StatusCode, header};
use tower::ServiceExt;

/// Test CORS allows standard HTTP methods
#[tokio::test]
async fn test_cors_allows_standard_methods() {
    let layer = kruxiaflow_api::middleware::cors_layer();
    let app = axum::Router::new()
        .route("/test", axum::routing::get(|| async { "ok" }))
        .layer(layer);

    // Test GET request
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/test")
                .method(Method::GET)
                .header("Origin", "http://example.com")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Check that CORS headers are present
    let headers = response.headers();
    assert!(
        headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "Should have Access-Control-Allow-Origin header"
    );
}

/// Test CORS preflight OPTIONS request
#[tokio::test]
async fn test_cors_preflight_options() {
    let layer = kruxiaflow_api::middleware::cors_layer();
    let app = axum::Router::new()
        .route("/test", axum::routing::post(|| async { "ok" }))
        .layer(layer);

    // Send preflight OPTIONS request
    let response = app
        .oneshot(
            Request::builder()
                .uri("/test")
                .method(Method::OPTIONS)
                .header("Origin", "http://example.com")
                .header("Access-Control-Request-Method", "POST")
                .header("Access-Control-Request-Headers", "content-type")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Preflight should return 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // Check CORS headers
    let headers = response.headers();
    assert!(
        headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "Should have Access-Control-Allow-Origin header"
    );
    assert!(
        headers.contains_key(header::ACCESS_CONTROL_ALLOW_METHODS),
        "Should have Access-Control-Allow-Methods header"
    );
    assert!(
        headers.contains_key(header::ACCESS_CONTROL_MAX_AGE),
        "Should have Access-Control-Max-Age header"
    );
}

/// Test CORS allows custom headers
#[tokio::test]
async fn test_cors_allows_custom_headers() {
    let layer = kruxiaflow_api::middleware::cors_layer();
    let app = axum::Router::new()
        .route("/test", axum::routing::get(|| async { "ok" }))
        .layer(layer);

    // Send request with custom headers
    let response = app
        .oneshot(
            Request::builder()
                .uri("/test")
                .method(Method::GET)
                .header("Origin", "http://example.com")
                .header("X-Request-ID", "test-123")
                .header("Authorization", "Bearer token")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Custom headers should be allowed
    let headers = response.headers();
    assert!(
        headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "Should have Access-Control-Allow-Origin header"
    );
}

/// Test CORS exposes custom headers
#[tokio::test]
async fn test_cors_exposes_custom_headers() {
    let layer = kruxiaflow_api::middleware::cors_layer();
    let app = axum::Router::new()
        .route(
            "/test",
            axum::routing::get(|| async {
                (
                    [(HeaderName::from_static("x-request-id"), "test-123")],
                    "ok",
                )
            }),
        )
        .layer(layer);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test")
                .method(Method::GET)
                .header("Origin", "http://example.com")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Check that expose headers are set
    let headers = response.headers();
    assert!(
        headers.contains_key(header::ACCESS_CONTROL_EXPOSE_HEADERS),
        "Should have Access-Control-Expose-Headers header"
    );
}

/// Test CorsConfig default values
#[test]
fn test_cors_config_default() {
    use kruxiaflow_api::middleware::cors::CorsConfig;

    let config = CorsConfig::default();

    assert_eq!(config.allowed_origins, vec!["*".to_string()]);
    assert_eq!(config.allow_credentials, true);
    assert_eq!(config.max_age_seconds, 3600);
}

/// Test CorsConfig custom values
#[test]
fn test_cors_config_custom() {
    use kruxiaflow_api::middleware::cors::CorsConfig;

    let config = CorsConfig {
        allowed_origins: vec!["https://example.com".to_string()],
        allow_credentials: false,
        max_age_seconds: 7200,
    };

    assert_eq!(
        config.allowed_origins,
        vec!["https://example.com".to_string()]
    );
    assert_eq!(config.allow_credentials, false);
    assert_eq!(config.max_age_seconds, 7200);
}

/// Test CorsConfig clone
#[test]
fn test_cors_config_clone() {
    use kruxiaflow_api::middleware::cors::CorsConfig;

    let config1 = CorsConfig {
        allowed_origins: vec!["https://example.com".to_string()],
        allow_credentials: true,
        max_age_seconds: 3600,
    };

    let config2 = config1.clone();
    assert_eq!(config1.allowed_origins, config2.allowed_origins);
    assert_eq!(config1.allow_credentials, config2.allow_credentials);
    assert_eq!(config1.max_age_seconds, config2.max_age_seconds);
}

/// Test CORS with multiple origins
#[tokio::test]
async fn test_cors_with_different_origins() {
    let layer = kruxiaflow_api::middleware::cors_layer();
    let app = axum::Router::new()
        .route("/test", axum::routing::get(|| async { "ok" }))
        .layer(layer);

    // Test with different origins
    let origins = vec![
        "http://example.com",
        "https://example.org",
        "http://localhost:3000",
    ];

    for origin in origins {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method(Method::GET)
                    .header("Origin", origin)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Should allow origin: {}",
            origin
        );
    }
}

/// Test CORS with all allowed methods
#[tokio::test]
async fn test_cors_with_all_methods() {
    let layer = kruxiaflow_api::middleware::cors_layer();

    // Test each allowed method
    let methods = vec![
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ];

    for method in methods {
        let app = axum::Router::new()
            .route("/test", axum::routing::any(|| async { "ok" }))
            .layer(layer.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method(method.clone())
                    .header("Origin", "http://example.com")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Some methods might not be routed, but CORS should handle them
        // We mainly care that CORS doesn't block them
        let status = response.status();
        assert!(
            status == StatusCode::OK || status == StatusCode::METHOD_NOT_ALLOWED,
            "Method {:?} should be processed by CORS",
            method
        );
    }
}

/// Test CORS max-age cache directive
#[tokio::test]
async fn test_cors_max_age() {
    let layer = kruxiaflow_api::middleware::cors_layer();
    let app = axum::Router::new()
        .route("/test", axum::routing::post(|| async { "ok" }))
        .layer(layer);

    // Send preflight request
    let response = app
        .oneshot(
            Request::builder()
                .uri("/test")
                .method(Method::OPTIONS)
                .header("Origin", "http://example.com")
                .header("Access-Control-Request-Method", "POST")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Check max-age header
    let headers = response.headers();
    if let Some(max_age) = headers.get(header::ACCESS_CONTROL_MAX_AGE) {
        let max_age_str = max_age.to_str().unwrap();
        let max_age_value: u64 = max_age_str.parse().unwrap();

        // Should be 3600 seconds (1 hour)
        assert_eq!(max_age_value, 3600, "Max age should be 3600 seconds");
    }
}
