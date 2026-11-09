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

fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

async fn setup_test_state_with_token(pool: PgPool, shutdown_token: CancellationToken) -> AppState {
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
        hash("wfe-test-secret", 4).unwrap(), // min cost: real hashing strength is not under test
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_rejects_missing_token(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
    let addr = start_test_server(state).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws", addr);
    let result = connect_async(&url).await;

    // Should fail during the HTTP upgrade (401)
    assert!(result.is_err(), "Expected connection to be rejected");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_rejects_invalid_token(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
    let addr = start_test_server(state).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token=bad-jwt", addr);
    let result = connect_async(&url).await;

    assert!(result.is_err(), "Expected connection to be rejected");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_connects_with_valid_token(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token={}", addr, token);
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with valid token");

    // Close cleanly
    ws_stream.close(None).await.ok();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_invalid_workflow_id(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id=not-a-uuid",
        addr, token
    );
    let result = connect_async(&url).await;

    assert!(
        result.is_err(),
        "Expected rejection for invalid workflow_id"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_invalid_event_type(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&event_type=BadType",
        addr, token
    );
    let result = connect_async(&url).await;

    assert!(result.is_err(), "Expected rejection for invalid event_type");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_invalid_from_event_id(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_valid_workflow_id_filter(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_valid_event_type_filter(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_multiple_workflow_ids(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_receives_broadcast_event(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(pool, shutdown_token.clone()).await;
    let manager = state.workflow_event_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let workflow_id = Uuid::now_v7();
    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&workflow_id={}",
        addr, token, workflow_id
    );
    let (mut ws_stream, _) = connect_async(&url).await.expect("Failed to connect");

    // Give the subscription time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Construct a WorkflowEvent and broadcast directly through the manager
    // (avoids polling stale events from the shared database)
    let event = kruxiaflow_core::events::WorkflowEvent {
        id: Uuid::now_v7(),
        position: 0,
        tx: 0,
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_shutdown_sends_close_frame(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(pool, shutdown_token.clone()).await;
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token={}", addr, token);
    let (mut ws_stream, _) = connect_async(&url).await.expect("Failed to connect");

    // Give the subscription time to register
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Trigger shutdown
    shutdown_token.cancel();

    // We should receive an error message followed by a close frame
    let mut received_shutdown_error = false;
    let mut received_close = false;

    while let Ok(Some(msg_result)) = timeout(Duration::from_secs(5), ws_stream.next()).await {
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

/// Wait until every committed event is deliverable: `poll` and replay only
/// return events below the instance's oldest open transaction, and other
/// tests' transactions on the same instance can hold that back briefly
async fn settle_events(pool: &PgPool) {
    for _ in 0..2000 {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workflow_events \
             WHERE tx >= pg_snapshot_xmin(pg_current_snapshot()))",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(false);
        if !blocked {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Collect `event` messages until `count` arrive or the stream goes quiet
async fn receive_events<S>(
    ws_stream: &mut S,
    count: usize,
    quiet: Duration,
) -> Vec<serde_json::Value>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut received = Vec::new();
    while received.len() < count {
        match timeout(quiet, ws_stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                if json["type"] == "event" {
                    received.push(json);
                }
            }
            _ => break,
        }
    }
    received
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_replay_from_event_id(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(pool.clone(), shutdown_token.clone()).await;
    let event_source = state.event_source.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    // Publish some events first
    let workflow_id = Uuid::now_v7();
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
    settle_events(&pool).await;
    let events = event_source
        .poll("replay-test-consumer")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 3);

    // Replay from the first one: the other two, in stream order
    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&from_event_id={}",
        addr, token, events[0].id
    );
    let (mut ws_stream, _) = connect_async(&url)
        .await
        .expect("Failed to connect with replay");

    let replayed = receive_events(&mut ws_stream, 2, Duration::from_secs(3)).await;
    let replayed_ids: Vec<String> = replayed
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        replayed_ids,
        vec![events[1].id.to_string(), events[2].id.to_string()]
    );
    assert_eq!(replayed[0]["position"], events[1].position);
    assert_eq!(replayed[1]["position"], events[2].position);

    ws_stream.close(None).await.ok();
    shutdown_token.cancel();
}

/// An event that commits after a later one is replayed in commit-safe order
#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_replay_includes_late_commit(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(pool.clone(), shutdown_token.clone()).await;
    let event_source = state.event_source.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let publish = |workflow_id: Uuid| {
        let event_source = event_source.clone();
        async move {
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::WorkflowCreated,
                    activity_key: None,
                    payload: serde_json::json!({}),
                    iteration: None,
                })
                .await
                .unwrap();
            workflow_id
        }
    };
    let id_of = |workflow_id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM workflow_events WHERE workflow_id = $1")
                .bind(workflow_id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    let e0 = id_of(publish(Uuid::now_v7()).await).await;

    // A inserts first and commits last
    let mut a = pool.begin().await.unwrap();
    let a_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workflow_events (workflow_id, event_type, payload) \
         VALUES ($1, 'WorkflowCreated', '{}') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&mut *a)
    .await
    .unwrap();
    let b_id = id_of(publish(Uuid::now_v7()).await).await;
    a.commit().await.unwrap();
    settle_events(&pool).await;

    for (from, expected) in [(e0, vec![a_id, b_id]), (a_id, vec![b_id])] {
        let url = format!(
            "ws://{}/api/v1/workflow_events/ws?token={}&from_event_id={}",
            addr, token, from
        );
        let (mut ws_stream, _) = connect_async(&url).await.expect("connect");
        let replayed = receive_events(&mut ws_stream, expected.len(), Duration::from_secs(3)).await;
        let ids: Vec<String> = replayed
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_string())
            .collect();
        let expected: Vec<String> = expected.iter().map(Uuid::to_string).collect();
        assert_eq!(ids, expected, "replay from {from}");
        ws_stream.close(None).await.ok();
    }

    shutdown_token.cancel();
}

/// An unknown from_event_id yields an empty replay; the connection stays
/// open and live events still arrive
#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_replay_unknown_id_streams_live(pool: PgPool) {
    let shutdown_token = CancellationToken::new();
    let state = setup_test_state_with_token(pool.clone(), shutdown_token.clone()).await;
    let event_source = state.event_source.clone();
    let manager = state.workflow_event_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let workflow_id = Uuid::now_v7();
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: serde_json::json!({}),
            iteration: None,
        })
        .await
        .unwrap();
    settle_events(&pool).await;

    let url = format!(
        "ws://{}/api/v1/workflow_events/ws?token={}&from_event_id={}",
        addr,
        token,
        Uuid::now_v7()
    );
    let (mut ws_stream, _) = connect_async(&url).await.expect("connect");
    assert!(
        receive_events(&mut ws_stream, 1, Duration::from_millis(500))
            .await
            .is_empty(),
        "unknown from_event_id must not replay anything"
    );

    // Live delivery still works
    let event = event_source
        .poll("unknown-id-test")
        .await
        .unwrap()
        .remove(0);
    manager.broadcast(&event).await;
    let live = receive_events(&mut ws_stream, 1, Duration::from_secs(5)).await;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0]["id"], event.id.to_string());

    ws_stream.close(None).await.ok();
    shutdown_token.cancel();
}

// ============================================================================
// Client Close Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_client_close(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
    let manager = state.workflow_event_manager.clone();
    let addr = start_test_server(state).await;
    let token = get_valid_token(addr).await;

    let url = format!("ws://{}/api/v1/workflow_events/ws?token={}", addr, token);
    let (mut ws_stream, _) = connect_async(&url).await.expect("Failed to connect");

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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_events_ws_empty_filters(pool: PgPool) {
    let state = setup_test_state_with_token(pool, CancellationToken::new()).await;
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
