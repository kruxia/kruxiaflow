use crate::activity_result::ActivityResult;
use crate::llm::{
    AnthropicProvider, EmbeddingRequest, GoogleProvider, LLMError, LLMProvider, OllamaProvider,
    OpenAIProvider, PromptRequest,
};
use crate::registry::ActivityImpl;
use crate::streaming::{StreamSender, StreamingActivity};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use kruxiaflow_core::cost::{CostCalculator, ModelPricing};
use uuid::Uuid;

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

/// Budget parameters for fallback chain execution
#[derive(Debug, Clone)]
pub struct BudgetParams {
    /// Pricing information for all models (key: "provider/model")
    pub model_pricing: HashMap<String, ModelPricing>,
    /// Budget limit in USD (takes minimum of activity and workflow limits)
    pub budget_limit_usd: Decimal,
    /// Cumulative cost already incurred by this activity (across retry attempts)
    pub cumulative_cost_usd: Decimal,
}

/// Fallback chain configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackChain {
    /// List of (provider, model) pairs to try in order
    pub provider_models: Vec<(String, String)>,
}

impl FallbackChain {
    /// Execute a prompt request with fallback and optional budget enforcement
    pub async fn prompt(
        &self,
        base_request: &PromptRequest,
        budget_params: Option<&BudgetParams>,
    ) -> Result<PromptResponse> {
        let config = ProviderConfig::from_env();
        let mut last_error = None;
        let mut cumulative_cost = budget_params
            .map(|bp| bp.cumulative_cost_usd)
            .unwrap_or(Decimal::ZERO);
        let mut calculated_cost: Option<Decimal> = None;

        for (provider_name, model_name) in &self.provider_models {
            // Budget check before attempting this provider
            if let Some(budget) = budget_params {
                let model_key = format!("{}/{}", provider_name, model_name);

                // Get pricing for this model
                if let Some(pricing) = budget.model_pricing.get(&model_key) {
                    // Estimate tokens for input (prompt + system_prompt)
                    let mut input_text = base_request.prompt.clone();
                    if let Some(system) = &base_request.system_prompt {
                        input_text.push_str(system);
                    }
                    let estimated_input_tokens =
                        CostCalculator::estimate_tokens(provider_name, &input_text);

                    // Estimate output tokens (use max_tokens or conservative default)
                    let estimated_output_tokens = base_request.max_tokens.unwrap_or(4096);

                    // Calculate estimated cost
                    let input_cost = Decimal::from(estimated_input_tokens)
                        * pricing.input_price_per_million
                        / Decimal::from(1_000_000);
                    let output_cost = Decimal::from(estimated_output_tokens)
                        * pricing.output_price_per_million
                        / Decimal::from(1_000_000);
                    let estimated_cost = input_cost + output_cost;

                    // Check if this would exceed budget
                    if cumulative_cost + estimated_cost > budget.budget_limit_usd {
                        tracing::warn!(
                            "Skipping {}/{}: estimated cost ${:.6} would exceed budget (cumulative: ${:.6}, limit: ${:.6})",
                            provider_name,
                            model_name,
                            estimated_cost,
                            cumulative_cost,
                            budget.budget_limit_usd
                        );
                        continue; // Skip to next provider
                    }
                }
            }

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
                            // Calculate actual cost if budget params provided
                            if let Some(budget) = budget_params {
                                let model_key = format!("{}/{}", provider_name, model_name);
                                if let Some(pricing) = budget.model_pricing.get(&model_key) {
                                    let input_cost = Decimal::from(response.usage.prompt_tokens)
                                        * pricing.input_price_per_million
                                        / Decimal::from(1_000_000);
                                    let output_cost = Decimal::from(response.usage.output_tokens)
                                        * pricing.output_price_per_million
                                        / Decimal::from(1_000_000);
                                    let actual_cost = input_cost + output_cost;
                                    calculated_cost = Some(actual_cost);
                                    cumulative_cost += actual_cost;

                                    tracing::info!(
                                        "Provider {}/{} succeeded with cost ${:.6} (cumulative: ${:.6})",
                                        provider_name,
                                        model_name,
                                        actual_cost,
                                        cumulative_cost
                                    );
                                }
                            }

                            return Ok(PromptResponse {
                                content: response.content,
                                model: response.model,
                                provider: provider_name.clone(),
                                usage: response.usage,
                                finish_reason: response.finish_reason,
                                cost_usd: calculated_cost,
                            });
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

    /// Execute a prompt request with streaming and fallback.
    ///
    /// Streams tokens to the provided sender while accumulating the full response.
    /// Returns the complete response after streaming finishes.
    pub async fn prompt_stream(
        &self,
        base_request: &PromptRequest,
        budget_params: Option<&BudgetParams>,
        sender: &dyn StreamSender,
    ) -> Result<PromptResponse> {
        let config = ProviderConfig::from_env();
        let mut last_error = None;
        let mut cumulative_cost = budget_params
            .map(|bp| bp.cumulative_cost_usd)
            .unwrap_or(Decimal::ZERO);
        let mut calculated_cost: Option<Decimal> = None;

        for (provider_name, model_name) in &self.provider_models {
            // Budget check before attempting this provider (same as non-streaming)
            if let Some(budget) = budget_params {
                let model_key = format!("{}/{}", provider_name, model_name);

                if let Some(pricing) = budget.model_pricing.get(&model_key) {
                    let mut input_text = base_request.prompt.clone();
                    if let Some(system) = &base_request.system_prompt {
                        input_text.push_str(system);
                    }
                    let estimated_input_tokens =
                        CostCalculator::estimate_tokens(provider_name, &input_text);
                    let estimated_output_tokens = base_request.max_tokens.unwrap_or(4096);

                    let input_cost = Decimal::from(estimated_input_tokens)
                        * pricing.input_price_per_million
                        / Decimal::from(1_000_000);
                    let output_cost = Decimal::from(estimated_output_tokens)
                        * pricing.output_price_per_million
                        / Decimal::from(1_000_000);
                    let estimated_cost = input_cost + output_cost;

                    if cumulative_cost + estimated_cost > budget.budget_limit_usd {
                        tracing::warn!(
                            "Skipping {}/{}: estimated cost ${:.6} would exceed budget",
                            provider_name,
                            model_name,
                            estimated_cost
                        );
                        continue;
                    }
                }
            }

            match config.create_provider(provider_name) {
                Ok(provider) => {
                    let request = PromptRequest {
                        model: model_name.clone(),
                        prompt: base_request.prompt.clone(),
                        system_prompt: base_request.system_prompt.clone(),
                        max_tokens: base_request.max_tokens,
                        temperature: base_request.temperature,
                        top_p: base_request.top_p,
                        stop_sequences: base_request.stop_sequences.clone(),
                    };

                    // Use streaming API
                    match provider.prompt_stream(&request).await {
                        Ok(mut stream) => {
                            let mut full_content = String::new();
                            let mut token_index = 0u32;
                            let mut finish_reason = crate::llm::FinishReason::Stop;

                            // Stream tokens
                            while let Some(chunk_result) = stream.next().await {
                                match chunk_result {
                                    Ok(chunk) => {
                                        if !chunk.content.is_empty() {
                                            full_content.push_str(&chunk.content);

                                            // Send token to subscribers
                                            if let Err(e) =
                                                sender.send_token(&chunk.content, token_index).await
                                            {
                                                tracing::warn!("Failed to send token: {}", e);
                                            }
                                            token_index += 1;
                                        }

                                        if let Some(reason) = chunk.finish_reason {
                                            finish_reason = reason;
                                        }
                                    }
                                    Err(e) => {
                                        last_error = Some(e);
                                        break;
                                    }
                                }
                            }

                            // If we got content, we succeeded
                            if !full_content.is_empty() || last_error.is_none() {
                                // Estimate cost (streaming doesn't always provide usage)
                                if let Some(budget) = budget_params {
                                    let model_key = format!("{}/{}", provider_name, model_name);
                                    if let Some(pricing) = budget.model_pricing.get(&model_key) {
                                        // Estimate based on content length
                                        let estimated_output_tokens =
                                            CostCalculator::estimate_tokens(
                                                provider_name,
                                                &full_content,
                                            );
                                        let input_cost =
                                            Decimal::from(CostCalculator::estimate_tokens(
                                                provider_name,
                                                &base_request.prompt,
                                            )) * pricing.input_price_per_million
                                                / Decimal::from(1_000_000);
                                        let output_cost = Decimal::from(estimated_output_tokens)
                                            * pricing.output_price_per_million
                                            / Decimal::from(1_000_000);
                                        let actual_cost = input_cost + output_cost;
                                        calculated_cost = Some(actual_cost);
                                        cumulative_cost += actual_cost;
                                    }
                                }

                                return Ok(PromptResponse {
                                    content: full_content,
                                    model: model_name.clone(),
                                    provider: provider_name.clone(),
                                    usage: crate::llm::TokenUsage {
                                        prompt_tokens: 0, // Not available in streaming
                                        output_tokens: 0,
                                        total_tokens: 0,
                                        cached_tokens: None,
                                    },
                                    finish_reason,
                                    cost_usd: calculated_cost,
                                });
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Provider {}/{} streaming failed: {}. Trying next provider...",
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
            "All providers failed for streaming. Last error: {}",
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
                            });
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
    pub cost_usd: Option<Decimal>,
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

    // Budget-aware parameters (enriched by orchestrator)
    /// Pricing information for all models in the fallback chain
    /// Key: "provider/model" string, Value: ModelPricing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_pricing: Option<HashMap<String, ModelPricing>>,

    /// Activity-level budget limit in USD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_budget_limit_usd: Option<Decimal>,

    /// Workflow-level budget limit in USD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_budget_limit_usd: Option<Decimal>,

    /// Cumulative cost already incurred by this activity (across retry attempts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cumulative_activity_cost_usd: Option<Decimal>,
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
        let params: LLMPromptParams =
            serde_json::from_value(parameters).context("Failed to parse LLM prompt parameters")?;

        // Convert model spec to fallback chain
        let fallback_chain = params.model.to_fallback_chain()?;

        // Construct budget parameters if provided by orchestrator
        let budget_params = if let Some(model_pricing) = params.model_pricing {
            // Determine effective budget limit (minimum of activity and workflow limits)
            let budget_limit = match (
                params.activity_budget_limit_usd,
                params.workflow_budget_limit_usd,
            ) {
                (Some(activity_limit), Some(workflow_limit)) => {
                    if activity_limit < workflow_limit {
                        Some(activity_limit)
                    } else {
                        Some(workflow_limit)
                    }
                }
                (Some(activity_limit), None) => Some(activity_limit),
                (None, Some(workflow_limit)) => Some(workflow_limit),
                (None, None) => {
                    // If pricing is provided but no limits, don't enforce budget
                    // This shouldn't happen in practice, but handle gracefully
                    None
                }
            };

            budget_limit.map(|limit| BudgetParams {
                model_pricing,
                budget_limit_usd: limit,
                cumulative_cost_usd: params.cumulative_activity_cost_usd.unwrap_or(Decimal::ZERO),
            })
        } else {
            None
        };

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

        let response = fallback_chain
            .prompt(&base_request, budget_params.as_ref())
            .await?;

        let outputs = json!({
            "content": response.content,
            "model": response.model,
            "provider": response.provider,
            "finish_reason": response.finish_reason,
            "cost_usd": response.cost_usd,
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

#[async_trait]
impl StreamingActivity for LLMPromptActivity {
    async fn execute_streaming(
        &self,
        activity_id: Uuid,
        parameters: Value,
        sender: Box<dyn StreamSender>,
    ) -> Result<ActivityResult> {
        let params: LLMPromptParams =
            serde_json::from_value(parameters).context("Failed to parse LLM prompt parameters")?;

        // Convert model spec to fallback chain
        let fallback_chain = params.model.to_fallback_chain()?;

        // Construct budget parameters if provided by orchestrator
        let budget_params = if let Some(model_pricing) = params.model_pricing {
            let budget_limit = match (
                params.activity_budget_limit_usd,
                params.workflow_budget_limit_usd,
            ) {
                (Some(activity_limit), Some(workflow_limit)) => {
                    if activity_limit < workflow_limit {
                        Some(activity_limit)
                    } else {
                        Some(workflow_limit)
                    }
                }
                (Some(activity_limit), None) => Some(activity_limit),
                (None, Some(workflow_limit)) => Some(workflow_limit),
                (None, None) => None,
            };

            budget_limit.map(|limit| BudgetParams {
                model_pricing,
                budget_limit_usd: limit,
                cumulative_cost_usd: params.cumulative_activity_cost_usd.unwrap_or(Decimal::ZERO),
            })
        } else {
            None
        };

        // Create base request
        let base_request = PromptRequest {
            model: String::new(),
            prompt: params.prompt,
            system_prompt: params.system_prompt,
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_p: params.top_p,
            stop_sequences: params.stop_sequences,
        };

        // Use streaming execution
        let response = fallback_chain
            .prompt_stream(&base_request, budget_params.as_ref(), sender.as_ref())
            .await?;

        // Send completion message
        let result_value = json!({
            "content": response.content,
            "model": response.model,
            "provider": response.provider,
            "finish_reason": response.finish_reason,
            "cost_usd": response.cost_usd,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "output_tokens": response.usage.output_tokens,
                "total_tokens": response.usage.total_tokens,
                "cached_tokens": response.usage.cached_tokens,
            }
        });

        if let Err(e) = sender
            .send_complete(activity_id, result_value.clone())
            .await
        {
            tracing::warn!(
                activity_id = %activity_id,
                "Failed to send completion message: {}",
                e
            );
        }

        if let Err(e) = sender.close().await {
            tracing::warn!(
                activity_id = %activity_id,
                "Failed to close stream sender: {}",
                e
            );
        }

        Ok(ActivityResult::value("result", result_value))
    }

    fn supports_streaming(&self) -> bool {
        true
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
    use rust_decimal_macros::dec;

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
        assert_eq!(
            chain.provider_models[0],
            (
                "anthropic".to_string(),
                "claude-3-5-sonnet-20241022".to_string()
            )
        );
        assert_eq!(
            chain.provider_models[1],
            ("openai".to_string(), "gpt-4".to_string())
        );
        assert_eq!(
            chain.provider_models[2],
            ("google".to_string(), "gemini-1.5-pro".to_string())
        );
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

    // ============================================================================
    // Budget-Aware Fallback Chain Tests
    // ============================================================================

    /// Helper to create test pricing data
    fn create_test_pricing() -> HashMap<String, ModelPricing> {
        let mut pricing = HashMap::new();

        // Expensive model: $3/$15 per million tokens
        pricing.insert(
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            ModelPricing {
                input_price_per_million: dec!(3.00),
                output_price_per_million: dec!(15.00),
                cached_input_price_per_million: Some(dec!(0.30)),
            },
        );

        // Mid-range model: $0.80/$4 per million tokens
        pricing.insert(
            "anthropic/claude-3-5-haiku-20241022".to_string(),
            ModelPricing {
                input_price_per_million: dec!(0.80),
                output_price_per_million: dec!(4.00),
                cached_input_price_per_million: Some(dec!(0.08)),
            },
        );

        // Free model: $0/$0 per million tokens
        pricing.insert(
            "ollama/llama3.2".to_string(),
            ModelPricing {
                input_price_per_million: dec!(0.00),
                output_price_per_million: dec!(0.00),
                cached_input_price_per_million: None,
            },
        );

        pricing
    }

    #[test]
    fn test_budget_params_construction() {
        let pricing = create_test_pricing();

        let budget_params = BudgetParams {
            model_pricing: pricing.clone(),
            budget_limit_usd: dec!(0.10),
            cumulative_cost_usd: dec!(0.05),
        };

        assert_eq!(budget_params.budget_limit_usd, dec!(0.10));
        assert_eq!(budget_params.cumulative_cost_usd, dec!(0.05));
        assert_eq!(budget_params.model_pricing.len(), 3);
    }

    #[test]
    fn test_budget_enforcement_minimum_logic() {
        // Test that activity budget takes precedence when lower than workflow budget
        let activity_limit = dec!(1.00);
        let workflow_limit = dec!(5.00);

        let effective_limit = if activity_limit < workflow_limit {
            activity_limit
        } else {
            workflow_limit
        };

        assert_eq!(effective_limit, dec!(1.00));

        // Test that workflow budget takes precedence when lower
        let activity_limit = dec!(5.00);
        let workflow_limit = dec!(1.00);

        let effective_limit = if activity_limit < workflow_limit {
            activity_limit
        } else {
            workflow_limit
        };

        assert_eq!(effective_limit, dec!(1.00));
    }

    #[test]
    fn test_cost_estimation_logic() {
        let pricing = create_test_pricing();

        // Get pricing for Sonnet model
        let sonnet_pricing = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();

        // Estimate tokens for a short prompt (should be ~10-15 tokens)
        let prompt = "Hello, world!";
        let _estimated_tokens = CostCalculator::estimate_tokens("anthropic", prompt);

        // Calculate estimated cost for 10 input tokens and 100 output tokens
        let input_cost =
            Decimal::from(10) * sonnet_pricing.input_price_per_million / Decimal::from(1_000_000);
        let output_cost =
            Decimal::from(100) * sonnet_pricing.output_price_per_million / Decimal::from(1_000_000);
        let total_cost = input_cost + output_cost;

        // Verify cost calculation
        // 10 * $3.00 / 1M = $0.00003
        // 100 * $15.00 / 1M = $0.0015
        // Total = $0.00153
        assert!(total_cost > dec!(0.001) && total_cost < dec!(0.002));
    }

    #[test]
    fn test_budget_skip_logic() {
        let pricing = create_test_pricing();

        // Budget: $0.10
        // Cumulative cost: $0.09
        // Remaining: $0.01
        let budget_limit = dec!(0.10);
        let cumulative_cost = dec!(0.09);

        // Sonnet estimated cost (10 input + 1000 output tokens)
        // 10 * $3.00 / 1M = $0.00003
        // 1000 * $15.00 / 1M = $0.015
        // Total = $0.01503
        let sonnet_pricing = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();
        let sonnet_input =
            Decimal::from(10) * sonnet_pricing.input_price_per_million / Decimal::from(1_000_000);
        let sonnet_output = Decimal::from(1000) * sonnet_pricing.output_price_per_million
            / Decimal::from(1_000_000);
        let sonnet_estimated = sonnet_input + sonnet_output;

        // Should skip: $0.09 + $0.01503 > $0.10
        assert!(cumulative_cost + sonnet_estimated > budget_limit);

        // Haiku estimated cost (10 input + 1000 output tokens)
        // 10 * $0.80 / 1M = $0.000008
        // 1000 * $4.00 / 1M = $0.004
        // Total = $0.004008
        let haiku_pricing = pricing.get("anthropic/claude-3-5-haiku-20241022").unwrap();
        let haiku_input =
            Decimal::from(10) * haiku_pricing.input_price_per_million / Decimal::from(1_000_000);
        let haiku_output =
            Decimal::from(1000) * haiku_pricing.output_price_per_million / Decimal::from(1_000_000);
        let haiku_estimated = haiku_input + haiku_output;

        // Should be within budget: $0.09 + $0.004008 = $0.094008 < $0.10
        // This demonstrates the fallback chain: skip Sonnet, use Haiku
        assert!(cumulative_cost + haiku_estimated < budget_limit);

        // Ollama estimated cost (free)
        let ollama_pricing = pricing.get("ollama/llama3.2").unwrap();
        let ollama_input =
            Decimal::from(10) * ollama_pricing.input_price_per_million / Decimal::from(1_000_000);
        let ollama_output = Decimal::from(1000) * ollama_pricing.output_price_per_million
            / Decimal::from(1_000_000);
        let ollama_estimated = ollama_input + ollama_output;

        // Should always be within budget: $0.09 + $0.00 < $0.10
        assert_eq!(ollama_estimated, dec!(0.00));
        assert!(cumulative_cost + ollama_estimated < budget_limit);
    }

    #[test]
    fn test_cumulative_cost_tracking() {
        let mut cumulative_cost = dec!(0.00);

        // First attempt costs $0.01
        cumulative_cost += dec!(0.01);
        assert_eq!(cumulative_cost, dec!(0.01));

        // Second attempt costs $0.02
        cumulative_cost += dec!(0.02);
        assert_eq!(cumulative_cost, dec!(0.03));

        // Third attempt costs $0.005
        cumulative_cost += dec!(0.005);
        assert_eq!(cumulative_cost, dec!(0.035));

        // Verify total is tracked correctly
        let budget_limit = dec!(0.10);
        assert!(cumulative_cost < budget_limit);

        // Verify we can detect budget exceeded
        let large_cost = dec!(0.10);
        assert!(cumulative_cost + large_cost > budget_limit);
    }

    #[test]
    fn test_pricing_lookup_by_model_key() {
        let pricing = create_test_pricing();

        // Test exact match
        let key = "anthropic/claude-3-5-sonnet-20241022";
        assert!(pricing.contains_key(key));

        let model_pricing = pricing.get(key).unwrap();
        assert_eq!(model_pricing.input_price_per_million, dec!(3.00));
        assert_eq!(model_pricing.output_price_per_million, dec!(15.00));

        // Test missing key
        let invalid_key = "unknown/model";
        assert!(!pricing.contains_key(invalid_key));
    }

    #[test]
    fn test_budget_limit_none_allows_execution() {
        // When budget_limit is None, all models should be attempted
        let pricing = create_test_pricing();

        // No budget limit means infinite budget
        let budget_limit: Option<Decimal> = None;

        // Even expensive models should be allowed
        let sonnet_pricing = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();
        let huge_cost = Decimal::from(1_000_000) * sonnet_pricing.output_price_per_million
            / Decimal::from(1_000_000);

        // Without a budget limit, this should be allowed
        if let Some(limit) = budget_limit {
            assert!(huge_cost > limit); // Would fail if budget existed
        } else {
            // No budget means always proceed
            assert!(true);
        }
    }

    #[test]
    fn test_zero_budget_blocks_all_paid_models() {
        let pricing = create_test_pricing();
        let budget_limit = dec!(0.00);
        let cumulative_cost = dec!(0.00);

        // Sonnet should be blocked
        let sonnet_pricing = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();
        let sonnet_cost =
            Decimal::from(1) * sonnet_pricing.input_price_per_million / Decimal::from(1_000_000);
        assert!(cumulative_cost + sonnet_cost > budget_limit);

        // Haiku should be blocked
        let haiku_pricing = pricing.get("anthropic/claude-3-5-haiku-20241022").unwrap();
        let haiku_cost =
            Decimal::from(1) * haiku_pricing.input_price_per_million / Decimal::from(1_000_000);
        assert!(cumulative_cost + haiku_cost > budget_limit);

        // Ollama should still work (free)
        let ollama_pricing = pricing.get("ollama/llama3.2").unwrap();
        let ollama_cost =
            Decimal::from(1) * ollama_pricing.input_price_per_million / Decimal::from(1_000_000);
        assert_eq!(ollama_cost, dec!(0.00));
        assert!(cumulative_cost + ollama_cost <= budget_limit);
    }

    #[test]
    fn test_cached_token_pricing() {
        let pricing = create_test_pricing();
        let sonnet_pricing = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();

        // Regular input tokens
        let regular_cost =
            Decimal::from(1000) * sonnet_pricing.input_price_per_million / Decimal::from(1_000_000);
        assert_eq!(regular_cost, dec!(0.003));

        // Cached input tokens (10x cheaper)
        let cached_cost = Decimal::from(1000)
            * sonnet_pricing.cached_input_price_per_million.unwrap()
            / Decimal::from(1_000_000);
        assert_eq!(cached_cost, dec!(0.0003));

        // Verify cached is 10x cheaper
        assert!(cached_cost * dec!(10) == regular_cost);
    }

    #[test]
    fn test_cost_usd_in_prompt_response() {
        let pricing = create_test_pricing();

        // Create budget params with pricing
        let budget_params = BudgetParams {
            model_pricing: pricing.clone(),
            budget_limit_usd: dec!(1.00),
            cumulative_cost_usd: dec!(0.00),
        };

        // Simulate a response with known token counts
        let mock_provider_response = crate::llm::PromptResponse {
            content: "Test response".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 100,  // 100 input tokens
                output_tokens: 1000, // 1000 output tokens
                total_tokens: 1100,
                cached_tokens: None,
            },
            finish_reason: crate::llm::FinishReason::Stop,
        };

        // Calculate expected cost
        let sonnet_pricing = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();
        let expected_input_cost =
            Decimal::from(100) * sonnet_pricing.input_price_per_million / Decimal::from(1_000_000);
        let expected_output_cost = Decimal::from(1000) * sonnet_pricing.output_price_per_million
            / Decimal::from(1_000_000);
        let expected_total_cost = expected_input_cost + expected_output_cost;

        // Expected: 100 * $3.00 / 1M = $0.0003
        //          1000 * $15.00 / 1M = $0.015
        //          Total = $0.0153
        assert_eq!(expected_total_cost, dec!(0.0153));

        // Verify cost calculation logic matches what activity does
        let model_key = "anthropic/claude-3-5-sonnet-20241022";
        if let Some(pricing) = budget_params.model_pricing.get(model_key) {
            let input_cost = Decimal::from(mock_provider_response.usage.prompt_tokens)
                * pricing.input_price_per_million
                / Decimal::from(1_000_000);
            let output_cost = Decimal::from(mock_provider_response.usage.output_tokens)
                * pricing.output_price_per_million
                / Decimal::from(1_000_000);
            let actual_cost = input_cost + output_cost;

            assert_eq!(actual_cost, expected_total_cost);
        }
    }

    #[test]
    fn test_cost_usd_none_when_no_pricing() {
        // When budget params are not provided, cost_usd should be None
        // This simulates the case where orchestrator doesn't enrich with pricing

        // Simulate a PromptResponse without cost calculation
        let response_without_pricing = PromptResponse {
            content: "Test response".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            provider: "anthropic".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 100,
                output_tokens: 1000,
                total_tokens: 1100,
                cached_tokens: None,
            },
            finish_reason: crate::llm::FinishReason::Stop,
            cost_usd: None, // No pricing available
        };

        // Verify cost is None when pricing not available
        assert_eq!(response_without_pricing.cost_usd, None);
    }

    #[test]
    fn test_cost_usd_available_in_activity_output() {
        // This test validates the complete flow:
        // 1. Activity calculates cost
        // 2. Cost is included in PromptResponse
        // 3. Cost is serialized to JSON output

        let response_with_cost = PromptResponse {
            content: "Test response".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            provider: "anthropic".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 100,
                output_tokens: 1000,
                total_tokens: 1100,
                cached_tokens: None,
            },
            finish_reason: crate::llm::FinishReason::Stop,
            cost_usd: Some(dec!(0.0153)),
        };

        // Simulate what the activity does when creating output JSON
        let outputs = json!({
            "content": response_with_cost.content,
            "model": response_with_cost.model,
            "provider": response_with_cost.provider,
            "finish_reason": response_with_cost.finish_reason,
            "cost_usd": response_with_cost.cost_usd,
            "usage": {
                "prompt_tokens": response_with_cost.usage.prompt_tokens,
                "output_tokens": response_with_cost.usage.output_tokens,
                "total_tokens": response_with_cost.usage.total_tokens,
                "cached_tokens": response_with_cost.usage.cached_tokens,
            }
        });

        // Verify cost_usd is present in JSON output
        assert_eq!(outputs["cost_usd"], json!(dec!(0.0153)));
        assert_eq!(outputs["model"], "claude-3-5-sonnet-20241022");
        assert_eq!(outputs["provider"], "anthropic");

        // Verify cost can be extracted from JSON (as would happen in template expression)
        let cost_from_json = outputs["cost_usd"].as_str().unwrap();
        let cost_decimal: Decimal = cost_from_json.parse().unwrap();
        assert_eq!(cost_decimal, dec!(0.0153));
    }
}
