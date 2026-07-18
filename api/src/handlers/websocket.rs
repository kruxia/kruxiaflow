//! WebSocket handler for activity streaming.
//!
//! Provides real-time streaming of activity results (particularly LLM tokens)
//! over WebSocket connections.
//!
//! # Endpoint
//!
//! `GET /api/v1/activities/{activity_id}/ws?token=<jwt>`
//!
//! # Authentication
//!
//! WebSocket connections authenticate via query parameter since the WebSocket
//! upgrade happens before HTTP middleware can run. The `token` query parameter
//! must contain a valid JWT Bearer token.
//!
//! # Protocol
//!
//! The server sends JSON messages of type [`StreamMessage`]:
//! - `Token`: Incremental LLM output tokens
//! - `Complete`: Activity finished successfully
//! - `Error`: Activity failed with error
//! - `Ping`: Connection keepalive
//!
//! Clients should not send data messages; only WebSocket control frames
//! (ping/pong/close) are expected.

use axum::{
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{error::AppError, state::AppState, websocket::StreamMessage};

/// Query parameters for WebSocket connection.
#[derive(Debug, Deserialize, IntoParams)]
pub struct StreamParams {
    /// JWT Bearer token for authentication.
    /// Required since WebSocket upgrade bypasses HTTP auth middleware.
    pub token: Option<String>,
}

/// WebSocket endpoint for activity streaming.
///
/// Upgrades HTTP connection to WebSocket after authenticating the token.
/// The connection is registered with the ConnectionManager and will receive
/// broadcast messages for the specified activity.
///
/// ## WebSocket Protocol
///
/// After connecting, the server sends JSON messages:
/// - `{"type": "token", "text": "...", "index": N, "timestamp": "..."}` - Incremental output
/// - `{"type": "complete", "activity_id": "...", "result": {...}, "timestamp": "..."}` - Success
/// - `{"type": "error", "activity_id": "...", "error": "...", "timestamp": "..."}` - Failure
#[utoipa::path(
    get,
    path = "/api/v1/activities/{activity_id}/ws",
    tag = "Streaming",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID to stream"),
        StreamParams
    ),
    responses(
        (status = 101, description = "WebSocket upgrade successful"),
        (status = 401, description = "Unauthorized - missing or invalid token"),
        (status = 400, description = "Bad request - invalid activity_id"),
    )
)]
pub async fn activity_stream_handler(
    ws: WebSocketUpgrade,
    Path(activity_id): Path<Uuid>,
    Query(params): Query<StreamParams>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    // Authenticate (token optional in insecure dev mode)
    let claims = crate::middleware::authenticate_optional_token(
        &state,
        params.token.as_deref(),
        "Missing authentication token. Use ?token=<jwt> query parameter",
    )
    .await
    .inspect_err(|_| {
        tracing::warn!(activity_id = %activity_id, "WebSocket authentication failed");
    })?;

    tracing::info!(
        activity_id = %activity_id,
        subject = %claims.sub,
        "WebSocket connection authenticated"
    );

    // Upgrade to WebSocket
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, activity_id, state)))
}

