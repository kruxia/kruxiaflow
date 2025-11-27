//! API handlers for activity streaming.
//!
//! These endpoints allow workers to publish streaming tokens and events
//! back to WebSocket subscribers.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{state::AppState, websocket::StreamMessage};

/// Payload for publishing a stream token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamTokenPayload {
    /// The token text content.
    #[schema(example = "Hello")]
    pub text: String,
    /// Sequential index of this token in the stream.
    #[schema(example = 0)]
    pub index: u32,
}

/// Endpoint for workers to publish streaming tokens.
///
/// Broadcasts a token to all WebSocket subscribers for the activity.
/// Returns the number of subscribers that received the token.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/stream/token",
    tag = "Streaming",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID to stream token for")
    ),
    request_body = StreamTokenPayload,
    responses(
        (status = 200, description = "Token broadcast to subscribers", body = PublishResponse),
        (status = 401, description = "Unauthorized - invalid or missing token"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn publish_stream_token(
    Path(activity_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(payload): Json<StreamTokenPayload>,
) -> Result<Json<PublishResponse>, (StatusCode, String)> {
    let count = state
        .connection_manager
        .broadcast(
            activity_id,
            StreamMessage::Token {
                text: payload.text,
                index: payload.index,
                timestamp: Utc::now(),
            },
        )
        .await;

    tracing::trace!(
        activity_id = %activity_id,
        index = payload.index,
        subscribers = count,
        "Published stream token"
    );

    Ok(Json(PublishResponse { subscribers: count }))
}

/// Payload for publishing a stream completion event.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamCompletePayload {
    /// The complete activity result.
    #[schema(value_type = Object, example = json!({"content": "Final response text"}))]
    pub result: serde_json::Value,
}

/// Endpoint for workers to signal stream completion.
///
/// Broadcasts a completion message to all subscribers and
/// closes all WebSocket connections for the activity.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/stream/complete",
    tag = "Streaming",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID to complete")
    ),
    request_body = StreamCompletePayload,
    responses(
        (status = 200, description = "Completion broadcast and connections closed"),
        (status = 401, description = "Unauthorized - invalid or missing token"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn publish_stream_complete(
    Path(activity_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(payload): Json<StreamCompletePayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Broadcast completion message
    let count = state
        .connection_manager
        .broadcast(
            activity_id,
            StreamMessage::Complete {
                activity_id,
                result: payload.result,
                timestamp: Utc::now(),
            },
        )
        .await;

    // Close all connections for this activity
    state.connection_manager.close_all(activity_id).await;

    tracing::debug!(
        activity_id = %activity_id,
        subscribers = count,
        "Published stream completion"
    );

    Ok(StatusCode::OK)
}

/// Payload for publishing a stream error event.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamErrorPayload {
    /// Error message describing what went wrong.
    #[schema(example = "Rate limit exceeded")]
    pub error: String,
}

/// Endpoint for workers to signal stream error.
///
/// Broadcasts an error message to all subscribers and
/// closes all WebSocket connections for the activity.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/stream/error",
    tag = "Streaming",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID that encountered an error")
    ),
    request_body = StreamErrorPayload,
    responses(
        (status = 200, description = "Error broadcast and connections closed"),
        (status = 401, description = "Unauthorized - invalid or missing token"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn publish_stream_error(
    Path(activity_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(payload): Json<StreamErrorPayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Broadcast error message
    let count = state
        .connection_manager
        .broadcast(
            activity_id,
            StreamMessage::Error {
                activity_id,
                error: payload.error.clone(),
                timestamp: Utc::now(),
            },
        )
        .await;

    // Close all connections for this activity
    state.connection_manager.close_all(activity_id).await;

    tracing::debug!(
        activity_id = %activity_id,
        error = %payload.error,
        subscribers = count,
        "Published stream error"
    );

    Ok(StatusCode::OK)
}

/// Response from publishing a stream event.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublishResponse {
    /// Number of subscribers that received the event.
    #[schema(example = 3)]
    pub subscribers: usize,
}

/// Get the number of active WebSocket subscribers for an activity.
///
/// This is useful for workers to check if streaming is worth doing
/// (two-level opt-in: activity config + subscribers present).
#[utoipa::path(
    get,
    path = "/api/v1/activities/{activity_id}/stream/subscribers",
    tag = "Streaming",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID to check subscribers for")
    ),
    responses(
        (status = 200, description = "Subscriber count retrieved", body = SubscriberCountResponse),
        (status = 401, description = "Unauthorized - invalid or missing token"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_subscriber_count(
    Path(activity_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Json<SubscriberCountResponse> {
    let count = state.connection_manager.connection_count(activity_id).await;
    Json(SubscriberCountResponse { count })
}

/// Response containing subscriber count.
#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriberCountResponse {
    /// Number of active WebSocket subscribers.
    #[schema(example = 1)]
    pub count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_token_payload_deserialize() {
        let json = r#"{"text": "Hello", "index": 0}"#;
        let payload: StreamTokenPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.text, "Hello");
        assert_eq!(payload.index, 0);
    }

    #[test]
    fn test_stream_complete_payload_deserialize() {
        let json = r#"{"result": {"content": "test"}}"#;
        let payload: StreamCompletePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.result["content"], "test");
    }

    #[test]
    fn test_stream_error_payload_deserialize() {
        let json = r#"{"error": "Something went wrong"}"#;
        let payload: StreamErrorPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.error, "Something went wrong");
    }
}
