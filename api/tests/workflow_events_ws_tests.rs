//! Integration tests for the workflow events WebSocket endpoint.
//!
//! Tests cover:
//! - Authentication (missing/invalid token)
//! - Connection establishment with valid token
//! - Workflow ID filtering
//! - Event type filtering
//! - Invalid parameter handling
//! - Live event broadcast
//! - Replay from event ID
//! - Server shutdown close frame
//! - Client close

use bcrypt::hash;
use futures::StreamExt;
use kruxiaflow_api::{AppState, AppStateBuild, app_router};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::{NewWorkflowEvent, PostgresEventSource, WorkflowEventType};
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serial_test::serial;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ============================================================================
// Test Infrastructure
// ============================================================================

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
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

fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

async fn setup_test_state_with_token(
    shutdown_token: CancellationToken,
) -> AppState {
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

    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "wfe-test-client",
        hash("wfe-test-secret", bcrypt::DEFAULT_COST).unwrap(),
        "Workflow Events Test Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test client");

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
        shutdown_token,
        "0.1.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-01-15T00:00:00Z".to_string(),
            git_hash: "wfetest123".to_string(),
        },
        vec!["workflows".to_string(), "workflow_events".to_string()],
    )
}

async fn start_test_server(state: AppState) -> SocketAddr {
    let app = app_router(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("Failed to get local address");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    addr
}

async fn get_valid_token(addr: SocketAddr) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/api/v1/oauth/token", addr))
        .json(&serde_json::json!({
            "grant_type": "client_credentials",
            "client_id": "wfe-test-client",
            "client_secret": "wfe-test-secret"
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
async fn test_workflow_events_ws_rejects_missing_token() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws", addr);
    let result = connect_async(&url).await;

    // Should fail during the HTTP upgrade (401)
    assert!(result.is_err(), "Expected connection to be rejected");
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_rejects_invalid_token() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token=bad-jwt", addr);
    let result = connect_async(&url).await;

    assert!(result.is_err(), "Expected connection to be rejected");
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_connects_with_valid_token() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token={}", addr, token);
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with valid token");

    // Close cleanly
    ws_stream.close(None).await.ok();
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_invalid_workflow_id() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id=not-a-uuid",
        addr, token
    );
    let result = connect_async(&url).await;

    assert!(result.is_err(), "Expected rejection for invalid workflow_id");
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_invalid_event_type() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&event_type=BadType",
        addr, token
    );
    let result = connect_async(&url).await;

    assert!(result.is_err(), "Expected rejection for invalid event_type");
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_invalid_from_event_id() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&from_event_id=not-a-uuid",
        addr, token
    );
    let result = connect_async(&url).await;

    assert!(
        result.is_err(),
        "Expected rejection for invalid from_event_id"
    );
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_valid_workflow_id_filter() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let wf_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id={}",
        addr, token, wf_id
    );
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with workflow_id filter");

    ws_stream.close(None).await.ok();
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_valid_event_type_filter() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&event_type=WorkflowCreated,ActivityCompleted",
        addr, token
    );
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with event_type filter");

    ws_stream.close(None).await.ok();
}

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_multiple_workflow_ids() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let wf1 = Uuid::now_v7();
    let wf2 = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id={},{}",
        addr, token, wf1, wf2
    );
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with multiple workflow_ids");

    ws_stream.close(None).await.ok();
}

// ============================================================================
// Live Event Broadcast Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_receives_broadcast_event() {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(shutdown_token.clone()).await;
    let manager = state.workflow_event_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let workflow_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id={}",
        addr, token, workflow_id
    );
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect");

    // Give the subscription time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Construct a WorkflowEvent and broadcast directly through the manager
    // (avoids polling stale events from the shared database)
    let event = kruxiaflow_core::events::WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::WorkflowCreated,
        activity_key: None,
        payload: serde_json::json!({"test": true}),
        timestamp: chrono::Utc::now(),
        iteration: None,
    };
    manager.broadcast(&event).await;

    // Receive the event on the WebSocket
    let received = timeout(Duration::from_secs(5), ws_stream.next())
        .await
        .expect("Timeout waiting for event")
        .expect("Stream ended")
        .expect("WebSocket error");

    match received {
        Message::Text(text) => {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(json["type"], "event");
            assert_eq!(json["workflow_id"], workflow_id.to_string());
            assert_eq!(json["event_type"], "WorkflowCreated");
        }
        other => panic!("Expected text message, got {:?}", other),
    }

    ws_stream.close(None).await.ok();
    shutdown_token.cancel();
}

