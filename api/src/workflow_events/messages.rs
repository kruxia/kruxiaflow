use chrono::{DateTime, Utc};
use kruxiaflow_core::events::WorkflowEvent;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// WebSocket close code: Going Away (server shutdown)
pub const CLOSE_GOING_AWAY: u16 = 1001;

/// WebSocket close code: Internal Server Error
pub const CLOSE_INTERNAL_ERROR: u16 = 1011;

/// WebSocket close code: Client too slow (custom)
pub const CLOSE_SLOW_CLIENT: u16 = 4002;

/// Messages sent over the workflow event WebSocket connection.
///
/// Serialized as JSON with `type` discriminator field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowEventMessage {
    /// A workflow event
    Event {
        /// Event ID (for replay checkpoint)
        id: Uuid,
        /// Workflow this event belongs to
        workflow_id: Uuid,
        /// PascalCase event type name
        event_type: String,
        /// Activity key (None for workflow-level events)
        activity_key: Option<String>,
        /// Event payload
        payload: serde_json::Value,
        /// When the event occurred
        timestamp: DateTime<Utc>,
        /// Retry iteration (None for first attempt)
        iteration: Option<i32>,
    },
    /// Keepalive ping (sent every 30s)
    Ping { timestamp: DateTime<Utc> },
    /// Error message (may precede disconnect)
    Error {
        /// WebSocket close code (1001, 1011, 4002)
        code: u16,
        /// Human-readable error message
        message: String,
        timestamp: DateTime<Utc>,
    },
}

impl WorkflowEventMessage {
    /// Create an Event message from a WorkflowEvent
    pub fn from_workflow_event(event: &WorkflowEvent) -> Self {
        Self::Event {
            id: event.id,
            workflow_id: event.workflow_id,
            event_type: event.event_type.to_string(),
            activity_key: event.activity_key.clone(),
            payload: event.payload.clone(),
            timestamp: event.timestamp,
            iteration: event.iteration,
        }
    }

    /// Create a Ping message
    pub fn ping() -> Self {
        Self::Ping {
            timestamp: Utc::now(),
        }
    }

    /// Create an Error message
    pub fn error(code: u16, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
            timestamp: Utc::now(),
        }
    }

    /// Serialize to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use kruxiaflow_core::events::WorkflowEventType;
    use serde_json::json;

    fn make_workflow_event() -> WorkflowEvent {
        WorkflowEvent {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("step_1".to_string()),
            payload: json!({"result": "ok"}),
            timestamp: Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(),
            iteration: Some(2),
        }
    }

    #[test]
    fn test_close_code_constants() {
        assert_eq!(CLOSE_GOING_AWAY, 1001);
        assert_eq!(CLOSE_INTERNAL_ERROR, 1011);
        assert_eq!(CLOSE_SLOW_CLIENT, 4002);
    }

    #[test]
    fn test_from_workflow_event() {
        let event = make_workflow_event();
        let msg = WorkflowEventMessage::from_workflow_event(&event);

        match msg {
            WorkflowEventMessage::Event {
                id,
                workflow_id,
                event_type,
                activity_key,
                payload,
                timestamp,
                iteration,
            } => {
                assert_eq!(id, event.id);
                assert_eq!(workflow_id, event.workflow_id);
                assert_eq!(event_type, "ActivityCompleted");
                assert_eq!(activity_key, Some("step_1".to_string()));
                assert_eq!(payload, json!({"result": "ok"}));
                assert_eq!(timestamp, event.timestamp);
                assert_eq!(iteration, Some(2));
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_from_workflow_event_no_activity_key() {
        let event = WorkflowEvent {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            timestamp: Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(),
            iteration: None,
        };
        let msg = WorkflowEventMessage::from_workflow_event(&event);

        match msg {
            WorkflowEventMessage::Event {
                activity_key,
                iteration,
                event_type,
                ..
            } => {
                assert_eq!(activity_key, None);
                assert_eq!(iteration, None);
                assert_eq!(event_type, "WorkflowCreated");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ping() {
        let before = Utc::now();
        let msg = WorkflowEventMessage::ping();
        let after = Utc::now();

        match msg {
            WorkflowEventMessage::Ping { timestamp } => {
                assert!(timestamp >= before && timestamp <= after);
            }
            _ => panic!("Expected Ping variant"),
        }
    }

    #[test]
    fn test_error() {
        let msg = WorkflowEventMessage::error(CLOSE_GOING_AWAY, "Server shutting down");

        match msg {
            WorkflowEventMessage::Error { code, message, .. } => {
                assert_eq!(code, 1001);
                assert_eq!(message, "Server shutting down");
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_error_with_string_coercion() {
        let msg = WorkflowEventMessage::error(CLOSE_SLOW_CLIENT, String::from("Too slow"));
        match msg {
            WorkflowEventMessage::Error { code, message, .. } => {
                assert_eq!(code, 4002);
                assert_eq!(message, "Too slow");
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_event_json_roundtrip() {
        let event = make_workflow_event();
        let msg = WorkflowEventMessage::from_workflow_event(&event);
        let json = msg.to_json().unwrap();
        let deserialized = WorkflowEventMessage::from_json(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_ping_json_roundtrip() {
        let msg = WorkflowEventMessage::ping();
        let json = msg.to_json().unwrap();
        let deserialized = WorkflowEventMessage::from_json(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_error_json_roundtrip() {
        let msg = WorkflowEventMessage::error(CLOSE_INTERNAL_ERROR, "Something broke");
        let json = msg.to_json().unwrap();
        let deserialized = WorkflowEventMessage::from_json(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_event_json_has_type_discriminator() {
        let event = make_workflow_event();
        let msg = WorkflowEventMessage::from_workflow_event(&event);
        let json = msg.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "event");
    }

    #[test]
    fn test_ping_json_has_type_discriminator() {
        let msg = WorkflowEventMessage::ping();
        let json = msg.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "ping");
    }

    #[test]
    fn test_error_json_has_type_discriminator() {
        let msg = WorkflowEventMessage::error(1011, "err");
        let json = msg.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "error");
    }

    #[test]
    fn test_from_json_invalid() {
        let result = WorkflowEventMessage::from_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_json_unknown_type() {
        let result = WorkflowEventMessage::from_json(r#"{"type": "unknown"}"#);
        assert!(result.is_err());
    }
}
