use super::provider::*;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, "https://api.anthropic.com".to_string())
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
        }
    }
}

// SSE event types for Anthropic streaming API
// Fields are needed for serde deserialization even if not directly read
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum AnthropicEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartPayload },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: ContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: ContentDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaPayload,
        usage: Option<UsagePayload>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: ErrorPayload },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageStartPayload {
    id: String,
    model: String,
    usage: Option<UsagePayload>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ContentDelta {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageDeltaPayload {
    stop_reason: Option<String>,
    stop_sequence: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct UsagePayload {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ErrorPayload {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

/// Parse a single SSE line (after "data: " prefix) into an AnthropicEvent
fn parse_sse_data(data: &str) -> Option<AnthropicEvent> {
    if data.trim().is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

/// Parse SSE events from a byte chunk, handling partial lines
struct SseParser {
    buffer: String,
}

impl SseParser {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Process incoming bytes and return complete events
    fn process(&mut self, bytes: &Bytes) -> Vec<AnthropicEvent> {
        let text = String::from_utf8_lossy(bytes);
        self.buffer.push_str(&text);

        let mut events = Vec::new();

        // Process complete lines
        while let Some(pos) = self.buffer.find("\n\n") {
            let chunk = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();

            // Parse SSE format: "event: xxx\ndata: {...}"
            for line in chunk.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Some(event) = parse_sse_data(data) {
                        events.push(event);
                    }
                }
            }
        }

        events
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn prompt(&self, request: &PromptRequest) -> Result<PromptResponse> {
        let messages = vec![json!({
            "role": "user",
            "content": request.prompt,
        })];

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
        });

        if let Some(system) = &request.system_prompt {
            body["system"] = json!(system);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            body["top_p"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            body["stop_sequences"] = json!(stops);
        }

        let url = format!("{}/v1/messages", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(LLMError::ProviderError(error_text));
        }

        let response_json: serde_json::Value = response.json().await?;

        let content = response_json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| LLMError::ProviderError("No content in response".to_string()))?
            .to_string();

        // Extract token usage - orchestrator will calculate cost
        let prompt_tokens = response_json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = response_json["usage"]["output_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;
        let total_tokens = prompt_tokens + output_tokens;

        let usage = TokenUsage {
            prompt_tokens,
            output_tokens,
            total_tokens,
            cached_tokens: None, // Anthropic doesn't report cached tokens separately yet
        };

        let finish_reason = match response_json["stop_reason"].as_str() {
            Some("end_turn") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::MaxTokens,
            Some("stop_sequence") => FinishReason::Stop,
            _ => FinishReason::Stop,
        };

        Ok(PromptResponse {
            content,
            model: request.model.clone(),
            usage,
            finish_reason,
            // NO cost_usd field - orchestrator calculates cost
        })
    }

    async fn prompt_stream(
        &self,
        request: &PromptRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PromptChunk>> + Send>>> {
        let messages = vec![json!({
            "role": "user",
            "content": request.prompt,
        })];

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
            "stream": true,
        });

        if let Some(system) = &request.system_prompt {
            body["system"] = json!(system);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            body["top_p"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            body["stop_sequences"] = json!(stops);
        }

        let url = format!("{}/v1/messages", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(LLMError::ProviderError(error_text));
        }

        // Create a stream that parses SSE events and yields PromptChunks
        let byte_stream = response.bytes_stream();

        // State for the stream: byte_stream, parser, pending chunks, finish reason
        let stream = futures::stream::unfold(
            (
                byte_stream,
                SseParser::new(),
                Vec::<PromptChunk>::new(),
                None::<FinishReason>,
            ),
            |(mut byte_stream, mut parser, mut pending_chunks, mut finish_reason)| async move {
                // First, yield any pending chunks
                if !pending_chunks.is_empty() {
                    let chunk = pending_chunks.remove(0);
                    return Some((
                        Ok(chunk),
                        (byte_stream, parser, pending_chunks, finish_reason),
                    ));
                }

                // Then fetch more data from the stream
                loop {
                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            let events = parser.process(&bytes);
                            for event in events {
                                match event {
                                    AnthropicEvent::ContentBlockDelta { delta, .. } => {
                                        if delta.delta_type == "text_delta" {
                                            if let Some(text) = delta.text {
                                                pending_chunks.push(PromptChunk {
                                                    content: text,
                                                    finish_reason: None,
                                                });
                                            }
                                        }
                                    }
                                    AnthropicEvent::MessageDelta { delta, .. } => {
                                        finish_reason =
                                            delta.stop_reason.map(|r| match r.as_str() {
                                                "end_turn" => FinishReason::Stop,
                                                "max_tokens" => FinishReason::MaxTokens,
                                                "stop_sequence" => FinishReason::Stop,
                                                _ => FinishReason::Stop,
                                            });
                                    }
                                    AnthropicEvent::MessageStop => {
                                        // Add final chunk with finish reason
                                        pending_chunks.push(PromptChunk {
                                            content: String::new(),
                                            finish_reason: finish_reason.take(),
                                        });
                                    }
                                    AnthropicEvent::Error { error } => {
                                        return Some((
                                            Err(LLMError::ProviderError(format!(
                                                "{}: {}",
                                                error.error_type, error.message
                                            ))),
                                            (byte_stream, parser, pending_chunks, finish_reason),
                                        ));
                                    }
                                    _ => {}
                                }
                            }

                            // If we have pending chunks, yield the first one
                            if !pending_chunks.is_empty() {
                                let chunk = pending_chunks.remove(0);
                                return Some((
                                    Ok(chunk),
                                    (byte_stream, parser, pending_chunks, finish_reason),
                                ));
                            }
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(LLMError::RequestError(e)),
                                (byte_stream, parser, pending_chunks, finish_reason),
                            ));
                        }
                        None => return None, // Stream ended
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }

    async fn embed(&self, _request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        // Anthropic doesn't have embeddings API
        Err(LLMError::ProviderError(
            "Anthropic does not support embeddings".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_provider_name() {
        let provider = AnthropicProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "anthropic");
    }

    #[tokio::test]
    async fn test_complete_success() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Hello! How can I help you?"
            }],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 7
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = AnthropicProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Hello! How can I help you?");
        assert_eq!(response.model, "claude-3-sonnet-20240229");
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(response.usage.total_tokens, 17);
        assert!(response.usage.cached_tokens.is_none());
        assert!(matches!(response.finish_reason, FinishReason::Stop));
    }

    #[tokio::test]
    async fn test_complete_with_system_prompt() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "id": "msg_124",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Understood!"
            }],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 15,
                "output_tokens": 3
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = AnthropicProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            prompt: "Do you understand?".to_string(),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            max_tokens: Some(100),
            temperature: Some(0.7),
            top_p: Some(0.9),
            stop_sequences: Some(vec!["STOP".to_string()]),
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Understood!");
        assert_eq!(response.usage.prompt_tokens, 15);
        assert_eq!(response.usage.output_tokens, 3);
    }

    #[tokio::test]
    async fn test_complete_max_tokens_finish() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "id": "msg_125",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "This is a truncated"
            }],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "max_tokens",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = AnthropicProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            prompt: "Tell me a story".to_string(),
            system_prompt: None,
            max_tokens: Some(5),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let response = provider.prompt(&request).await.unwrap();

        assert!(matches!(response.finish_reason, FinishReason::MaxTokens));
    }

    #[tokio::test]
    async fn test_complete_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider =
            AnthropicProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let result = provider.prompt(&request).await;
        assert!(result.is_err());
        if let Err(LLMError::ProviderError(msg)) = result {
            assert!(msg.contains("Invalid API key"));
        }
    }

    #[tokio::test]
    async fn test_embed_not_supported() {
        let provider = AnthropicProvider::new("test-key".to_string());
        let request = EmbeddingRequest {
            model: "claude-3-sonnet".to_string(),
            input: vec!["test".to_string()],
        };

        let result = provider.embed(&request).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not support embeddings")
        );
    }

    #[tokio::test]
    async fn test_streaming_success() {
        let mock_server = MockServer::start().await;

        // SSE response body simulating Anthropic streaming
        let sse_body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[],"model":"claude-3-sonnet-20240229","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}

event: message_stop
data: {"type":"message_stop"}

"#;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&mock_server)
            .await;

        let provider = AnthropicProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            prompt: "Say hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let stream = provider.prompt_stream(&request).await.unwrap();

        // Collect all chunks from the stream
        let chunks: Vec<_> = stream.collect().await;

        // Should have received text chunks: "Hello", "!", and final empty chunk with finish reason
        assert!(
            chunks.len() >= 2,
            "Expected at least 2 chunks, got {}",
            chunks.len()
        );

        // First chunk should be "Hello"
        let first = chunks[0].as_ref().unwrap();
        assert_eq!(first.content, "Hello");
        assert!(first.finish_reason.is_none());

        // Second chunk should be "!"
        let second = chunks[1].as_ref().unwrap();
        assert_eq!(second.content, "!");
        assert!(second.finish_reason.is_none());

        // Last chunk should have finish reason
        let last = chunks.last().unwrap().as_ref().unwrap();
        assert!(matches!(last.finish_reason, Some(FinishReason::Stop)));
    }

    #[tokio::test]
    async fn test_streaming_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider =
            AnthropicProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let result = provider.prompt_stream(&request).await;
        assert!(result.is_err());
    }
}