// ============================================================================
// Server Shutdown Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_shutdown_sends_close_frame() {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(shutdown_token.clone()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token={}", addr, token);
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect");

    // Give the subscription time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Trigger shutdown
    shutdown_token.cancel();

    // We should receive an error message followed by a close frame
    let mut received_shutdown_error = false;
    let mut received_close = false;

    while let Ok(Some(msg_result)) =
        timeout(Duration::from_secs(5), ws_stream.next()).await
    {
        match msg_result {
            Ok(Message::Text(text)) => {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                if json["type"] == "error" && json["code"] == 1001 {
                    received_shutdown_error = true;
                }
            }
            Ok(Message::Close(_)) => {
                received_close = true;
                break;
            }
            Err(_) => break,
            _ => {}
        }
    }

    assert!(
        received_shutdown_error || received_close,
        "Expected shutdown notification"
    );
}

// ============================================================================
// Replay Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_replay_from_event_id() {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(shutdown_token.clone()).await;
    let event_source = state.event_source.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    // Publish some events first
    let workflow_id = Uuid::now_v7();
    let mut event_ids = Vec::new();
    for i in 0..3 {
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::WorkflowUpdated,
                activity_key: Some(format!("step_{}", i)),
                payload: serde_json::json!({"index": i}),
                iteration: None,
            })
            .await
            .expect("Failed to publish event");
    }

    // Poll to get the event IDs
    let events = event_source
        .poll("replay-test-consumer")
        .await
        .expect("Failed to poll");

    // Consume them to record position
    if let Some(last) = events.last() {
        event_source
            .update_position("replay-test-consumer", last.id)
            .await
            .ok();
    }

    for e in &events {
        event_ids.push(e.id);
    }

    // If we have at least 2 events, replay from the first one
    if event_ids.len() >= 2 {
        let from_id = event_ids[0];
        let url = format!(
            "ws://{}/api/v1/workflow_events/ws?token={}&from_event_id={}",
            addr, token, from_id
        );
        let (mut ws_stream, _) = connect_async(&url)
            .await
            .expect("Failed to connect with replay");

        // Should receive replayed events (those after from_id)
        let mut replayed = 0;
        while let Ok(Some(msg_result)) =
            timeout(Duration::from_secs(3), ws_stream.next()).await
        {
            match msg_result {
                Ok(Message::Text(text)) => {
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if json["type"] == "event" {
                        replayed += 1;
                    }
                }
                _ => break,
            }
            // Stop after we've received what we expect
            if replayed >= 2 {
                break;
            }
        }

        assert!(replayed >= 1, "Expected at least 1 replayed event, got {}", replayed);
        ws_stream.close(None).await.ok();
    }

    shutdown_token.cancel();
}

// ============================================================================
// Client Close Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_client_close() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let manager = state.workflow_event_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token={}", addr, token);
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect");

    // Give subscription time to register
    tokio::time::sleep(Duration::from_millis(100)).await;
    let count_before = manager.subscriber_count().await;
    assert!(count_before >= 1, "Expected at least 1 subscriber");

    // Close the WebSocket from the client side
    ws_stream.close(None).await.ok();

    // Give the server time to clean up
    tokio::time::sleep(Duration::from_millis(200)).await;
    let count_after = manager.subscriber_count().await;
    assert!(
        count_after < count_before,
        "Expected subscriber count to decrease after client close"
    );
}

// ============================================================================
// Empty Filter Tests (no workflow_id or event_type)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_workflow_events_ws_empty_filters() {
    let state = setup_test_state_with_token(CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    // Empty string filters should be treated as "no filter"
    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id=&event_type=&from_event_id=",
        addr, token
    );
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with empty filters");

    ws_stream.close(None).await.ok();
}
