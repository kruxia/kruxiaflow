//! WebSocket integration tests for activity streaming.
//!
//! These tests verify WebSocket functionality including:
//! - Authentication via query parameter
//! - Connection management
//! - Message broadcasting
//! - Concurrent connections
//! - Graceful connection close

use bcrypt::hash;
use futures::StreamExt;
use serial_test::serial;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use streamflow_api::{AppState, AppStateBuild, app_router};
use streamflow_core::events::PostgresEventSource;
use streamflow_core::queue::{PostgresQueue, QueueConfig};
use streamflow_oauth::{AuthConfig, PostgresAuthService};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Load test RSA keys
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to create test AppState
async fn setup_test_state() -> AppState {
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

    // Create test client
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "ws-test-client",
        hash("ws-test-secret", bcrypt::DEFAULT_COST).unwrap(),
        "WebSocket Test Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test client");

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(streamflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(streamflow_core::cache::NoOpCache::new());

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        cache_service,
        CancellationToken::new(),
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-01-15T00:00:00Z".to_string(),
            git_hash: "wstest123".to_string(),
        },
        vec!["workflows".to_string(), "websockets".to_string()],
    )
}

/// Start a test server and return its address
async fn start_test_server(state: AppState) -> SocketAddr {
    let app = app_router(state);

    // Bind to a random available port
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("Failed to get local address");

    // Spawn server in background
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    addr
}

/// Get a valid access token from the server
async fn get_valid_token(addr: SocketAddr) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/oauth/token", addr))
        .json(&serde_json::json!({
            "grant_type": "client_credentials",
            "client_id": "ws-test-client",
            "client_secret": "ws-test-secret"
        }))
        .send()
        .await
        .expect("Failed to get token");

    let body: serde_json::Value = response
        .json()
        .await
        .expect("Failed to parse token response");
    body["access_token"]
        .as_str()
        .expect("No access_token in response")
        .to_string()
}

// ============================================================================
// Authentication Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_websocket_rejects_missing_token() {
    let state = setup_test_state().await;
    let addr = start_test_server(state).await;

    let activity_id = Uuid::now_v7();
    let url = format!("ws://{}/api/v1/activities/{}/stream", addr, activity_id);

    // Attempt connection without token
    let result = connect_async(&url).await;

    // Should fail with HTTP error (401 Unauthorized)
    assert!(result.is_err(), "Connection should fail without token");
}

#[tokio::test]
#[serial]
async fn test_websocket_rejects_invalid_token() {
    let state = setup_test_state().await;
    let addr = start_test_server(state).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token=invalid_token_here",
        addr, activity_id
    );

    // Attempt connection with invalid token
    let result = connect_async(&url).await;

    // Should fail with HTTP error (401 Unauthorized)
    assert!(result.is_err(), "Connection should fail with invalid token");
}

#[tokio::test]
#[serial]
async fn test_websocket_accepts_valid_token() {
    let state = setup_test_state().await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Should connect successfully
    let result = timeout(Duration::from_secs(5), connect_async(&url)).await;

    assert!(result.is_ok(), "Connection should not timeout");
    let (ws_stream, response) = result
        .unwrap()
        .expect("Connection should succeed with valid token");

    assert_eq!(
        response.status().as_u16(),
        101, // SWITCHING_PROTOCOLS
        "Should upgrade to WebSocket"
    );

    // Clean up
    drop(ws_stream);
}

// ============================================================================
// Broadcasting Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_websocket_receives_broadcast_token() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect WebSocket");

    // Give connection time to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast a token message
    let msg = streamflow_api::StreamMessage::token("Hello", 0);
    let count = connection_manager.broadcast(activity_id, msg).await;
    assert_eq!(count, 1, "Should have 1 connection");

    // Receive the message
    let received = timeout(Duration::from_secs(2), ws_stream.next())
        .await
        .expect("Should not timeout")
        .expect("Should receive message")
        .expect("Message should be valid");

    match received {
        Message::Text(text) => {
            let json: serde_json::Value =
                serde_json::from_str(&text).expect("Should be valid JSON");
            assert_eq!(json["type"], "token");
            assert_eq!(json["text"], "Hello");
            assert_eq!(json["index"], 0);
        }
        _ => panic!("Expected text message, got {:?}", received),
    }
}

