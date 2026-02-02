//! Streaming support for activity execution.
//!
//! This module provides types and traits for activities that emit streaming
//! output, particularly LLM activities that generate tokens incrementally.
//!
//! # Architecture
//!
//! ```text
//! Activity ──StreamSender──► API Server ──WebSocket──► Client
//!              │
//!    (trait object allows different implementations)
//! ```
//!
//! The `StreamSender` trait abstracts token delivery, allowing:
//! - Direct WebSocket broadcasting (in-process API server)
//! - Event stream publishing (distributed workers)
//! - Testing with mock senders
//!
//! # Usage
//!
//! Activities that support streaming implement [`StreamingActivity`] and
//! receive a [`StreamSender`] when executed with streaming enabled.
//!
//! ```ignore
//! // Activity implementation
//! impl StreamingActivity for MyLLMActivity {
//!     async fn execute_streaming(
//!         &self,
//!         params: Value,
//!         sender: Box<dyn StreamSender>,
//!     ) -> Result<ActivityResult> {
//!         for token in llm_response.tokens() {
//!             sender.send_token(token.text, token.index).await?;
//!         }
//!         sender.send_complete(activity_id, result).await?;
//!         Ok(result)
//!     }
//! }
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;
use uuid::Uuid;

use crate::activity_result::ActivityResult;

/// A token emitted during streaming activity execution.
///
/// Represents an incremental piece of output, typically from an LLM
/// generating text token-by-token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamToken {
    /// The token text content.
    pub text: String,
    /// Zero-based index of this token in the stream.
    pub index: u32,
}

impl StreamToken {
    /// Create a new stream token.
    pub fn new(text: impl Into<String>, index: u32) -> Self {
        Self {
            text: text.into(),
            index,
        }
    }
}

