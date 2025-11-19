use crate::activity_result::ActivityResult;
use crate::llm::{
    AnthropicProvider, PromptRequest, EmbeddingRequest, GoogleProvider, LLMError, LLMProvider,
    OllamaProvider, OpenAIProvider,
};
use crate::registry::ActivityImpl;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;

// ============================================================================
// Provider Configuration
// ============================================================================

/// Provider configuration from environment variables
struct ProviderConfig {
    anthropic_api_key: Option<String>,
    openai_api_key: Option<String>,
    google_api_key: Option<String>,
    ollama_base_url: Option<String>,
    ollama_api_key: Option<String>,
}

impl ProviderConfig {
    fn from_env() -> Self {
        Self {
            anthropic_api_key: env::var("ANTHROPIC_API_KEY").ok(),
            openai_api_key: env::var("OPENAI_API_KEY").ok(),
            google_api_key: env::var("GOOGLE_API_KEY").ok(),
            ollama_base_url: env::var("OLLAMA_BASE_URL").ok(),
            ollama_api_key: env::var("OLLAMA_API_KEY").ok(),
        }
    }

    fn create_provider(&self, provider: &str) -> Result<Arc<dyn LLMProvider>> {
        match provider.to_lowercase().as_str() {
            "anthropic" => {
                let api_key = self
                    .anthropic_api_key
                    .clone()
                    .ok_or_else(|| anyhow!("ANTHROPIC_API_KEY not set"))?;
                Ok(Arc::new(AnthropicProvider::new(api_key)))
            }
            "openai" => {
                let api_key = self
                    .openai_api_key
                    .clone()
                    .ok_or_else(|| anyhow!("OPENAI_API_KEY not set"))?;
                Ok(Arc::new(OpenAIProvider::new(api_key)))
            }
            "google" => {
                let api_key = self
                    .google_api_key
                    .clone()
                    .ok_or_else(|| anyhow!("GOOGLE_API_KEY not set"))?;
                Ok(Arc::new(GoogleProvider::new(api_key)))
            }
            "ollama" => Ok(Arc::new(OllamaProvider::new(
                self.ollama_base_url.clone(),
                self.ollama_api_key.clone(),
            ))),
            _ => Err(anyhow!("Unknown provider: {}", provider)),
        }
    }
}

// ============================================================================
// Fallback Chain
// ============================================================================

/// Fallback chain configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackChain {
    /// List of (provider, model) pairs to try in order
    pub provider_models: Vec<(String, String)>,
}

impl FallbackChain {
    /// Execute a prompt request with fallback
    pub async fn prompt(&self, base_request: &PromptRequest) -> Result<PromptResponse> {
        let config = ProviderConfig::from_env();
        let mut last_error = None;

        for (provider_name, model_name) in &self.provider_models {
            match config.create_provider(provider_name) {
                Ok(provider) => {
                    // Create request with the specific model for this provider
                    let request = PromptRequest {
                        model: model_name.clone(),
                        prompt: base_request.prompt.clone(),
                        system_prompt: base_request.system_prompt.clone(),
                        max_tokens: base_request.max_tokens,
                        temperature: base_request.temperature,
                        top_p: base_request.top_p,
                        stop_sequences: base_request.stop_sequences.clone(),
                    };

                    match provider.prompt(&request).await {
                        Ok(response) => {
                            return Ok(PromptResponse {
                                content: response.content,
                                model: response.model,
                                provider: provider_name.clone(),
                                usage: response.usage,
                                finish_reason: response.finish_reason,
                            })
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Provider {}/{} failed: {}. Trying next provider...",
                                provider_name,
                                model_name,
                                e
                            );
                            last_error = Some(e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create provider {}: {}. Trying next provider...",
                        provider_name,
                        e
                    );
                    last_error = Some(LLMError::ProviderError(e.to_string()));
                }
            }
        }

        Err(anyhow!(
            "All providers failed. Last error: {}",
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "No providers configured".to_string())
        ))
    }

