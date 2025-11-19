use super::provider::*;
use async_trait::async_trait;
use futures::stream::Stream;
use reqwest::Client;
use serde_json::json;
use std::pin::Pin;

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    api_key: Option<String>, // Optional for authenticated Ollama instances
}

impl OllamaProvider {
    pub fn new(base_url: Option<String>, api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
            api_key,
        }
    }
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn prompt(&self, request: &PromptRequest) -> Result<PromptResponse> {
        let mut prompt = request.prompt.clone();

        // Prepend system prompt if provided
        if let Some(system) = &request.system_prompt {
            prompt = format!("{}\n\n{}", system, prompt);
        }

        let mut body = json!({
            "model": request.model,
            "prompt": prompt,
            "stream": false,
        });

        let mut options = json!({});

        if let Some(max_tokens) = request.max_tokens {
            options["num_predict"] = json!(max_tokens);
        }

        if let Some(temp) = request.temperature {
            options["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.top_p {
            options["top_p"] = json!(top_p);
        }

        if let Some(stops) = &request.stop_sequences {
            options["stop"] = json!(stops);
        }

        if !options.as_object().unwrap().is_empty() {
            body["options"] = options;
        }

        let url = format!("{}/api/generate", self.base_url);

        let mut req_builder = self
            .client
            .post(&url)
            .header("content-type", "application/json");

        // Add authentication header if API key is provided
        if let Some(api_key) = &self.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req_builder.json(&body).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(LLMError::ProviderError(error_text));
        }

        let response_json: serde_json::Value = response.json().await?;

        let content = response_json["response"]
            .as_str()
            .ok_or_else(|| LLMError::ProviderError("No content in response".to_string()))?
            .to_string();

        // Extract token usage - Ollama uses different field names
        let prompt_tokens = response_json["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
        let output_tokens = response_json["eval_count"].as_u64().unwrap_or(0) as u32;
        let total_tokens = prompt_tokens + output_tokens;

        let usage = TokenUsage {
            prompt_tokens,
            output_tokens,
            total_tokens,
            cached_tokens: None,
        };

        // Ollama doesn't provide a finish_reason, assume Stop if done
        let finish_reason = if response_json["done"].as_bool().unwrap_or(false) {
            FinishReason::Stop
        } else {
            FinishReason::Error
        };

        Ok(PromptResponse {
            content,
            model: request.model.clone(),
            usage,
            finish_reason,
            // NO cost_usd - orchestrator calculates (Ollama is zero cost)
        })
    }

    async fn prompt_stream(
        &self,
        _request: &PromptRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PromptChunk>> + Send>>> {
        // Post-MVP: Implement streaming using Ollama's streaming API
        Err(LLMError::ProviderError(
            "Streaming support is post-MVP".to_string(),
        ))
    }

    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        let url = format!("{}/api/embeddings", self.base_url);

        let mut embeddings = Vec::new();
        let mut total_prompt_tokens = 0u32;

        // Process each input separately
        for input_text in &request.input {
            let body = json!({
                "model": request.model,
                "prompt": input_text,
            });

            let mut req_builder = self
                .client
                .post(&url)
                .header("content-type", "application/json");

            // Add authentication header if API key is provided
            if let Some(api_key) = &self.api_key {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
            }

            let response = req_builder.json(&body).send().await?;

            if !response.status().is_success() {
                let error_text = response.text().await?;
                return Err(LLMError::ProviderError(error_text));
            }

            let response_json: serde_json::Value = response.json().await?;

            let embedding: Vec<f64> = response_json["embedding"]
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

            // Ollama doesn't return token counts for embeddings
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
            // NO cost_usd - orchestrator calculates (Ollama is zero cost)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_provider_name() {
        let provider = OllamaProvider::new(None, None);
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_default_base_url() {
        let provider = OllamaProvider::new(None, None);
        assert_eq!(provider.base_url, "http://localhost:11434");
    }

    #[test]
    fn test_custom_base_url() {
        let provider = OllamaProvider::new(Some("http://ollama:11434".to_string()), None);
        assert_eq!(provider.base_url, "http://ollama:11434");
    }

    #[tokio::test]
    async fn test_complete_success() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "model": "llama2",
            "created_at": "2023-08-04T19:22:45.499127Z",
            "response": "Hello! How can I help you today?",
            "done": true,
            "prompt_eval_count": 10,
            "eval_count": 8
        });

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), None);

        let request = PromptRequest {
            model: "llama2".to_string(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Hello! How can I help you today?");
        assert_eq!(response.model, "llama2");
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.output_tokens, 8);
        assert_eq!(response.usage.total_tokens, 18);
        assert!(response.usage.cached_tokens.is_none());
        assert!(matches!(response.finish_reason, FinishReason::Stop));
    }

    #[tokio::test]
    async fn test_complete_with_system_prompt() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "model": "llama2",
            "created_at": "2023-08-04T19:22:45.499127Z",
            "response": "I understand your instructions.",
            "done": true,
            "prompt_eval_count": 25,
            "eval_count": 5
        });

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), None);

        let request = PromptRequest {
            model: "llama2".to_string(),
            prompt: "Do you understand?".to_string(),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            max_tokens: Some(100),
            temperature: Some(0.7),
            top_p: Some(0.9),
            stop_sequences: Some(vec!["STOP".to_string()]),
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "I understand your instructions.");
        assert_eq!(response.usage.prompt_tokens, 25);
        assert_eq!(response.usage.output_tokens, 5);
    }

    #[tokio::test]
    async fn test_complete_with_api_key() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "model": "llama2",
            "created_at": "2023-08-04T19:22:45.499127Z",
            "response": "Authenticated response",
            "done": true,
            "prompt_eval_count": 5,
            "eval_count": 3
        });

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), Some("test-key".to_string()));

        let request = PromptRequest {
            model: "llama2".to_string(),
            prompt: "Test".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let response = provider.prompt(&request).await.unwrap();

        assert_eq!(response.content, "Authenticated response");
    }

    #[tokio::test]
    async fn test_complete_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Model not found"))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), None);

        let request = PromptRequest {
            model: "nonexistent-model".to_string(),
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
            assert!(msg.contains("Model not found"));
        }
    }

    #[tokio::test]
    async fn test_embed_success() {
        let mock_server = MockServer::start().await;

        let mock_response_1 = json!({
            "embedding": [0.1, 0.2, 0.3, 0.4]
        });

        let mock_response_2 = json!({
            "embedding": [0.5, 0.6, 0.7, 0.8]
        });

        Mock::given(method("POST"))
            .and(path("/api/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response_1))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response_2))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), None);

        let request = EmbeddingRequest {
            model: "nomic-embed-text".to_string(),
            input: vec!["Hello".to_string(), "World".to_string()],
        };

        let response = provider.embed(&request).await.unwrap();

        assert_eq!(response.embeddings.len(), 2);
        assert_eq!(response.embeddings[0], vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(response.embeddings[1], vec![0.5, 0.6, 0.7, 0.8]);
        assert_eq!(response.model, "nomic-embed-text");
        assert_eq!(response.usage.output_tokens, 0);
        // Token count is estimated, should be > 0
        assert!(response.usage.prompt_tokens > 0);
    }

    #[tokio::test]
    async fn test_embed_with_api_key() {
        let mock_server = MockServer::start().await;

        let mock_response = json!({
            "embedding": [0.1, 0.2, 0.3, 0.4]
        });

        Mock::given(method("POST"))
            .and(path("/api/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), Some("test-key".to_string()));

        let request = EmbeddingRequest {
            model: "nomic-embed-text".to_string(),
            input: vec!["Hello".to_string()],
        };

        let response = provider.embed(&request).await.unwrap();

        assert_eq!(response.embeddings.len(), 1);
        assert_eq!(response.embeddings[0], vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[tokio::test]
    async fn test_embed_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/embeddings"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Model not found"))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new(Some(mock_server.uri()), None);

        let request = EmbeddingRequest {
            model: "nonexistent-model".to_string(),
            input: vec!["Hello".to_string()],
        };

        let result = provider.embed(&request).await;
        assert!(result.is_err());
        if let Err(LLMError::ProviderError(msg)) = result {
            assert!(msg.contains("Model not found"));
        }
    }

    #[tokio::test]
    async fn test_streaming_not_supported() {
        let provider = OllamaProvider::new(None, None);
        let request = PromptRequest {
            model: "llama2".to_string(),
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
