use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use kruxiaflow_core::events::WorkflowEventType;
use serde::Deserialize;
use std::borrow::Cow;
use uuid::Uuid;

use crate::{
    error::AppError,
    state::AppState,
    workflow_events::{
        manager::SubscriptionFilter,
        messages::{CLOSE_GOING_AWAY, CLOSE_SLOW_CLIENT, WorkflowEventMessage},
    },
};

/// Channel capacity for workflow event subscriptions.
/// When full, slow clients are disconnected.
const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 1000;

/// Replay event limit per reconnection
const REPLAY_LIMIT: i64 = 1000;

/// Ping interval for keepalive
const PING_INTERVAL_SECS: u64 = 30;

/// Query parameters for the workflow events WebSocket endpoint.
#[derive(Debug, Deserialize)]
pub struct WorkflowEventParams {
    /// JWT Bearer token (required)
    pub token: Option<String>,
    /// Comma-separated workflow IDs to filter
    pub workflow_id: Option<String>,
    /// Comma-separated event types to filter (PascalCase)
    pub event_type: Option<String>,
    /// Reconnection replay: pass the last received event `id` (UUIDv7) to resume
    /// from that exact checkpoint. Events with `id > from_event_id` are replayed before
    /// live streaming begins. Clients should track the `id` field from each received
    /// event message. Not intended for historical queries by time range.
    pub from_event_id: Option<String>,
}

/// Parse a PascalCase event type string into WorkflowEventType.
fn parse_event_type(s: &str) -> Result<WorkflowEventType, String> {
    match s.trim() {
        "WorkflowCreated" => Ok(WorkflowEventType::WorkflowCreated),
        "WorkflowUpdated" => Ok(WorkflowEventType::WorkflowUpdated),
        "ActivityScheduled" => Ok(WorkflowEventType::ActivityScheduled),
        "ActivityWaiting" => Ok(WorkflowEventType::ActivityWaiting),
        "ActivitySignaled" => Ok(WorkflowEventType::ActivitySignaled),
        "ActivityCompleted" => Ok(WorkflowEventType::ActivityCompleted),
        "ActivityFailed" => Ok(WorkflowEventType::ActivityFailed),
        "WorkflowCompleted" => Ok(WorkflowEventType::WorkflowCompleted),
        "WorkflowFailed" => Ok(WorkflowEventType::WorkflowFailed),
        other => Err(format!("Invalid event type: '{}'", other)),
    }
}

/// WebSocket endpoint for workflow event streaming.
///
/// `GET /api/v1/workflow_events/ws?token=<jwt>&workflow_id=<csv>&event_type=<csv>&from_event_id=<uuid>`
///
/// Authentication is via query parameter since WebSocket upgrade bypasses
/// HTTP middleware.
pub async fn workflow_events_ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WorkflowEventParams>,
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
        tracing::warn!("WebSocket workflow events authentication failed");
    })?;

    // Parse workflow_id comma-delimited string
    let workflow_ids: Vec<Uuid> = match &params.workflow_id {
        Some(csv) if !csv.is_empty() => {
            let mut ids = Vec::new();
            for part in csv.split(',') {
                let id = Uuid::parse_str(part.trim()).map_err(|_| {
                    AppError::BadRequest(format!("Invalid workflow_id: '{}'", part.trim()))
                })?;
                ids.push(id);
            }
            ids
        }
        _ => Vec::new(),
    };

    // Parse event_type comma-delimited string
    let event_types: Vec<WorkflowEventType> = match &params.event_type {
        Some(csv) if !csv.is_empty() => {
            let mut types = Vec::new();
            for part in csv.split(',') {
                let et = parse_event_type(part).map_err(AppError::BadRequest)?;
                types.push(et);
            }
            types
        }
        _ => Vec::new(),
    };

    // Parse from_event_id for replay
    let replay_from: Option<Uuid> = match &params.from_event_id {
        Some(id_str) if !id_str.is_empty() => {
            let id = Uuid::parse_str(id_str.trim()).map_err(|_| {
                AppError::BadRequest(format!("Invalid from_event_id: '{}'", id_str))
            })?;
            Some(id)
        }
        _ => None,
    };

    tracing::info!(
        subject = %claims.sub,
        workflow_filter_count = workflow_ids.len(),
        event_type_filter_count = event_types.len(),
        replay = replay_from.is_some(),
        "WebSocket workflow events connection authenticated"
    );

    let filter = SubscriptionFilter {
        workflow_ids,
        event_types,
    };

    Ok(ws
        .on_upgrade(move |socket| handle_workflow_event_socket(socket, filter, replay_from, state)))
}