    /// Execute an embedding request with fallback
    pub async fn embed(&self, base_request: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        let config = ProviderConfig::from_env();
        let mut last_error = None;

        for (provider_name, model_name) in &self.provider_models {
            match config.create_provider(provider_name) {
                Ok(provider) => {
                    // Create request with the specific model for this provider
                    let request = EmbeddingRequest {
                        model: model_name.clone(),
                        input: base_request.input.clone(),
                    };

                    match provider.embed(&request).await {
                        Ok(response) => {
                            return Ok(EmbeddingResponse {
                                embeddings: response.embeddings,
                                model: response.model,
                                provider: provider_name.clone(),
                                usage: response.usage,
                            })
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Provider {}/{} failed: {}. Trying next provider...",
                                provider_name,
                                model_name,
                                e
                            );
                            last_error = Some(e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create provider {}: {}. Trying next provider...",
                        provider_name,
                        e
                    );
                    last_error = Some(LLMError::ProviderError(e.to_string()));
                }
            }
        }

        Err(anyhow!(
            "All providers failed. Last error: {}",
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "No providers configured".to_string())
        ))
    }
}

/// Completion response with provider information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResponse {
    pub content: String,
    pub model: String,
    pub provider: String,
    pub usage: crate::llm::TokenUsage,
    pub finish_reason: crate::llm::FinishReason,
}

/// Embedding response with provider information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub embeddings: Vec<Vec<f64>>,
    pub model: String,
    pub provider: String,
    pub usage: crate::llm::TokenUsage,
}

// ============================================================================
// LLM Prompt Activity
// ============================================================================

/// LLM Prompt Activity parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMPromptParams {
    /// Model specification in "provider/model" format
    /// Can be a single string or array for fallback
    /// Examples:
    ///   - "anthropic/claude-3-5-sonnet-20241022"
    ///   - ["anthropic/claude-3-5-sonnet-20241022", "openai/gpt-4", "google/gemini-1.5-pro"]
    pub model: ModelSpec,

    /// The prompt text
    pub prompt: String,

    /// Optional system prompt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Temperature (0.0-2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Top-p sampling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Stop sequences
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
}

/// Model specification - can be single model or fallback chain
/// Format: "provider/model"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelSpec {
    Single(String),
    Fallback(Vec<String>),
}

impl ModelSpec {
    /// Parse "provider/model" string into tuple
    fn parse_provider_model(s: &str) -> Result<(String, String)> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(anyhow!(
                "Invalid model format. Expected 'provider/model', got '{}'",
                s
            ));
        }
        Ok((parts[0].to_string(), parts[1].to_string()))
    }

    fn to_fallback_chain(&self) -> Result<FallbackChain> {
        match self {
            ModelSpec::Single(model_str) => {
                let (provider, model) = Self::parse_provider_model(model_str)?;
                Ok(FallbackChain {
                    provider_models: vec![(provider, model)],
                })
            }
            ModelSpec::Fallback(model_strs) => {
                let mut provider_models = Vec::new();
                for model_str in model_strs {
                    let (provider, model) = Self::parse_provider_model(model_str)?;
                    provider_models.push((provider, model));
                }
                Ok(FallbackChain { provider_models })
            }
        }
    }
}

/// LLM Prompt Activity implementation
pub struct LLMPromptActivity;

impl LLMPromptActivity {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LLMPromptActivity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ActivityImpl for LLMPromptActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        let params: LLMPromptParams = serde_json::from_value(parameters)
            .context("Failed to parse LLM prompt parameters")?;

        // Convert model spec to fallback chain
        let fallback_chain = params.model.to_fallback_chain()?;

        // Create base request (model will be filled in by fallback chain for each provider)
        let base_request = PromptRequest {
            model: String::new(), // Placeholder, will be replaced
            prompt: params.prompt,
            system_prompt: params.system_prompt,
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_p: params.top_p,
            stop_sequences: params.stop_sequences,
        };

