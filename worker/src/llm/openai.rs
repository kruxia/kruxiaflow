use super::provider::*;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, "https://api.openai.com".to_string())
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
        }
    }
}

// SSE event types for OpenAI streaming API
#[derive(Debug, Deserialize)]
struct OpenAIStreamChunk {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    object: String,
    #[allow(dead_code)]
    created: u64,
    #[allow(dead_code)]
    model: String,
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[allow(dead_code)]
    index: u32,
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[allow(dead_code)]
    role: Option<String>,
    content: Option<String>,
}

/// Parse a single SSE line (after "data: " prefix) into an OpenAIStreamChunk
fn parse_sse_data(data: &str) -> Option<OpenAIStreamChunk> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return None;
    }
    serde_json::from_str(trimmed).ok()
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
    fn process(&mut self, bytes: &Bytes) -> Vec<OpenAIStreamChunk> {
        let text = String::from_utf8_lossy(bytes);
        self.buffer.push_str(&text);

        let mut events = Vec::new();

        // Process complete lines (OpenAI uses single newlines between data lines)
        while let Some(pos) = self.buffer.find("\n\n") {
            let chunk = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();

            // Parse SSE format: "data: {...}"
            for line in chunk.lines() {
                if let Some(data) = line.strip_prefix("data: ")
                    && let Some(event) = parse_sse_data(data)
                {
                    events.push(event);
                }
            }
        }

        // Also check for single-newline separated lines (OpenAI sometimes does this)
        while let Some(pos) = self.buffer.find('\n') {
            let line = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ")
                && let Some(event) = parse_sse_data(data)
            {
                events.push(event);
            }
        }

        events
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn prompt(&self, request: &PromptRequest) -> Result<PromptResponse> {
        let mut messages = vec![];

        // Add system prompt if provided
        if let Some(system) = &request.system_prompt {
            messages.push(json!({
                "role": "system",
                "content": system,
            }));
        }

        // Add user prompt
        messages.push(json!({
            "role": "user",
            "content": request.prompt,
        }));

        let mut body = json!({
            "model": request.model,
            "messages": messages,
        });

        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            body["top_p"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            body["stop"] = json!(stops);
        }

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(LLMError::ProviderError(error_text));
        }

        let response_json: serde_json::Value = response.json().await?;

        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| LLMError::ProviderError("No content in response".to_string()))?
            .to_string();

        // Extract token usage - orchestrator will calculate cost
        let prompt_tokens = response_json["usage"]["prompt_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;
        let output_tokens = response_json["usage"]["completion_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;
        let total_tokens = response_json["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32;

        let usage = TokenUsage {
            prompt_tokens,
            output_tokens,
            total_tokens,
            cached_tokens: None,
        };

        let finish_reason = match response_json["choices"][0]["finish_reason"].as_str() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::MaxTokens,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        };

        Ok(PromptResponse {
            content,
            model: request.model.clone(),
            usage,
            finish_reason,
            // NO cost_usd - orchestrator calculates
        })
    }

    async fn prompt_stream(
        &self,
        request: &PromptRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PromptChunk>> + Send>>> {
        let mut messages = vec![];

        // Add system prompt if provided
        if let Some(system) = &request.system_prompt {
            messages.push(json!({
                "role": "system",
                "content": system,
            }));
        }

        // Add user prompt
        messages.push(json!({
            "role": "user",
            "content": request.prompt,
        }));

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "stream": true,
        });

        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            body["top_p"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            body["stop"] = json!(stops);
        }

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
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
                                if let Some(choice) = event.choices.first() {
                                    // Check for content delta
                                    if let Some(content) = &choice.delta.content
                                        && !content.is_empty()
                                    {
                                        pending_chunks.push(PromptChunk {
                                            content: content.clone(),
                                            finish_reason: None,
                                        });
                                    }

                                    // Check for finish reason
                                    if let Some(reason) = &choice.finish_reason {
                                        finish_reason = Some(match reason.as_str() {
                                            "stop" => FinishReason::Stop,
                                            "length" => FinishReason::MaxTokens,
                                            "content_filter" => FinishReason::ContentFilter,
                                            _ => FinishReason::Stop,
                                        });

                                        // Add final chunk with finish reason
                                        pending_chunks.push(PromptChunk {
                                            content: String::new(),
                                            finish_reason: finish_reason.take(),
                                        });
                                    }
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

    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        let body = json!({
            "model": request.model,
            "input": request.input,
        });

        let url = format!("{}/v1/embeddings", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(LLMError::ProviderError(error_text));
        }

        let response_json: serde_json::Value = response.json().await?;

        let embeddings: Vec<Vec<f64>> = response_json["data"]
            .as_array()
            .ok_or_else(|| LLMError::ProviderError("No embeddings in response".to_string()))?
            .iter()
            .map(|item| {
                item["embedding"]
                    .as_array()
                    .ok_or_else(|| LLMError::ProviderError("Invalid embedding format".to_string()))
                    .and_then(|arr| {
                        arr.iter()
                            .map(|v| {
                                v.as_f64().ok_or_else(|| {
                                    LLMError::ProviderError("Invalid embedding value".to_string())
                                })
                            })
                            .collect::<Result<Vec<f64>>>()
                    })
            })
            .collect::<Result<Vec<Vec<f64>>>>()?;

        // Extract token usage
        let prompt_tokens = response_json["usage"]["prompt_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;
        let total_tokens = response_json["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32;

        let usage = TokenUsage {
            prompt_tokens,
            output_tokens: 0, // Embeddings don't have output tokens
            total_tokens,
            cached_tokens: None,
        };

        Ok(EmbeddingResponse {
            embeddings,
            model: request.model.clone(),
            usage,
            // NO cost_usd - orchestrator calculates
        })
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
        let provider = OpenAIProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "openai");
    }

    #[tokio::test]
    async fn test_complete_success() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello! How can I assist you?"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 7,
                "total_tokens": 17
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gpt-4".to_string(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Hello! How can I assist you?");
        assert_eq!(response.model, "gpt-4");
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
            "id": "chatcmpl-124",
            "object": "chat.completion",
            "created": 1677652289,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Yes, I understand."
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 5,
                "total_tokens": 25
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gpt-4".to_string(),
            prompt: "Do you understand?".to_string(),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            max_tokens: Some(100),
            temperature: Some(0.7),
            top_p: Some(0.9),
            stop_sequences: Some(vec!["STOP".to_string()]),
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Yes, I understand.");
        assert_eq!(response.usage.prompt_tokens, 20);
        assert_eq!(response.usage.output_tokens, 5);
    }

    #[tokio::test]
    async fn test_complete_max_tokens_finish() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "id": "chatcmpl-125",
            "object": "chat.completion",
            "created": 1677652290,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "This is a truncated"
                },
                "finish_reason": "length"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gpt-4".to_string(),
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
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gpt-4".to_string(),
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
    async fn test_embed_success() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "object": "list",
            "data": [
                {
                    "object": "embedding",
                    "embedding": [0.1, 0.2, 0.3, 0.4],
                    "index": 0
                },
                {
                    "object": "embedding",
                    "embedding": [0.5, 0.6, 0.7, 0.8],
                    "index": 1
                }
            ],
            "model": "text-embedding-ada-002",
            "usage": {
                "prompt_tokens": 10,
                "total_tokens": 10
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = EmbeddingRequest {
            model: "text-embedding-ada-002".to_string(),
            input: vec!["Hello".to_string(), "World".to_string()],
        };

        let response = provider.embed(&request).await.unwrap();

        assert_eq!(response.embeddings.len(), 2);
        assert_eq!(response.embeddings[0], vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(response.embeddings[1], vec![0.5, 0.6, 0.7, 0.8]);
        assert_eq!(response.model, "text-embedding-ada-002");
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.output_tokens, 0);
        assert_eq!(response.usage.total_tokens, 10);
    }

    #[tokio::test]
    async fn test_embed_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = EmbeddingRequest {
            model: "text-embedding-ada-002".to_string(),
            input: vec!["Hello".to_string()],
        };

        let result = provider.embed(&request).await;
        assert!(result.is_err());
        if let Err(LLMError::ProviderError(msg)) = result {
            assert!(msg.contains("Invalid API key"));
        }
    }

    #[tokio::test]
    async fn test_streaming_success() {
        let mock_server = MockServer::start().await;

        // SSE response body simulating OpenAI streaming
        let sse_body = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1677652288,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1677652288,"model":"gpt-4","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1677652288,"model":"gpt-4","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1677652288,"model":"gpt-4","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]

"#;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gpt-4".to_string(),
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
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider = OpenAIProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gpt-4".to_string(),
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
