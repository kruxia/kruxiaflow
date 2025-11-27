use super::provider::*;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::pin::Pin;

pub struct GoogleProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(
            api_key,
            "https://generativelanguage.googleapis.com".to_string(),
        )
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
        }
    }
}

// SSE event types for Google Gemini streaming API
#[derive(Debug, Deserialize)]
struct GeminiStreamChunk {
    candidates: Option<Vec<GeminiCandidate>>,
    #[allow(dead_code)]
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
    #[allow(dead_code)]
    index: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    parts: Option<Vec<GeminiPart>>,
    #[allow(dead_code)]
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
    #[allow(dead_code)]
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[allow(dead_code)]
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
    #[allow(dead_code)]
    #[serde(rename = "totalTokenCount")]
    total_token_count: Option<u32>,
}

/// Parse a single SSE line (after "data: " prefix) into a GeminiStreamChunk
fn parse_sse_data(data: &str) -> Option<GeminiStreamChunk> {
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
    fn process(&mut self, bytes: &Bytes) -> Vec<GeminiStreamChunk> {
        let text = String::from_utf8_lossy(bytes);
        self.buffer.push_str(&text);

        let mut events = Vec::new();

        // Process complete lines (Google uses NDJSON-like format with newlines)
        while let Some(pos) = self.buffer.find("\r\n") {
            let chunk = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();

            // Parse format: "data: {...}" or just "{...}"
            let data = chunk.strip_prefix("data: ").unwrap_or(&chunk);
            if let Some(event) = parse_sse_data(data) {
                events.push(event);
            }
        }

        // Also check for single newlines
        while let Some(pos) = self.buffer.find('\n') {
            let line = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 1..].to_string();

            let data = line.strip_prefix("data: ").unwrap_or(&line);
            if let Some(event) = parse_sse_data(data) {
                events.push(event);
            }
        }

        events
    }
}