#[tokio::test]
#[serial]
async fn test_websocket_receives_complete_message() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect WebSocket");

    // Give connection time to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast a complete message
    let msg = streamflow_api::StreamMessage::complete(
        activity_id,
        serde_json::json!({"output": "Final result"}),
    );
    connection_manager.broadcast(activity_id, msg).await;

    // Receive the message
    let received = timeout(Duration::from_secs(2), ws_stream.next())
        .await
        .expect("Should not timeout")
        .expect("Should receive message")
        .expect("Message should be valid");

    match received {
        Message::Text(text) => {
            let json: serde_json::Value =
                serde_json::from_str(&text).expect("Should be valid JSON");
            assert_eq!(json["type"], "complete");
            assert_eq!(json["activity_id"], activity_id.to_string());
            assert_eq!(json["result"]["output"], "Final result");
        }
        _ => panic!("Expected text message"),
    }
}

#[tokio::test]
#[serial]
async fn test_websocket_receives_error_message() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect WebSocket");

    // Give connection time to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast an error message
    let msg = streamflow_api::StreamMessage::error(activity_id, "LLM provider timeout");
    connection_manager.broadcast(activity_id, msg).await;

    // Receive the message
    let received = timeout(Duration::from_secs(2), ws_stream.next())
        .await
        .expect("Should not timeout")
        .expect("Should receive message")
        .expect("Message should be valid");

    match received {
        Message::Text(text) => {
            let json: serde_json::Value =
                serde_json::from_str(&text).expect("Should be valid JSON");
            assert_eq!(json["type"], "error");
            assert_eq!(json["activity_id"], activity_id.to_string());
            assert_eq!(json["error"], "LLM provider timeout");
        }
        _ => panic!("Expected text message"),
    }
}

// ============================================================================
// Connection Management Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_websocket_multiple_connections_same_activity() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect multiple clients
    let (mut ws1, _) = connect_async(&url).await.expect("Failed to connect ws1");
    let (mut ws2, _) = connect_async(&url).await.expect("Failed to connect ws2");
    let (mut ws3, _) = connect_async(&url).await.expect("Failed to connect ws3");

    // Give connections time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify connection count
    let count = connection_manager.connection_count(activity_id).await;
    assert_eq!(count, 3, "Should have 3 connections");

    // Broadcast a message
    let msg = streamflow_api::StreamMessage::token("broadcast", 0);
    let delivered = connection_manager.broadcast(activity_id, msg).await;
    assert_eq!(delivered, 3, "Should deliver to 3 connections");

    // All should receive the message
    for (i, ws) in [&mut ws1, &mut ws2, &mut ws3].iter_mut().enumerate() {
        let received = timeout(Duration::from_secs(2), ws.next())
            .await
            .expect(&format!("ws{} should not timeout", i + 1))
            .expect(&format!("ws{} should receive message", i + 1))
            .expect(&format!("ws{} message should be valid", i + 1));

        match received {
            Message::Text(text) => {
                assert!(
                    text.contains("broadcast"),
                    "ws{} should receive broadcast",
                    i + 1
                );
            }
            _ => panic!("Expected text message for ws{}", i + 1),
        }
    }
}

#[tokio::test]
#[serial]
async fn test_websocket_different_activities_isolated() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity1 = Uuid::now_v7();
    let activity2 = Uuid::now_v7();

    let url1 = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity1, token
    );
    let url2 = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity2, token
    );

    // Connect to different activities
    let (mut ws1, _) = connect_async(&url1).await.expect("Failed to connect ws1");
    let (mut ws2, _) = connect_async(&url2).await.expect("Failed to connect ws2");

    // Give connections time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Broadcast to activity1 only
    let msg = streamflow_api::StreamMessage::token("for-activity-1", 0);
    connection_manager.broadcast(activity1, msg).await;

    // ws1 should receive it
    let received = timeout(Duration::from_secs(2), ws1.next())
        .await
        .expect("ws1 should not timeout")
        .expect("ws1 should receive message")
        .expect("ws1 message should be valid");

    match received {
        Message::Text(text) => {
            assert!(text.contains("for-activity-1"));
        }
        _ => panic!("Expected text message"),
    }

    // ws2 should NOT receive it (different activity)
    let result = timeout(Duration::from_millis(200), ws2.next()).await;
    assert!(
        result.is_err(),
        "ws2 should timeout (no message for its activity)"
    );
}