/// Handle an established WebSocket connection.
///
/// Registers the connection with the ConnectionManager and spawns tasks
/// for bidirectional message handling.
async fn handle_socket(socket: WebSocket, activity_id: Uuid, state: AppState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Create channel for sending messages to this connection
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Register connection with manager
    let conn_id = state.connection_manager.register(activity_id, tx).await;

    tracing::debug!(
        activity_id = %activity_id,
        connection_id = %conn_id,
        "WebSocket connection established"
    );

    // Task: Forward messages from channel to WebSocket
    let send_activity_id = activity_id;
    let send_conn_id = conn_id;
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(Message::Text(msg)).await.is_err() {
                tracing::debug!(
                    activity_id = %send_activity_id,
                    connection_id = %send_conn_id,
                    "WebSocket send failed, connection closing"
                );
                break;
            }
        }
    });

    // Task: Handle incoming WebSocket messages (ping/pong/close)
    let recv_activity_id = activity_id;
    let recv_conn_id = conn_id;
    let recv_task = tokio::spawn(async move {
        while let Some(msg_result) = ws_receiver.next().await {
            match msg_result {
                Ok(Message::Close(frame)) => {
                    tracing::debug!(
                        activity_id = %recv_activity_id,
                        connection_id = %recv_conn_id,
                        close_frame = ?frame,
                        "WebSocket close frame received"
                    );
                    break;
                }
                Ok(Message::Ping(_)) => {
                    // Axum handles pong responses automatically
                    tracing::trace!(
                        activity_id = %recv_activity_id,
                        connection_id = %recv_conn_id,
                        "WebSocket ping received"
                    );
                }
                Ok(Message::Pong(_)) => {
                    tracing::trace!(
                        activity_id = %recv_activity_id,
                        connection_id = %recv_conn_id,
                        "WebSocket pong received"
                    );
                }
                Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {
                    // Clients shouldn't send data messages in this protocol
                    tracing::warn!(
                        activity_id = %recv_activity_id,
                        connection_id = %recv_conn_id,
                        "Unexpected data message from WebSocket client"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        activity_id = %recv_activity_id,
                        connection_id = %recv_conn_id,
                        error = %e,
                        "WebSocket error"
                    );
                    break;
                }
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {
            tracing::debug!(
                activity_id = %activity_id,
                connection_id = %conn_id,
                "Send task completed"
            );
        }
        _ = recv_task => {
            tracing::debug!(
                activity_id = %activity_id,
                connection_id = %conn_id,
                "Receive task completed"
            );
        }
    }

    // Cleanup: unregister connection
    state
        .connection_manager
        .unregister(activity_id, conn_id)
        .await;

    tracing::info!(
        activity_id = %activity_id,
        connection_id = %conn_id,
        "WebSocket connection closed"
    );
}

/// Broadcast a message to all WebSocket connections for an activity.
///
/// This is a convenience function for use by activity executors.
/// Returns the number of connections that received the message.
pub async fn broadcast_to_activity(
    state: &AppState,
    activity_id: Uuid,
    message: StreamMessage,
) -> usize {
    state
        .connection_manager
        .broadcast(activity_id, message)
        .await
}

/// Close all WebSocket connections for an activity.
///
/// Called when an activity completes or fails to clean up connections.
pub async fn close_activity_connections(state: &AppState, activity_id: Uuid) {
    state.connection_manager.close_all(activity_id).await;
}

/// Get the number of active WebSocket connections for an activity.
pub async fn activity_connection_count(state: &AppState, activity_id: Uuid) -> usize {
    state.connection_manager.connection_count(activity_id).await
}

/// WebSocket endpoint for activity streaming by workflow ID and activity key.
///
/// Resolves `(workflow_id, activity_key)` to `activity_id` via DB lookup, then
/// delegates to the existing WebSocket connection infrastructure.
///
/// Retries the lookup with short sleeps (3 × 100ms) to handle the timing race
/// where a client connects before the activity is scheduled.
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/activities/{activity_key}/ws",
    tag = "Streaming",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID"),
        ("activity_key" = String, Path, description = "Activity key from workflow definition"),
        StreamParams
    ),
    responses(
        (status = 101, description = "WebSocket upgrade successful"),
        (status = 401, description = "Unauthorized - missing or invalid token"),
        (status = 404, description = "Activity not found for given workflow/key"),
    )
)]
pub async fn activity_stream_by_key_handler(
    ws: WebSocketUpgrade,
    Path((workflow_id, activity_key)): Path<(Uuid, String)>,
    Query(params): Query<StreamParams>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    // Authenticate (token optional in insecure dev mode)
    let claims = crate::middleware::authenticate_optional_token(
        &state,
        params.token.as_deref(),
        "Missing authentication token. Use ?token=<jwt> query parameter",
    )
    .await
    .inspect_err(|_| {
        tracing::warn!(
            workflow_id = %workflow_id,
            activity_key = %activity_key,
            "WebSocket authentication failed"
        );
    })?;

    tracing::info!(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        subject = %claims.sub,
        "WebSocket connection authenticated (by key)"
    );

    // Resolve activity_key to activity_id (with retry for timing race)
    let activity_id = resolve_activity_id(&state, workflow_id, &activity_key).await?;

    // Upgrade to WebSocket
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, activity_id, state)))
}