/// Error type for streaming operations.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// Failed to send token (connection closed, etc.)
    #[error("Failed to send token: {0}")]
    SendFailed(String),

    /// Activity execution failed
    #[error("Activity execution failed: {0}")]
    ExecutionFailed(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

/// Trait for sending streaming tokens during activity execution.
///
/// This trait abstracts the mechanism for delivering tokens to clients,
/// allowing different implementations for different deployment scenarios:
///
/// - In-process: Direct broadcast to WebSocket ConnectionManager
/// - Distributed: Publish to event stream for API server to consume
/// - Testing: Collect tokens in a Vec for assertions
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to allow use across async tasks.
#[async_trait]
pub trait StreamSender: Send + Sync + Debug {
    /// Send a token to all connected clients.
    ///
    /// # Arguments
    ///
    /// * `text` - The token text content
    /// * `index` - Zero-based index in the stream
    ///
    /// # Returns
    ///
    /// Number of clients that received the token, or error if send failed.
    async fn send_token(&self, text: &str, index: u32) -> Result<usize, StreamError>;

    /// Signal successful completion of the streaming activity.
    ///
    /// # Arguments
    ///
    /// * `activity_id` - The activity that completed
    /// * `result` - The final activity result
    async fn send_complete(&self, activity_id: Uuid, result: Value) -> Result<usize, StreamError>;

    /// Signal that the activity failed with an error.
    ///
    /// # Arguments
    ///
    /// * `activity_id` - The activity that failed
    /// * `error` - Human-readable error message
    async fn send_error(&self, activity_id: Uuid, error: &str) -> Result<usize, StreamError>;

    /// Close all connections for this activity.
    ///
    /// Called after sending complete or error to clean up resources.
    async fn close(&self) -> Result<(), StreamError>;
}

/// Trait for activities that support streaming output.
///
/// Activities implementing this trait can emit incremental output during
/// execution, enabling real-time feedback to clients (e.g., LLM token streaming).
///
/// # Implementation Notes
///
/// - Call `sender.send_token()` for each incremental output
/// - Call `sender.send_complete()` when finished successfully
/// - Call `sender.send_error()` if execution fails
/// - The sender handles delivery to connected clients
///
/// # Example
///
/// ```ignore
/// #[async_trait]
/// impl StreamingActivity for LLMPromptActivity {
///     async fn execute_streaming(
///         &self,
///         activity_id: Uuid,
///         parameters: Value,
///         sender: Box<dyn StreamSender>,
///     ) -> Result<ActivityResult> {
///         let params: LLMPromptParams = serde_json::from_value(parameters)?;
///
///         let mut index = 0;
///         for token in llm.stream_completion(&params).await? {
///             sender.send_token(&token, index).await?;
///             index += 1;
///         }
///
///         let result = json!({"content": full_response});
///         sender.send_complete(activity_id, result.clone()).await?;
///         sender.close().await?;
///
///         Ok(ActivityResult::value("result", result))
///     }
///
///     fn supports_streaming(&self) -> bool {
///         true
///     }
/// }
/// ```
#[async_trait]
pub trait StreamingActivity: Send + Sync {
    /// Execute the activity with streaming support.
    ///
    /// # Arguments
    ///
    /// * `activity_id` - Unique identifier for this activity execution
    /// * `parameters` - Activity input parameters
    /// * `sender` - Channel for sending streaming tokens
    ///
    /// # Returns
    ///
    /// The final activity result (same as non-streaming execution).
    async fn execute_streaming(
        &self,
        activity_id: Uuid,
        parameters: Value,
        sender: Box<dyn StreamSender>,
    ) -> anyhow::Result<ActivityResult>;

    /// Check if this activity supports streaming.
    ///
    /// Returns `true` if the activity can emit incremental output.
    /// Default implementation returns `true` since implementing this
    /// trait implies streaming support.
    fn supports_streaming(&self) -> bool {
        true
    }
}

/// A no-op stream sender that discards all tokens.
///
/// Useful for:
/// - Testing activities without streaming infrastructure
/// - Running streaming activities in non-streaming mode
/// - Benchmarking activity execution without I/O overhead
#[derive(Debug, Clone, Default)]
pub struct NoOpStreamSender;

impl NoOpStreamSender {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl StreamSender for NoOpStreamSender {
    async fn send_token(&self, _text: &str, _index: u32) -> Result<usize, StreamError> {
        Ok(0) // No clients
    }

    async fn send_complete(
        &self,
        _activity_id: Uuid,
        _result: Value,
    ) -> Result<usize, StreamError> {
        Ok(0)
    }

    async fn send_error(&self, _activity_id: Uuid, _error: &str) -> Result<usize, StreamError> {
        Ok(0)
    }

    async fn close(&self) -> Result<(), StreamError> {
        Ok(())
    }
}

/// A stream sender that collects tokens for testing.
///
/// Useful for unit tests that need to verify streaming output.
#[derive(Debug, Default)]
pub struct CollectingStreamSender {
    tokens: std::sync::Mutex<Vec<StreamToken>>,
    completed: std::sync::Mutex<Option<(Uuid, Value)>>,
    error: std::sync::Mutex<Option<(Uuid, String)>>,
}

impl CollectingStreamSender {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all collected tokens.
    pub fn tokens(&self) -> Vec<StreamToken> {
        self.tokens.lock().unwrap().clone()
    }

    /// Get the completion result if activity completed successfully.
    pub fn completion(&self) -> Option<(Uuid, Value)> {
        self.completed.lock().unwrap().clone()
    }

    /// Get the error if activity failed.
    pub fn error(&self) -> Option<(Uuid, String)> {
        self.error.lock().unwrap().clone()
    }

    /// Check if the activity completed (either success or error).
    pub fn is_finished(&self) -> bool {
        self.completed.lock().unwrap().is_some() || self.error.lock().unwrap().is_some()
    }
}

#[async_trait]
impl StreamSender for CollectingStreamSender {
    async fn send_token(&self, text: &str, index: u32) -> Result<usize, StreamError> {
        self.tokens
            .lock()
            .unwrap()
            .push(StreamToken::new(text, index));
        Ok(1)
    }

    async fn send_complete(&self, activity_id: Uuid, result: Value) -> Result<usize, StreamError> {
        *self.completed.lock().unwrap() = Some((activity_id, result));
        Ok(1)
    }

    async fn send_error(&self, activity_id: Uuid, error: &str) -> Result<usize, StreamError> {
        *self.error.lock().unwrap() = Some((activity_id, error.to_string()));
        Ok(1)
    }

    async fn close(&self) -> Result<(), StreamError> {
        Ok(())
    }
}

/// HTTP-based stream sender that publishes tokens via the API server.
///
/// This implementation is used by distributed workers that don't have
/// direct access to the WebSocket ConnectionManager.
#[derive(Debug)]
pub struct HttpStreamSender {
    client: reqwest::Client,
    api_url: String,
    activity_id: Uuid,
    auth_token: Option<String>,
}

impl HttpStreamSender {
    /// Create a new HTTP stream sender.
    ///
    /// # Arguments
    ///
    /// * `api_url` - Base URL of the API server (e.g., "http://localhost:8080")
    /// * `activity_id` - ID of the activity being executed
    /// * `auth_token` - Optional JWT token for authentication
    pub fn new(api_url: String, activity_id: Uuid, auth_token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_url,
            activity_id,
            auth_token,
        }
    }

    /// Check if there are any WebSocket subscribers for this activity.
    ///
    /// Returns `true` if there is at least one subscriber, indicating
    /// that streaming is worth doing.
    pub async fn has_subscribers(&self) -> Result<bool, StreamError> {
        let url = format!(
            "{}/api/v1/activities/{}/ws/subscribers",
            self.api_url, self.activity_id
        );

        let mut request = self.client.get(&url);

        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| StreamError::SendFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StreamError::SendFailed(format!(
                "Failed to get subscriber count: {} - {}",
                status, body
            )));
        }

        #[derive(serde::Deserialize)]
        struct Response {
            count: usize,
        }

        let result: Response = response
            .json()
            .await
            .map_err(|e| StreamError::SendFailed(e.to_string()))?;

        Ok(result.count > 0)
    }
}

