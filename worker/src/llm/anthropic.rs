use super::provider::*;
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
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
        _request: &PromptRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PromptChunk>> + Send>>> {
        // Post-MVP: Implement streaming using Server-Sent Events (SSE)
        Err(LLMError::ProviderError(
            "Streaming support is post-MVP".to_string(),
        ))
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

        let provider = AnthropicProvider::with_base_url("invalid-key".to_string(), mock_server.uri());

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
    async fn test_streaming_not_supported() {
        let provider = AnthropicProvider::new("test-key".to_string());
        let request = PromptRequest {
            model: "claude-3-sonnet".to_string(),
            prompt: "test".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let result = provider.prompt_stream(&request).await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Streaming support is post-MVP"));
        }
    }
}
