//! WebSocket message types for activity streaming.
//!
//! This module defines the message protocol for streaming activity results
//! (particularly LLM token streaming) over WebSocket connections.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages sent from server to client over WebSocket.
///
/// All messages are JSON-encoded with a `type` field for discriminating variants.
///
/// # Wire Format Examples
///
/// Token message:
/// ```json
/// {"type":"token","text":"hello","index":0,"timestamp":"2024-01-15T10:30:00Z"}
/// ```
///
/// Complete message:
/// ```json
/// {"type":"complete","activity_id":"...","result":{...},"timestamp":"..."}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamMessage {
    /// Token chunk from LLM streaming.
    ///
    /// Sent incrementally as the LLM generates output.
    Token {
        /// The token text content.
        text: String,
        /// Zero-based index of this token in the stream.
        index: u32,
        /// Server timestamp when token was received.
        timestamp: DateTime<Utc>,
    },

    /// Activity completed successfully.
    ///
    /// Sent when the streaming activity finishes. The connection
    /// will be closed shortly after this message.
    Complete {
        /// The activity that completed.
        activity_id: Uuid,
        /// The activity result payload.
        result: serde_json::Value,
        /// Server timestamp of completion.
        timestamp: DateTime<Utc>,
    },

    /// Activity failed with error.
    ///
    /// Sent when the streaming activity encounters an error.
    /// The connection will be closed shortly after this message.
    Error {
        /// The activity that failed.
        activity_id: Uuid,
        /// Human-readable error message.
        error: String,
        /// Server timestamp of the error.
        timestamp: DateTime<Utc>,
    },

    /// Heartbeat to keep connection alive.
    ///
    /// Sent periodically by the server to prevent connection timeouts.
    /// Clients should respond with a pong frame (handled at WebSocket protocol level).
    Ping {
        /// Server timestamp of the ping.
        timestamp: DateTime<Utc>,
    },
}

impl StreamMessage {
    /// Serialize message to JSON string.
    ///
    /// # Errors
    ///
    /// Returns error if serialization fails (should not happen with valid data).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize message from JSON string.
    ///
    /// # Errors
    ///
    /// Returns error if the JSON is invalid or doesn't match any variant.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Create a token message with the current timestamp.
    pub fn token(text: impl Into<String>, index: u32) -> Self {
        Self::Token {
            text: text.into(),
            index,
            timestamp: Utc::now(),
        }
    }

    /// Create a complete message with the current timestamp.
    pub fn complete(activity_id: Uuid, result: serde_json::Value) -> Self {
        Self::Complete {
            activity_id,
            result,
            timestamp: Utc::now(),
        }
    }

    /// Create an error message with the current timestamp.
    pub fn error(activity_id: Uuid, error: impl Into<String>) -> Self {
        Self::Error {
            activity_id,
            error: error.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a ping message with the current timestamp.
    pub fn ping() -> Self {
        Self::Ping {
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_message_serialization() {
        let timestamp = Utc::now();
        let msg = StreamMessage::Token {
            text: "hello".to_string(),
            index: 0,
            timestamp,
        };

        let json = msg.to_json().unwrap();
        assert!(json.contains(r#""type":"token"#));
        assert!(json.contains(r#""text":"hello"#));
        assert!(json.contains(r#""index":0"#));

        // Round-trip test
        let parsed = StreamMessage::from_json(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn test_complete_message_serialization() {
        let activity_id = Uuid::now_v7();
        let timestamp = Utc::now();
        let msg = StreamMessage::Complete {
            activity_id,
            result: serde_json::json!({"status": "success", "output": "Hello, world!"}),
            timestamp,
        };

        let json = msg.to_json().unwrap();
        assert!(json.contains(r#""type":"complete"#));
        assert!(json.contains(&activity_id.to_string()));
        assert!(json.contains(r#""status":"success"#));

        // Round-trip test
        let parsed = StreamMessage::from_json(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn test_error_message_serialization() {
        let activity_id = Uuid::now_v7();
        let timestamp = Utc::now();
        let msg = StreamMessage::Error {
            activity_id,
            error: "LLM provider timeout".to_string(),
            timestamp,
        };

        let json = msg.to_json().unwrap();
        assert!(json.contains(r#""type":"error"#));
        assert!(json.contains(&activity_id.to_string()));
        assert!(json.contains(r#""error":"LLM provider timeout"#));

        // Round-trip test
        let parsed = StreamMessage::from_json(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn test_ping_message_serialization() {
        let timestamp = Utc::now();
        let msg = StreamMessage::Ping { timestamp };

        let json = msg.to_json().unwrap();
        assert!(json.contains(r#""type":"ping"#));
        assert!(json.contains(r#""timestamp""#));

        // Round-trip test
        let parsed = StreamMessage::from_json(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn test_helper_constructors() {
        let msg = StreamMessage::token("test", 5);
        match msg {
            StreamMessage::Token { text, index, .. } => {
                assert_eq!(text, "test");
                assert_eq!(index, 5);
            }
            _ => panic!("Expected Token variant"),
        }

        let activity_id = Uuid::now_v7();
        let msg = StreamMessage::complete(activity_id, serde_json::json!({"ok": true}));
        match msg {
            StreamMessage::Complete {
                activity_id: id,
                result,
                ..
            } => {
                assert_eq!(id, activity_id);
                assert_eq!(result["ok"], true);
            }
            _ => panic!("Expected Complete variant"),
        }

        let msg = StreamMessage::error(activity_id, "test error");
        match msg {
            StreamMessage::Error {
                activity_id: id,
                error,
                ..
            } => {
                assert_eq!(id, activity_id);
                assert_eq!(error, "test error");
            }
            _ => panic!("Expected Error variant"),
        }

        let msg = StreamMessage::ping();
        assert!(matches!(msg, StreamMessage::Ping { .. }));
    }

    #[test]
    fn test_json_wire_format() {
        // Verify exact wire format matches API contract
        let msg = StreamMessage::Token {
            text: "world".to_string(),
            index: 42,
            timestamp: DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };

        let json = msg.to_json().unwrap();
        // Parse as generic JSON to verify structure
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["type"], "token");
        assert_eq!(value["text"], "world");
        assert_eq!(value["index"], 42);
        assert!(value["timestamp"].is_string());
    }

    #[test]
    fn test_deserialize_unknown_type_fails() {
        let json = r#"{"type":"unknown","data":"test"}"#;
        let result = StreamMessage::from_json(json);
        assert!(result.is_err());
    }
}