#[tokio::test]
#[serial]
async fn test_websocket_close_all_connections() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect multiple clients
    let (_ws1, _) = connect_async(&url).await.expect("Failed to connect ws1");
    let (_ws2, _) = connect_async(&url).await.expect("Failed to connect ws2");

    // Give connections time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(connection_manager.connection_count(activity_id).await, 2);

    // Close all connections
    connection_manager.close_all(activity_id).await;

    // Connection count should be 0
    assert_eq!(connection_manager.connection_count(activity_id).await, 0);
}

#[tokio::test]
#[serial]
async fn test_websocket_cleanup_on_client_disconnect() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect WebSocket");

    // Give connection time to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(connection_manager.connection_count(activity_id).await, 1);

    // Client sends close frame
    ws_stream
        .close(None)
        .await
        .expect("Failed to close WebSocket");

    // Give server time to process close
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connection should be cleaned up
    // Note: Cleanup happens when we try to broadcast and the channel is closed
    let msg = streamflow_api::StreamMessage::token("test", 0);
    connection_manager.broadcast(activity_id, msg).await;

    // After failed broadcast, connection should be removed
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        connection_manager.connection_count(activity_id).await,
        0,
        "Connection should be cleaned up after client disconnect"
    );
}

// ============================================================================
// Concurrent Connections Test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_websocket_many_concurrent_connections() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect many clients (100 for reasonable test time)
    let num_connections = 100;
    let mut connections = Vec::with_capacity(num_connections);

    for _ in 0..num_connections {
        let (ws, _) = connect_async(&url).await.expect("Failed to connect");
        connections.push(ws);
    }

    // Give connections time to register
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify connection count
    let count = connection_manager.connection_count(activity_id).await;
    assert_eq!(
        count, num_connections,
        "Should have {} connections",
        num_connections
    );

    // Broadcast a message
    let msg = streamflow_api::StreamMessage::token("concurrent-test", 0);
    let delivered = connection_manager.broadcast(activity_id, msg).await;
    assert_eq!(
        delivered, num_connections,
        "Should deliver to all {} connections",
        num_connections
    );

    // Verify all connections receive the message
    for (i, ws) in connections.iter_mut().enumerate() {
        let result = timeout(Duration::from_secs(5), ws.next()).await;
        assert!(result.is_ok(), "Connection {} should not timeout", i);
        let msg = result.unwrap();
        assert!(msg.is_some(), "Connection {} should receive message", i);
        let msg = msg.unwrap();
        assert!(msg.is_ok(), "Connection {} message should be valid", i);

        match msg.unwrap() {
            Message::Text(text) => {
                assert!(
                    text.contains("concurrent-test"),
                    "Connection {} should receive the broadcast",
                    i
                );
            }
            other => panic!("Connection {} expected text message, got {:?}", i, other),
        }
    }
}

// ============================================================================
// Message Sequence Test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_websocket_message_ordering() {
    let state = setup_test_state().await;
    let connection_manager = state.connection_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let activity_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/activities/{}/stream?token={}",
        addr, activity_id, token
    );

    // Connect
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect WebSocket");

    // Give connection time to register
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send multiple tokens in order
    for i in 0..10 {
        let msg = streamflow_api::StreamMessage::token(format!("token-{}", i), i);
        connection_manager.broadcast(activity_id, msg).await;
    }

    // Receive all messages and verify order
    for expected_index in 0..10 {
        let received = timeout(Duration::from_secs(2), ws_stream.next())
            .await
            .expect("Should not timeout")
            .expect("Should receive message")
            .expect("Message should be valid");

        match received {
            Message::Text(text) => {
                let json: serde_json::Value =
                    serde_json::from_str(&text).expect("Should be valid JSON");
                assert_eq!(json["type"], "token");
                assert_eq!(
                    json["index"], expected_index,
                    "Messages should be received in order"
                );
                assert_eq!(
                    json["text"],
                    format!("token-{}", expected_index),
                    "Token text should match"
                );
            }
            _ => panic!("Expected text message"),
        }
    }
}
