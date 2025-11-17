use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use streamflow_api::{routes::app_router, state::AppState};
use streamflow_core::events::PostgresEventSource;
use streamflow_core::queue::{PostgresQueue, QueueConfig};
use streamflow_oauth::{AuthConfig, PostgresAuthService};
use streamflow_worker::{ActivityImpl, HttpRequestActivity};
use tokio_util::sync::CancellationToken;

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

/// Create and start a real API server on a random port
async fn create_real_server() -> (String, PgPool, tokio::task::JoinHandle<()>) {
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

    // Create test OAuth client
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "test_http_client",
        bcrypt::hash("test_http_secret", bcrypt::DEFAULT_COST).unwrap(),
        "Test HTTP Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test OAuth client");

    let activity_queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let shutdown_token = CancellationToken::new();

    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        activity_queue,
        event_source,
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

#[tokio::test]
#[serial]
async fn test_http_get_request() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_get_with_query_params() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/api/v1/info", server_url),
        "query": {
            "format": "json"
        }
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_post_request_with_json_body() {
    let (server_url, _pool, server_handle) = create_real_server().await;
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

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    // Verify we got an access token in the response body
    let body = response.get("body").unwrap();
    assert!(body.get("access_token").is_some());

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_request_with_custom_headers() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url),
        "headers": {
            "User-Agent": "StreamFlow-Test/1.0",
            "X-Custom-Header": "test-value"
        }
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_head_request_excludes_body() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "HEAD",
        "url": format!("{}/health", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);
    // HEAD requests should not include body
    assert!(response.get("body").is_none() || response.get("body").unwrap().is_null());

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_request_include_body_false() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url),
        "include_body": false
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);
    // Body should be excluded when include_body is false
    assert!(response.get("body").is_none() || response.get("body").unwrap().is_null());

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_request_include_body_true() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/api/v1/info", server_url),
        "include_body": true
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);
    // Body should be included
    assert!(response.get("body").is_some() && !response.get("body").unwrap().is_null());

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_request_default_user_agent() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    // Just verify the request succeeded with default User-Agent
    // (we can't directly inspect request headers from the response in this test)

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_request_404_not_found() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/nonexistent-endpoint", server_url)
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 404);
    assert_eq!(response.get("success").unwrap(), false);

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_http_request_with_timeout() {
    let (server_url, _pool, server_handle) = create_real_server().await;
    let activity = HttpRequestActivity::new();

    let params = json!({
        "method": "GET",
        "url": format!("{}/health", server_url),
        "timeout_seconds": 5
    });

    let result = activity.execute(params).await.unwrap();

    assert!(result.get("response").is_some());
    let response = result.get("response").unwrap();
    assert_eq!(response.get("status").unwrap(), 200);
    assert_eq!(response.get("success").unwrap(), true);

    server_handle.abort();
}