        let response = fallback_chain.prompt(&base_request).await?;

        let outputs = json!({
            "content": response.content,
            "model": response.model,
            "provider": response.provider,
            "finish_reason": response.finish_reason,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "output_tokens": response.usage.output_tokens,
                "total_tokens": response.usage.total_tokens,
                "cached_tokens": response.usage.cached_tokens,
            }
        });

        Ok(ActivityResult::value("result", outputs))
    }

    fn name(&self) -> &str {
        "llm_prompt"
    }

    fn worker(&self) -> &str {
        "builtin"
    }
}

// ============================================================================
// Embedding Activity
// ============================================================================

/// Embedding Activity parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingParams {
    /// Model specification in "provider/model" format
    /// Can be a single string or array for fallback
    /// Examples:
    ///   - "openai/text-embedding-3-small"
    ///   - ["openai/text-embedding-3-small", "google/text-embedding-004"]
    pub model: ModelSpec,

    /// Input texts to embed
    pub input: Vec<String>,
}

/// Embedding Activity implementation
pub struct EmbeddingActivity;

impl EmbeddingActivity {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmbeddingActivity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ActivityImpl for EmbeddingActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        let params: EmbeddingParams =
            serde_json::from_value(parameters).context("Failed to parse embedding parameters")?;

        // Convert model spec to fallback chain
        let fallback_chain = params.model.to_fallback_chain()?;

        // Create base request (model will be filled in by fallback chain for each provider)
        let base_request = EmbeddingRequest {
            model: String::new(), // Placeholder, will be replaced
            input: params.input,
        };

        let response = fallback_chain.embed(&base_request).await?;

        let outputs = json!({
            "embeddings": response.embeddings,
            "model": response.model,
            "provider": response.provider,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "output_tokens": response.usage.output_tokens,
                "total_tokens": response.usage.total_tokens,
                "cached_tokens": response.usage.cached_tokens,
            }
        });

        Ok(ActivityResult::value("result", outputs))
    }

    fn name(&self) -> &str {
        "embedding"
    }

    fn worker(&self) -> &str {
        "builtin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_spec_single() {
        let spec = ModelSpec::Single("anthropic/claude-3-5-sonnet-20241022".to_string());
        let chain = spec.to_fallback_chain().unwrap();
        assert_eq!(chain.provider_models.len(), 1);
        assert_eq!(chain.provider_models[0].0, "anthropic");
        assert_eq!(chain.provider_models[0].1, "claude-3-5-sonnet-20241022");
    }

    #[test]
    fn test_model_spec_fallback() {
        let spec = ModelSpec::Fallback(vec![
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            "openai/gpt-4".to_string(),
            "google/gemini-1.5-pro".to_string(),
        ]);
        let chain = spec.to_fallback_chain().unwrap();
        assert_eq!(chain.provider_models.len(), 3);
        assert_eq!(chain.provider_models[0], ("anthropic".to_string(), "claude-3-5-sonnet-20241022".to_string()));
        assert_eq!(chain.provider_models[1], ("openai".to_string(), "gpt-4".to_string()));
        assert_eq!(chain.provider_models[2], ("google".to_string(), "gemini-1.5-pro".to_string()));
    }

    #[test]
    fn test_model_spec_invalid_format() {
        let spec = ModelSpec::Single("invalid-no-slash".to_string());
        assert!(spec.to_fallback_chain().is_err());

        let spec2 = ModelSpec::Single("too/many/slashes".to_string());
        assert!(spec2.to_fallback_chain().is_err());
    }

    #[test]
    fn test_llm_prompt_activity_name() {
        let activity = LLMPromptActivity::new();
        assert_eq!(activity.name(), "llm_prompt");
        assert_eq!(activity.worker(), "builtin");
    }

    #[test]
    fn test_embedding_activity_name() {
        let activity = EmbeddingActivity::new();
        assert_eq!(activity.name(), "embedding");
        assert_eq!(activity.worker(), "builtin");
    }
}