#[async_trait]
impl StreamSender for HttpStreamSender {
    async fn send_token(&self, text: &str, index: u32) -> Result<usize, StreamError> {
        let url = format!(
            "{}/api/v1/activities/{}/ws/token",
            self.api_url, self.activity_id
        );

        #[derive(serde::Serialize)]
        struct Payload {
            text: String,
            index: u32,
        }

        let payload = Payload {
            text: text.to_string(),
            index,
        };

        let mut request = self.client.post(&url).json(&payload);

        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| StreamError::SendFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StreamError::SendFailed(format!(
                "Failed to publish token: {} - {}",
                status, body
            )));
        }

        #[derive(serde::Deserialize)]
        struct Response {
            subscribers: usize,
        }

        let result: Response = response
            .json()
            .await
            .map_err(|e| StreamError::SendFailed(e.to_string()))?;

        Ok(result.subscribers)
    }

    async fn send_complete(&self, activity_id: Uuid, result: Value) -> Result<usize, StreamError> {
        let url = format!(
            "{}/api/v1/activities/{}/ws/complete",
            self.api_url, activity_id
        );

        #[derive(serde::Serialize)]
        struct Payload {
            result: Value,
        }

        let payload = Payload { result };

        let mut request = self.client.post(&url).json(&payload);

        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| StreamError::SendFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StreamError::SendFailed(format!(
                "Failed to publish completion: {} - {}",
                status, body
            )));
        }

        Ok(0) // Completion endpoint doesn't return subscriber count
    }

    async fn send_error(&self, activity_id: Uuid, error: &str) -> Result<usize, StreamError> {
        let url = format!(
            "{}/api/v1/activities/{}/ws/error",
            self.api_url, activity_id
        );

        #[derive(serde::Serialize)]
        struct Payload {
            error: String,
        }

        let payload = Payload {
            error: error.to_string(),
        };

        let mut request = self.client.post(&url).json(&payload);

        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| StreamError::SendFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StreamError::SendFailed(format!(
                "Failed to publish error: {} - {}",
                status, body
            )));
        }

        Ok(0)
    }

    async fn close(&self) -> Result<(), StreamError> {
        // Closing is handled by complete/error endpoints
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_stream_token_new() {
        let token = StreamToken::new("hello", 0);
        assert_eq!(token.text, "hello");
        assert_eq!(token.index, 0);
    }

    #[test]
    fn test_stream_token_serialization() {
        let token = StreamToken::new("world", 42);
        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("\"text\":\"world\""));
        assert!(json.contains("\"index\":42"));

        let parsed: StreamToken = serde_json::from_str(&json).unwrap();
        assert_eq!(token, parsed);
    }

    #[tokio::test]
    async fn test_noop_stream_sender() {
        let sender = NoOpStreamSender::new();

        let count = sender.send_token("test", 0).await.unwrap();
        assert_eq!(count, 0);

        let count = sender
            .send_complete(Uuid::now_v7(), json!({"ok": true}))
            .await
            .unwrap();
        assert_eq!(count, 0);

        sender.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_collecting_stream_sender() {
        let sender = CollectingStreamSender::new();

        // Send some tokens
        sender.send_token("Hello", 0).await.unwrap();
        sender.send_token(" ", 1).await.unwrap();
        sender.send_token("world", 2).await.unwrap();

        // Check collected tokens
        let tokens = sender.tokens();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].text, "Hello");
        assert_eq!(tokens[1].text, " ");
        assert_eq!(tokens[2].text, "world");

        // Not finished yet
        assert!(!sender.is_finished());

        // Complete
        let activity_id = Uuid::now_v7();
        let result = json!({"content": "Hello world"});
        sender
            .send_complete(activity_id, result.clone())
            .await
            .unwrap();

        // Now finished
        assert!(sender.is_finished());

        let (id, res) = sender.completion().unwrap();
        assert_eq!(id, activity_id);
        assert_eq!(res, result);
    }

    #[tokio::test]
    async fn test_collecting_stream_sender_error() {
        let sender = CollectingStreamSender::new();
        let activity_id = Uuid::now_v7();

        sender
            .send_error(activity_id, "Something went wrong")
            .await
            .unwrap();

        assert!(sender.is_finished());
        assert!(sender.completion().is_none());

        let (id, err) = sender.error().unwrap();
        assert_eq!(id, activity_id);
        assert_eq!(err, "Something went wrong");
    }

    #[test]
    fn test_stream_error_display() {
        let err = StreamError::SendFailed("connection closed".to_string());
        assert_eq!(err.to_string(), "Failed to send token: connection closed");

        let err = StreamError::ExecutionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "Activity execution failed: timeout");
    }

    #[test]
    fn test_stream_error_serialization_error_display() {
        let err = StreamError::SerializationError("invalid JSON".to_string());
        assert_eq!(err.to_string(), "Serialization error: invalid JSON");
    }

    #[test]
    fn test_stream_error_debug() {
        let err = StreamError::SendFailed("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("SendFailed"));
    }

    #[test]
    fn test_http_stream_sender_new() {
        let activity_id = Uuid::now_v7();
        let sender = HttpStreamSender::new(
            "http://localhost:8080".to_string(),
            activity_id,
            Some("test-token".to_string()),
        );

        assert_eq!(sender.api_url, "http://localhost:8080");
        assert_eq!(sender.activity_id, activity_id);
        assert_eq!(sender.auth_token, Some("test-token".to_string()));
    }

    #[test]
    fn test_http_stream_sender_new_without_token() {
        let activity_id = Uuid::now_v7();
        let sender = HttpStreamSender::new("http://localhost:8080".to_string(), activity_id, None);

        assert!(sender.auth_token.is_none());
    }

    #[test]
    fn test_http_stream_sender_debug() {
        let activity_id = Uuid::now_v7();
        let sender = HttpStreamSender::new("http://localhost:8080".to_string(), activity_id, None);

        let debug_str = format!("{:?}", sender);
        assert!(debug_str.contains("HttpStreamSender"));
        assert!(debug_str.contains("localhost:8080"));
    }

    #[test]
    fn test_stream_token_clone() {
        let token = StreamToken::new("hello", 5);
        let cloned = token.clone();
        assert_eq!(token, cloned);
    }

    #[test]
    fn test_stream_token_equality() {
        let a = StreamToken::new("hello", 0);
        let b = StreamToken::new("hello", 0);
        let c = StreamToken::new("world", 0);
        let d = StreamToken::new("hello", 1);

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[tokio::test]
    async fn test_noop_stream_sender_error() {
        let sender = NoOpStreamSender::new();
        let activity_id = Uuid::now_v7();

        let count = sender.send_error(activity_id, "test error").await.unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_noop_stream_sender_default() {
        let sender = NoOpStreamSender;
        let debug_str = format!("{:?}", sender);
        assert!(debug_str.contains("NoOpStreamSender"));
    }

    #[test]
    fn test_noop_stream_sender_clone() {
        let sender = NoOpStreamSender::new();
        let _cloned = sender.clone();
    }

    #[tokio::test]
    async fn test_collecting_stream_sender_not_finished_initially() {
        let sender = CollectingStreamSender::new();
        assert!(!sender.is_finished());
        assert!(sender.completion().is_none());
        assert!(sender.error().is_none());
        assert!(sender.tokens().is_empty());
    }

    #[tokio::test]
    async fn test_collecting_stream_sender_close() {
        let sender = CollectingStreamSender::new();
        sender.close().await.unwrap();
        // close is a no-op for collecting sender
    }

    #[tokio::test]
    async fn test_http_stream_sender_close_is_noop() {
        let sender =
            HttpStreamSender::new("http://localhost:9999".to_string(), Uuid::now_v7(), None);
        // close() just returns Ok for HTTP sender
        sender.close().await.unwrap();
    }

    // =========================================================================
    // HttpStreamSender wiremock tests
    // =========================================================================

    #[tokio::test]
    async fn test_http_stream_sender_has_subscribers_true() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/activities/.*/ws/subscribers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 3})))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let result = sender.has_subscribers().await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_http_stream_sender_has_subscribers_false() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/activities/.*/ws/subscribers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 0})))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let result = sender.has_subscribers().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_http_stream_sender_has_subscribers_with_auth() {
        use wiremock::matchers::{header, method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/activities/.*/ws/subscribers"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 1})))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(
            mock_server.uri(),
            activity_id,
            Some("test-token".to_string()),
        );
        let result = sender.has_subscribers().await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_http_stream_sender_has_subscribers_server_error() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("GET"))
            .and(path_regex(r"/api/v1/activities/.*/ws/subscribers"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal error"))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let result = sender.has_subscribers().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_token_success() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"subscribers": 2})))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let count = sender.send_token("hello world", 0).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_token_with_auth() {
        use wiremock::matchers::{header, method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/token"))
            .and(header("Authorization", "Bearer my-jwt"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"subscribers": 1})))
            .mount(&mock_server)
            .await;

        let sender =
            HttpStreamSender::new(mock_server.uri(), activity_id, Some("my-jwt".to_string()));
        let count = sender.send_token("token text", 5).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_token_server_error() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/token"))
            .respond_with(ResponseTemplate::new(503).set_body_string("Service unavailable"))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let result = sender.send_token("test", 0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_complete_success() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/complete"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let count = sender
            .send_complete(activity_id, json!({"output": "done"}))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_complete_server_error() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/complete"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let result = sender.send_complete(activity_id, json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_error_success() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/error"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let count = sender
            .send_error(activity_id, "something went wrong")
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_http_stream_sender_send_error_server_error() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let activity_id = Uuid::now_v7();

        Mock::given(method("POST"))
            .and(path_regex(r"/api/v1/activities/.*/ws/error"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let sender = HttpStreamSender::new(mock_server.uri(), activity_id, None);
        let result = sender.send_error(activity_id, "test error").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_stream_sender_connection_refused() {
        let activity_id = Uuid::now_v7();
        let sender = HttpStreamSender::new(
            "http://127.0.0.1:1".to_string(), // Port 1 should refuse connections
            activity_id,
            None,
        );

        let result = sender.has_subscribers().await;
        assert!(result.is_err());

        let result = sender.send_token("test", 0).await;
        assert!(result.is_err());
    }
}