#[async_trait]
impl LLMProvider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
    }

    async fn prompt(&self, request: &PromptRequest) -> Result<PromptResponse> {
        let mut contents = vec![];

        // Add system instruction if provided
        let mut body = if let Some(system) = &request.system_prompt {
            json!({
                "contents": [],
                "systemInstruction": {
                    "parts": [{"text": system}]
                }
            })
        } else {
            json!({
                "contents": []
            })
        };

        // Add user prompt
        contents.push(json!({
            "role": "user",
            "parts": [{"text": request.prompt}]
        }));

        body["contents"] = json!(contents);

        // Add generation config
        let mut generation_config = json!({});

        if let Some(max_tokens) = request.max_tokens {
            generation_config["maxOutputTokens"] = json!(max_tokens);
        }

        if let Some(temp) = request.temperature {
            generation_config["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            generation_config["topP"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            generation_config["stopSequences"] = json!(stops);
        }

        if !generation_config.as_object().unwrap().is_empty() {
            body["generationConfig"] = generation_config;
        }

        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.base_url, request.model
        );

        let response = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(LLMError::ProviderError(error_text));
        }

        let response_json: serde_json::Value = response.json().await?;

        let content = response_json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| LLMError::ProviderError("No content in response".to_string()))?
            .to_string();

        // Extract token usage - orchestrator will calculate cost
        let prompt_tokens = response_json["usageMetadata"]["promptTokenCount"]
            .as_u64()
            .unwrap_or(0) as u32;
        let output_tokens = response_json["usageMetadata"]["candidatesTokenCount"]
            .as_u64()
            .unwrap_or(0) as u32;
        let total_tokens = response_json["usageMetadata"]["totalTokenCount"]
            .as_u64()
            .unwrap_or((prompt_tokens + output_tokens) as u64) as u32;

        let usage = TokenUsage {
            prompt_tokens,
            output_tokens,
            total_tokens,
            cached_tokens: None,
        };

        let finish_reason = match response_json["candidates"][0]["finishReason"].as_str() {
            Some("STOP") => FinishReason::Stop,
            Some("MAX_TOKENS") => FinishReason::MaxTokens,
            Some("SAFETY") => FinishReason::ContentFilter,
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
        let mut contents = vec![];

        // Add user prompt
        contents.push(json!({
            "role": "user",
            "parts": [{"text": request.prompt}]
        }));

        // Build request body
        let mut body = if let Some(system) = &request.system_prompt {
            json!({
                "contents": contents,
                "systemInstruction": {
                    "parts": [{"text": system}]
                }
            })
        } else {
            json!({
                "contents": contents
            })
        };

        // Add generation config
        let mut generation_config = json!({});

        if let Some(max_tokens) = request.max_tokens {
            generation_config["maxOutputTokens"] = json!(max_tokens);
        }

        if let Some(temp) = request.temperature {
            generation_config["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            generation_config["topP"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            generation_config["stopSequences"] = json!(stops);
        }

        if !generation_config.as_object().unwrap().is_empty() {
            body["generationConfig"] = generation_config;
        }

        // Use streamGenerateContent endpoint for streaming
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.base_url, request.model
        );

        let response = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
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
                                if let Some(candidates) = &event.candidates {
                                    if let Some(candidate) = candidates.first() {
                                        // Extract text from content parts
                                        if let Some(content) = &candidate.content {
                                            if let Some(parts) = &content.parts {
                                                for part in parts {
                                                    if let Some(text) = &part.text {
                                                        if !text.is_empty() {
                                                            pending_chunks.push(PromptChunk {
                                                                content: text.clone(),
                                                                finish_reason: None,
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        // Check for finish reason
                                        if let Some(reason) = &candidate.finish_reason {
                                            finish_reason = Some(match reason.as_str() {
                                                "STOP" => FinishReason::Stop,
                                                "MAX_TOKENS" => FinishReason::MaxTokens,
                                                "SAFETY" => FinishReason::ContentFilter,
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
        let url = format!(
            "{}/v1beta/models/{}:embedContent",
            self.base_url, request.model
        );

        let mut embeddings = Vec::new();
        let mut total_prompt_tokens = 0u32;

        // Process each input separately
        for input_text in &request.input {
            let body = json!({
                "content": {
                    "parts": [{"text": input_text}]
                }
            });

            let response = self
                .client
                .post(&url)
                .header("x-goog-api-key", &self.api_key)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;

            if !response.status().is_success() {
                let error_text = response.text().await?;
                return Err(LLMError::ProviderError(error_text));
            }

            let response_json: serde_json::Value = response.json().await?;

            let embedding: Vec<f64> = response_json["embedding"]["values"]
                .as_array()
                .ok_or_else(|| LLMError::ProviderError("No embedding in response".to_string()))?
                .iter()
                .map(|v| {
                    v.as_f64().ok_or_else(|| {
                        LLMError::ProviderError("Invalid embedding value".to_string())
                    })
                })
                .collect::<Result<Vec<f64>>>()?;

            embeddings.push(embedding);

            // Google API doesn't return token counts for embeddings
            // Simple estimation: ~4 chars per token
            let tokens = (input_text.len() as f64 / 4.0).ceil() as u32;
            total_prompt_tokens += tokens;
        }

        let usage = TokenUsage {
            prompt_tokens: total_prompt_tokens,
            output_tokens: 0, // Embeddings don't have output tokens
            total_tokens: total_prompt_tokens,
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
    use wiremock::matchers::{header, method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_provider_name() {
        let provider = GoogleProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "google");
    }

    #[tokio::test]
    async fn test_complete_success() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "Hello! I'm here to help."
                    }],
                    "role": "model"
                },
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 7,
                "totalTokenCount": 17
            }
        });

        Mock::given(method("POST"))
            .and(path_regex(r"/v1beta/models/.*:generateContent"))
            .and(header("x-goog-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gemini-pro".to_string(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Hello! I'm here to help.");
        assert_eq!(response.model, "gemini-pro");
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(response.usage.total_tokens, 17);
        assert!(response.usage.cached_tokens.is_none());
        assert!(matches!(response.finish_reason, FinishReason::Stop));
    }

    #[tokio::test]
    async fn test_complete_with_system_instruction() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "I understand completely."
                    }],
                    "role": "model"
                },
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 25,
                "candidatesTokenCount": 4,
                "totalTokenCount": 29
            }
        });

        Mock::given(method("POST"))
            .and(path_regex(r"/v1beta/models/.*:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gemini-pro".to_string(),
            prompt: "Do you understand?".to_string(),
            system_prompt: Some("You are a helpful AI assistant.".to_string()),
            max_tokens: Some(100),
            temperature: Some(0.7),
            top_p: Some(0.9),
            stop_sequences: Some(vec!["STOP".to_string()]),
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "I understand completely.");
        assert_eq!(response.usage.prompt_tokens, 25);
        assert_eq!(response.usage.output_tokens, 4);
    }

    #[tokio::test]
    async fn test_complete_max_tokens_finish() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "This is truncated"
                    }],
                    "role": "model"
                },
                "finishReason": "MAX_TOKENS",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5,
                "totalTokenCount": 15
            }
        });

        Mock::given(method("POST"))
            .and(path_regex(r"/v1beta/models/.*:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gemini-pro".to_string(),
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
            .and(path_regex(r"/v1beta/models/.*:generateContent"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gemini-pro".to_string(),
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

        let mock_response_1 = json!({
            "embedding": {
                "values": [0.1, 0.2, 0.3, 0.4]
            }
        });

        let mock_response_2 = json!({
            "embedding": {
                "values": [0.5, 0.6, 0.7, 0.8]
            }
        });

        Mock::given(method("POST"))
            .and(path_regex(r"/v1beta/models/.*:embedContent"))
            .and(header("x-goog-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response_1))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(r"/v1beta/models/.*:embedContent"))
            .and(header("x-goog-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response_2))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = EmbeddingRequest {
            model: "text-embedding-004".to_string(),
            input: vec!["Hello".to_string(), "World".to_string()],
        };

        let response = provider.embed(&request).await.unwrap();

        assert_eq!(response.embeddings.len(), 2);
        assert_eq!(response.embeddings[0], vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(response.embeddings[1], vec![0.5, 0.6, 0.7, 0.8]);
        assert_eq!(response.model, "text-embedding-004");
        assert_eq!(response.usage.output_tokens, 0);
        // Token count is estimated, should be > 0
        assert!(response.usage.prompt_tokens > 0);
    }

    #[tokio::test]
    async fn test_embed_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r"/v1beta/models/.*:embedContent"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = EmbeddingRequest {
            model: "text-embedding-004".to_string(),
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

        // SSE response body simulating Google Gemini streaming
        let sse_body = r#"data: {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"index":0}]}

data: {"candidates":[{"content":{"parts":[{"text":"!"}],"role":"model"},"index":0}]}

data: {"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":2,"totalTokenCount":12}}

"#;

        Mock::given(method("POST"))
            .and(path(
                "/v1beta/models/gemini-1.5-flash:streamGenerateContent",
            ))
            .and(header("x-goog-api-key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("test-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gemini-1.5-flash".to_string(),
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

        // Should have received text chunks: "Hello", "!", and final chunk with finish reason
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
            .and(wiremock::matchers::path_regex(
                "/v1beta/models/.*:streamGenerateContent",
            ))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
            .mount(&mock_server)
            .await;

        let provider = GoogleProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

        let request = PromptRequest {
            model: "gemini-1.5-flash".to_string(),
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
