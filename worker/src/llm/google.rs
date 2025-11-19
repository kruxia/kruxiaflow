use super::provider::*;
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
use serde_json::json;
use std::pin::Pin;

pub struct GoogleProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, "https://generativelanguage.googleapis.com".to_string())
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
        _request: &PromptRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PromptChunk>> + Send>>> {
        // Post-MVP: Implement streaming using Server-Sent Events (SSE)
        Err(LLMError::ProviderError(
            "Streaming support is post-MVP".to_string(),
        ))
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
    use wiremock::matchers::{header, method, path_regex};
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
    async fn test_streaming_not_supported() {
        let provider = GoogleProvider::new("test-key".to_string());
        let request = PromptRequest {
            model: "gemini-pro".to_string(),
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