/// Handle an established WebSocket connection for workflow events.
async fn handle_workflow_event_socket(
    socket: WebSocket,
    filter: SubscriptionFilter,
    replay_from: Option<Uuid>,
    state: AppState,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Create bounded channel for backpressure
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(SUBSCRIPTION_CHANNEL_CAPACITY);

    // Clone tx for the ping task before registering (which moves tx into the manager)
    let ping_tx = tx.clone();

    // Capture filter fields needed for replay before filter is moved into register().
    let replay_workflow_ids = filter.workflow_ids.clone();
    let replay_event_types = filter.event_types.clone();

    // Register subscription BEFORE replay so that live events arriving during replay
    // are buffered in the channel. This avoids a gap where events between the last
    // replayed event and the moment of registration would be silently lost.
    //
    // The trade-off is that some events may appear in both the replay and the channel
    // (duplicates). Clients must deduplicate by event ID, which is included in every
    // message. Duplicates are preferable to lost events.
    let conn_id = state.workflow_event_manager.register(filter, tx).await;

    // Replay missed events directly to WebSocket (not through the channel, to ensure
    // replay events arrive in order before any live events buffered in the channel).
    if let Some(from_event_id) = replay_from {
        let workflow_ids_filter = if replay_workflow_ids.is_empty() {
            None
        } else {
            Some(replay_workflow_ids.as_slice())
        };

        match state
            .event_source
            .get_events_from_id(from_event_id, workflow_ids_filter, REPLAY_LIMIT)
            .await
        {
            Ok(events) => {
                let mut replay_count = 0;
                for event in &events {
                    // Apply event_type filter (get_events_from_id only filters by workflow_id)
                    if !replay_event_types.is_empty()
                        && !replay_event_types.contains(&event.event_type)
                    {
                        continue;
                    }
                    let msg = WorkflowEventMessage::from_workflow_event(event);
                    if let Ok(json) = msg.to_json() {
                        if ws_sender.send(Message::Text(json)).await.is_err() {
                            tracing::debug!("WebSocket closed during replay");
                            state.workflow_event_manager.unregister(conn_id).await;
                            return;
                        }
                        replay_count += 1;
                    }
                }
                if replay_count > 0 {
                    tracing::info!(replay_count, "Replayed missed workflow events");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to replay workflow events");
                // Continue without replay — better to stream live than disconnect
            }
        }
    }

    tracing::debug!(connection_id = %conn_id, "Workflow event WebSocket connection established");

    // Task: Send periodic ping keepalives through the channel
    let ping_shutdown = state.shutdown_token.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PING_INTERVAL_SECS));
        interval.tick().await; // Skip the first immediate tick
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let ping = WorkflowEventMessage::ping();
                    if let Ok(json) = ping.to_json()
                        && ping_tx.try_send(json).is_err()
                    {
                        // Channel full or closed — stop pinging
                        break;
                    }
                }
                _ = ping_shutdown.cancelled() => break,
            }
        }
    });

    // Task: Forward events from channel to WebSocket
    let send_conn_id = conn_id;
    let send_shutdown = state.shutdown_token.clone();
    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(json) => {
                            if ws_sender.send(Message::Text(json)).await.is_err() {
                                tracing::debug!(
                                    connection_id = %send_conn_id,
                                    "WebSocket send failed, closing"
                                );
                                break;
                            }
                        }
                        None => {
                            // Channel closed — slow client was dropped by manager
                            let error_msg = WorkflowEventMessage::error(
                                CLOSE_SLOW_CLIENT,
                                "Client too slow, disconnecting",
                            );
                            if let Ok(json) = error_msg.to_json() {
                                let _ = ws_sender.send(Message::Text(json)).await;
                            }
                            let _ = ws_sender.send(Message::Close(Some(CloseFrame {
                                code: CLOSE_SLOW_CLIENT,
                                reason: Cow::Borrowed("Client too slow"),
                            }))).await;
                            break;
                        }
                    }
                }
                _ = send_shutdown.cancelled() => {
                    let error_msg = WorkflowEventMessage::error(
                        CLOSE_GOING_AWAY,
                        "Server shutting down",
                    );
                    if let Ok(json) = error_msg.to_json() {
                        let _ = ws_sender.send(Message::Text(json)).await;
                    }
                    let _ = ws_sender.send(Message::Close(Some(CloseFrame {
                        code: CLOSE_GOING_AWAY,
                        reason: Cow::Borrowed("Server shutting down"),
                    }))).await;
                    break;
                }
            }
        }
    });

    // Task: Handle incoming WebSocket messages (ping/pong/close)
    let recv_conn_id = conn_id;
    let recv_task = tokio::spawn(async move {
        while let Some(msg_result) = ws_receiver.next().await {
            match msg_result {
                Ok(Message::Close(frame)) => {
                    tracing::debug!(
                        connection_id = %recv_conn_id,
                        close_frame = ?frame,
                        "WebSocket close frame received"
                    );
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                    // Axum handles pong responses automatically
                }
                Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {
                    tracing::warn!(
                        connection_id = %recv_conn_id,
                        "Unexpected data message from workflow events WebSocket client"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        connection_id = %recv_conn_id,
                        error = %e,
                        "WebSocket error"
                    );
                    break;
                }
            }
        }
    });

    // Wait for any task to complete (send/recv ending means connection is done)
    tokio::select! {
        _ = send_task => {
            tracing::debug!(connection_id = %conn_id, "Send task completed");
        }
        _ = recv_task => {
            tracing::debug!(connection_id = %conn_id, "Receive task completed");
        }
    }

    // Abort the ping task since connection is closing
    ping_task.abort();

    // Cleanup
    state.workflow_event_manager.unregister(conn_id).await;

    tracing::info!(connection_id = %conn_id, "Workflow event WebSocket connection closed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use kruxiaflow_core::events::WorkflowEventType;

    #[test]
    fn test_parse_event_type_workflow_created() {
        assert_eq!(
            parse_event_type("WorkflowCreated").unwrap(),
            WorkflowEventType::WorkflowCreated
        );
    }

    #[test]
    fn test_parse_event_type_workflow_updated() {
        assert_eq!(
            parse_event_type("WorkflowUpdated").unwrap(),
            WorkflowEventType::WorkflowUpdated
        );
    }

    #[test]
    fn test_parse_event_type_activity_scheduled() {
        assert_eq!(
            parse_event_type("ActivityScheduled").unwrap(),
            WorkflowEventType::ActivityScheduled
        );
    }

    #[test]
    fn test_parse_event_type_activity_waiting() {
        assert_eq!(
            parse_event_type("ActivityWaiting").unwrap(),
            WorkflowEventType::ActivityWaiting
        );
    }

    #[test]
    fn test_parse_event_type_activity_signaled() {
        assert_eq!(
            parse_event_type("ActivitySignaled").unwrap(),
            WorkflowEventType::ActivitySignaled
        );
    }

    #[test]
    fn test_parse_event_type_activity_completed() {
        assert_eq!(
            parse_event_type("ActivityCompleted").unwrap(),
            WorkflowEventType::ActivityCompleted
        );
    }

    #[test]
    fn test_parse_event_type_activity_failed() {
        assert_eq!(
            parse_event_type("ActivityFailed").unwrap(),
            WorkflowEventType::ActivityFailed
        );
    }

    #[test]
    fn test_parse_event_type_workflow_completed() {
        assert_eq!(
            parse_event_type("WorkflowCompleted").unwrap(),
            WorkflowEventType::WorkflowCompleted
        );
    }

    #[test]
    fn test_parse_event_type_workflow_failed() {
        assert_eq!(
            parse_event_type("WorkflowFailed").unwrap(),
            WorkflowEventType::WorkflowFailed
        );
    }

    #[test]
    fn test_parse_event_type_invalid() {
        let result = parse_event_type("NotAType");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid event type: 'NotAType'");
    }

    #[test]
    fn test_parse_event_type_trims_whitespace() {
        assert_eq!(
            parse_event_type("  WorkflowCreated  ").unwrap(),
            WorkflowEventType::WorkflowCreated
        );
    }

    #[test]
    fn test_parse_event_type_empty_string() {
        assert!(parse_event_type("").is_err());
    }

    #[test]
    fn test_parse_event_type_case_sensitive() {
        assert!(parse_event_type("workflowcreated").is_err());
        assert!(parse_event_type("WORKFLOWCREATED").is_err());
    }

    #[test]
    fn test_params_deserialization() {
        let json = serde_json::json!({
            "token": "my-jwt",
            "workflow_id": "abc,def",
            "event_type": "WorkflowCreated",
            "from_event_id": "01234567-89ab-cdef-0123-456789abcdef"
        });
        let params: WorkflowEventParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.token, Some("my-jwt".to_string()));
        assert_eq!(params.workflow_id, Some("abc,def".to_string()));
        assert_eq!(params.event_type, Some("WorkflowCreated".to_string()));
        assert!(params.from_event_id.is_some());
    }

    #[test]
    fn test_params_deserialization_all_optional() {
        let json = serde_json::json!({});
        let params: WorkflowEventParams = serde_json::from_value(json).unwrap();
        assert!(params.token.is_none());
        assert!(params.workflow_id.is_none());
        assert!(params.event_type.is_none());
        assert!(params.from_event_id.is_none());
    }
}