/// Resolve (workflow_id, activity_key) → activity_id via DB lookup.
///
/// Retries up to 3 times with 100ms sleeps to handle the race where a client
/// connects before the activity is scheduled in the queue.
async fn resolve_activity_id(
    state: &AppState,
    workflow_id: Uuid,
    activity_key: &str,
) -> Result<Uuid, AppError> {
    for attempt in 0..3 {
        let result = sqlx::query_scalar!(
            r#"SELECT id as "id!" FROM activity_queue
               WHERE workflow_id = $1 AND activity_key = $2
               ORDER BY iteration DESC LIMIT 1"#,
            workflow_id,
            activity_key
        )
        .fetch_optional(&state.db_pool)
        .await?;

        if let Some(id) = result {
            return Ok(id);
        }

        if attempt < 2 {
            tracing::debug!(
                workflow_id = %workflow_id,
                activity_key = %activity_key,
                attempt = attempt + 1,
                "Activity not yet scheduled, retrying"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    Err(AppError::NotFound(format!(
        "Activity '{}' not found for workflow {}",
        activity_key, workflow_id
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::state::tests::*;
    use kruxiaflow_core::cache::NoOpCache;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    // Note: Full WebSocket integration tests require a running server
    // and are in api/tests/websocket_integration_tests.rs

    #[test]
    fn test_stream_params_deserialize() {
        let params: StreamParams = serde_json::from_str(r#"{"token": "test_token"}"#).unwrap();
        assert_eq!(params.token, Some("test_token".to_string()));
    }

    #[test]
    fn test_stream_params_deserialize_missing_token() {
        let params: StreamParams = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(params.token, None);
    }

    #[test]
    fn test_stream_params_deserialize_null_token() {
        let params: StreamParams = serde_json::from_str(r#"{"token": null}"#).unwrap();
        assert_eq!(params.token, None);
    }

    async fn setup_test_state() -> AppState {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });
        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");
        AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn test_broadcast_to_activity_no_connections() {
        let state = setup_test_state().await;
        let activity_id = Uuid::now_v7();
        let message = StreamMessage::ping();
        let count = broadcast_to_activity(&state, activity_id, message).await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_activity_connection_count_no_connections() {
        let state = setup_test_state().await;
        let activity_id = Uuid::now_v7();
        let count = activity_connection_count(&state, activity_id).await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_close_activity_connections_no_connections() {
        let state = setup_test_state().await;
        let activity_id = Uuid::now_v7();
        // Should not panic even when there are no connections
        close_activity_connections(&state, activity_id).await;
    }

    #[tokio::test]
    async fn test_broadcast_after_register() {
        let state = setup_test_state().await;
        let activity_id = Uuid::now_v7();

        // Register a connection
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let conn_id = state.connection_manager.register(activity_id, tx).await;

        // Verify connection count
        let count = activity_connection_count(&state, activity_id).await;
        assert_eq!(count, 1);

        // Broadcast a message
        let msg = StreamMessage::ping();
        let sent = broadcast_to_activity(&state, activity_id, msg).await;
        assert_eq!(sent, 1);

        // Verify message received
        let received = rx.try_recv();
        assert!(received.is_ok());

        // Cleanup
        state
            .connection_manager
            .unregister(activity_id, conn_id)
            .await;
        let count = activity_connection_count(&state, activity_id).await;
        assert_eq!(count, 0);
    }
}
