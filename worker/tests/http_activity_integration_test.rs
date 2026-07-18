use kruxiaflow_api::{routes::app_router, state::AppState};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_worker::{ActivityImpl, HttpRequestActivity};
use serde_json::json;
use sqlx::PgPool;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Generate test RSA private key
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

/// Generate test RSA public key
fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Create and start a real API server on a random port
async fn create_real_server(pool: PgPool) -> (String, PgPool, tokio::task::JoinHandle<()>) {
    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };

    let auth_service = PostgresAuthService::new(pool.clone(), auth_config)
        .expect("Failed to create test auth service");

    // Create test OAuth client
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "test_http_client",
        bcrypt::hash("test_http_secret", 4).unwrap(), // min cost: real hashing strength is not under test
        "Test HTTP Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test OAuth client");

    let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let shutdown_token = CancellationToken::new();

    let subscription_service = Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        activity_queue,
        event_source,
        workflow_storage,
        cache_service,
        subscription_service,
        shutdown_token,
    );
    let app = app_router(state);

    // Bind to a random available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to port");
    let addr = listener.local_addr().expect("Failed to get local address");
    let server_url = format!("http://{}", addr);

    // Spawn the server in the background
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Server failed to start");
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    (server_url, pool, handle)
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_get_request(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_get_with_query_params(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/api/v1/info", server_url),
        "query": {
            "format": "json"
        }
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_post_request_with_json_body(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "POST",
        "url": format!("{}/api/v1/oauth/token", server_url),
        "headers": {
            "Content-Type": "application/json"
        },
        "body": {
            "grant_type": "client_credentials",
            "client_id": "test_http_client",
            "client_secret": "test_http_secret"
        }
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    // Verify we got an access token in the response body
    let body = response.get("body").unwrap();
    assert!(body.get("access_token").is_some());

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_request_with_custom_headers(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url),
        "headers": {
            "User-Agent": "Kruxia Flow-Test/1.0",
            "X-Custom-Header": "test-value"
        }
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_head_request_excludes_body(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "HEAD",
        "url": format!("{}/health", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);
    // HEAD requests should not include body
    assert!(response.get("body").is_none() || response.get("body").unwrap().is_null());

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_request_include_body_false(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url),
        "include_body": false
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);
    // Body should be excluded when include_body is false
    assert!(response.get("body").is_none() || response.get("body").unwrap().is_null());

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_request_include_body_true(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/api/v1/info", server_url),
        "include_body": true
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);
    // Body should be included
    assert!(response.get("body").is_some() && !response.get("body").unwrap().is_null());

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_request_default_user_agent(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    // Just verify the request succeeded with default User-Agent
    // (we can't directly inspect request headers from the response in this test)

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_request_404_not_found(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/nonexistent-endpoint", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 404);
    assert_eq!(response.get("success").unwrap(), false);

    server_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_http_request_with_timeout(pool: PgPool) {
    let (server_url, _pool, server_handle) = create_real_server(pool).await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url),
        "timeout_seconds": 5
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    assert!(output_value.get("response").is_some());
    let response = output_value.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

// ============================================================================
// Gzip decompression tests
// Regression tests for: docs/bugs/2026-01-07-http-request-gzip-response-not-decompressed.md
// ============================================================================

/// Helper function to gzip compress data
fn gzip_compress(data: &[u8]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("Failed to write gzip data");
    encoder.finish().expect("Failed to finish gzip encoding")
}

#[tokio::test]
async fn test_http_gzip_compressed_json_response() {
    // This test verifies that gzip-compressed JSON responses are automatically
    // decompressed by the HTTP client. This was the root cause of the bug where
    // gzip binary data containing null bytes couldn't be stored in PostgreSQL JSON.

    let mock_server = MockServer::start().await;

    let json_body = r#"{"title":"Test Article","author":"Test Author","id":12345}"#;
    let compressed_body = gzip_compress(json_body.as_bytes());

    Mock::given(method("GET"))
        .and(path("/api/article"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(compressed_body)
                .insert_header("Content-Type", "application/json")
                .insert_header("Content-Encoding", "gzip"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/api/article", mock_server.uri())
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    // Verify the body was properly decompressed and parsed as JSON
    let body = response.get("body").unwrap();
    assert_eq!(body.get("title").unwrap(), "Test Article");
    assert_eq!(body.get("author").unwrap(), "Test Author");
    assert_eq!(body.get("id").unwrap(), 12345);
}

#[tokio::test]
async fn test_http_gzip_compressed_text_response() {
    // Test that gzip-compressed text/html responses are also decompressed

    let mock_server = MockServer::start().await;

    let html_body =
        "<html><head><title>Test Page</title></head><body><h1>Hello World</h1></body></html>";
    let compressed_body = gzip_compress(html_body.as_bytes());

    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(compressed_body)
                .insert_header("Content-Type", "text/html; charset=utf-8")
                .insert_header("Content-Encoding", "gzip"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/page", mock_server.uri())
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    // Verify the body was properly decompressed (stored as string since not JSON)
    let body = response.get("body").unwrap().as_str().unwrap();
    assert!(body.contains("<title>Test Page</title>"));
    assert!(body.contains("<h1>Hello World</h1>"));
}

#[tokio::test]
async fn test_http_uncompressed_json_response() {
    // Verify that uncompressed responses still work correctly

    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/data"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "ok", "count": 42}))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/api/data", mock_server.uri())
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    let body = response.get("body").unwrap();
    assert_eq!(body.get("status").unwrap(), "ok");
    assert_eq!(body.get("count").unwrap(), 42);
}

#[tokio::test]
async fn test_http_explicit_accept_encoding_identity() {
    // Test that explicit Accept-Encoding: identity header is respected
    // This was the documented workaround before the fix

    let mock_server = MockServer::start().await;

    // Server should return uncompressed when Accept-Encoding: identity is sent
    Mock::given(method("GET"))
        .and(path("/api/uncompressed"))
        .and(header("Accept-Encoding", "identity"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"compressed": false}))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/api/uncompressed", mock_server.uri()),
        "headers": {
            "Accept-Encoding": "identity"
        }
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);
    let body = response.get("body").unwrap();
    assert_eq!(body.get("compressed").unwrap(), false);
}

#[tokio::test]
async fn test_http_explicit_accept_encoding_gzip() {
    // Test that explicit Accept-Encoding: gzip header works with gzip responses

    let mock_server = MockServer::start().await;

    let json_body = r#"{"compressed":true,"data":"test data"}"#;
    let compressed_body = gzip_compress(json_body.as_bytes());

    Mock::given(method("GET"))
        .and(path("/api/compressed"))
        .and(header("Accept-Encoding", "gzip"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(compressed_body)
                .insert_header("Content-Type", "application/json")
                .insert_header("Content-Encoding", "gzip"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/api/compressed", mock_server.uri()),
        "headers": {
            "Accept-Encoding": "gzip"
        }
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);
    let body = response.get("body").unwrap();
    assert_eq!(body.get("compressed").unwrap(), true);
    assert_eq!(body.get("data").unwrap(), "test data");
}

#[tokio::test]
async fn test_http_gzip_response_with_unicode() {
    // Test that gzip-compressed responses with unicode characters are handled correctly
    // This verifies the decompression doesn't corrupt multi-byte UTF-8 characters

    let mock_server = MockServer::start().await;

    let json_body = r#"{"message":"Hello 世界! 🌍","emoji":"🎉"}"#;
    let compressed_body = gzip_compress(json_body.as_bytes());

    Mock::given(method("GET"))
        .and(path("/api/unicode"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(compressed_body)
                .insert_header("Content-Type", "application/json; charset=utf-8")
                .insert_header("Content-Encoding", "gzip"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/api/unicode", mock_server.uri())
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);

    let body = response.get("body").unwrap();
    assert_eq!(body.get("message").unwrap(), "Hello 世界! 🌍");
    assert_eq!(body.get("emoji").unwrap(), "🎉");
}

#[tokio::test]
async fn test_http_gzip_large_response() {
    // Test that larger gzip-compressed responses decompress correctly

    let mock_server = MockServer::start().await;

    // Create a JSON response with repeated data to make it larger
    let items: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            json!({
                "id": i,
                "name": format!("Item number {}", i),
                "description": "This is a longer description to make the response larger"
            })
        })
        .collect();

    let json_body = serde_json::to_string(&json!({"items": items})).unwrap();
    let compressed_body = gzip_compress(json_body.as_bytes());

    // Verify compression actually reduced the size
    assert!(compressed_body.len() < json_body.len());

    Mock::given(method("GET"))
        .and(path("/api/items"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(compressed_body)
                .insert_header("Content-Type", "application/json")
                .insert_header("Content-Encoding", "gzip"),
        )
        .mount(&mock_server)
        .await;

    let activity = HttpRequestActivity::new();
    let params = json!({
        "method": "GET",
        "url": format!("{}/api/items", mock_server.uri())
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let response = output_value.get("response").unwrap();

    assert_eq!(response.get("status").unwrap(), 200);

    let body = response.get("body").unwrap();
    let items = body.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 100);
    assert_eq!(items[0].get("id").unwrap(), 0);
    assert_eq!(items[99].get("id").unwrap(), 99);
}
