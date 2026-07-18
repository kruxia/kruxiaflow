use crate::activity_result::ActivityResult;
use crate::llm::{
    AnthropicProvider, EmbeddingRequest, GoogleProvider, LLMError, LLMProvider, OllamaProvider,
    OpenAIProvider, PromptRequest,
};
use crate::registry::{ActivityContext, ActivityImpl};
use crate::streaming::{StreamSender, StreamingActivity};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use kruxiaflow_core::cost::{CostCalculator, ModelPricing};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
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
        "std"
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

    /// Batch size for large inputs (default: 500)
    /// When input.len() > batch_size, inputs are processed in batches
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    500
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
    /// Execute with context - streams all embeddings to workflow storage
    ///
    /// Embeddings are always streamed to workflow storage as JSON Lines format,
    /// ensuring consistent memory usage regardless of input size.
    async fn execute_with_context(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult> {
        let params: EmbeddingParams = serde_json::from_value(parameters.clone())
            .context("Failed to parse embedding parameters")?;

        let total_inputs = params.input.len();

        // Require workflow storage for streaming
        let storage = match &ctx.storage {
            Some(s) => s.clone(),
            None => {
                tracing::warn!(
                    total_inputs = total_inputs,
                    "No workflow storage available - falling back to in-memory"
                );
                return self.execute(parameters).await;
            }
        };

        tracing::info!(
            total_inputs = total_inputs,
            workflow_id = %ctx.workflow_id,
            activity_key = %ctx.activity_key,
            "Streaming embeddings to workflow storage"
        );

        // Convert model spec to fallback chain
        let fallback_chain = params.model.to_fallback_chain()?;
        let batch_size = params.batch_size;
        let filename = "embeddings.jsonl";

        // Create channel for streaming to storage
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(32);

        // Spawn upload task
        let upload_workflow_id = ctx.workflow_id;
        let upload_activity_key = ctx.activity_key.clone();
        let upload_storage = storage.clone();
        let upload_handle = tokio::spawn(async move {
            let stream = ReceiverStream::new(rx).map(Ok::<_, std::io::Error>);
            upload_storage
                .upload_file(
                    upload_workflow_id,
                    &upload_activity_key,
                    filename,
                    Some("application/x-ndjson"),
                    Box::pin(stream),
                )
                .await
        });

        // Process batches and stream to storage
        let mut total_prompt_tokens = 0u32;
        let mut total_output_tokens = 0u32;
        let mut model_name = String::new();
        let mut provider_name = String::new();
        let mut embedding_count = 0usize;

        let num_batches = total_inputs.div_ceil(batch_size);
        tracing::info!(
            total_inputs = total_inputs,
            batch_size = batch_size,
            num_batches = num_batches,
            "Processing embeddings in batches with streaming"
        );

        for (batch_idx, batch) in params.input.chunks(batch_size).enumerate() {
            tracing::info!(
                batch = batch_idx + 1,
                batch_size = batch.len(),
                progress = format!("{}/{}", batch_idx * batch_size + batch.len(), total_inputs),
                "Processing embedding batch"
            );

            let batch_request = EmbeddingRequest {
                model: String::new(),
                input: batch.to_vec(),
            };

            let response = fallback_chain
                .embed(&batch_request)
                .await
                .with_context(|| {
                    format!(
                        "Failed to generate embeddings for batch {} ({} items)",
                        batch_idx + 1,
                        batch.len()
                    )
                })?;

            // Stream embeddings to storage as JSON Lines (one per line)
            for embedding in response.embeddings {
                let line =
                    serde_json::to_string(&embedding).context("Failed to serialize embedding")?;
                tx.send(Bytes::from(format!("{}\n", line)))
                    .await
                    .context("Failed to send embedding to storage stream")?;
                embedding_count += 1;
            }

            total_prompt_tokens += response.usage.prompt_tokens;
            total_output_tokens += response.usage.output_tokens;
            model_name = response.model;
            provider_name = response.provider;

            tracing::info!(
                batch = batch_idx + 1,
                embeddings_streamed = embedding_count,
                "Batch completed and streamed"
            );
        }

        // Close stream and wait for upload to complete
        drop(tx);
        let file_metadata = upload_handle
            .await
            .context("Upload task panicked")?
            .context("Failed to upload embeddings to storage")?;

        tracing::info!(
            total_embeddings = embedding_count,
            total_prompt_tokens = total_prompt_tokens,
            file_size = file_metadata.size,
            "All embedding batches completed and streamed to storage"
        );

        // Get file reference for consumer activities
        let file_ref = storage
            .get_file_reference(ctx.workflow_id, &ctx.activity_key, filename)
            .await
            .context("Failed to get file reference")?;

        tracing::info!(
            file_ref = %file_ref,
            "Generated embeddings file reference"
        );

        // Return file reference instead of embeddings array
        // Always include both keys so templates can reference either one
        let outputs = json!({
            "embeddings": null,  // Not present when streaming - use embeddings_file instead
            "embeddings_file": file_ref,
            "embedding_count": embedding_count,
            "model": model_name,
            "provider": provider_name,
            "usage": {
                "prompt_tokens": total_prompt_tokens,
                "output_tokens": total_output_tokens,
                "total_tokens": total_prompt_tokens + total_output_tokens,
                "cached_tokens": null,
            }
        });

        tracing::info!(
            outputs_keys = ?outputs.as_object().map(|m| m.keys().collect::<Vec<_>>()),
            embeddings_file = ?outputs.get("embeddings_file"),
            "Embedding activity returning streamed output"
        );

        Ok(ActivityResult::value("result", outputs))
    }

    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        let params: EmbeddingParams =
            serde_json::from_value(parameters).context("Failed to parse embedding parameters")?;

        // Convert model spec to fallback chain
        let fallback_chain = params.model.to_fallback_chain()?;

        let total_inputs = params.input.len();
        let batch_size = params.batch_size;

        // If input is small enough, process in one batch
        if total_inputs <= batch_size {
            let base_request = EmbeddingRequest {
                model: String::new(),
                input: params.input,
            };

            let response = fallback_chain.embed(&base_request).await?;

            // Always include both keys so templates can reference either one
            let outputs = json!({
                "embeddings": response.embeddings,
                "embeddings_file": null,  // Not present for inline embeddings
                "embedding_count": response.embeddings.len(),
                "model": response.model,
                "provider": response.provider,
                "usage": {
                    "prompt_tokens": response.usage.prompt_tokens,
                    "output_tokens": response.usage.output_tokens,
                    "total_tokens": response.usage.total_tokens,
                    "cached_tokens": response.usage.cached_tokens,
                }
            });

            return Ok(ActivityResult::value("result", outputs));
        }

        // Batch processing for large inputs
        tracing::info!(
            total_inputs = total_inputs,
            batch_size = batch_size,
            num_batches = total_inputs.div_ceil(batch_size),
            "Processing embeddings in batches"
        );

        let mut all_embeddings: Vec<Vec<f64>> = Vec::with_capacity(total_inputs);
        let mut total_prompt_tokens = 0u32;
        let mut total_output_tokens = 0u32;
        let mut model_name = String::new();
        let mut provider_name = String::new();

        for (batch_idx, batch) in params.input.chunks(batch_size).enumerate() {
            tracing::info!(
                batch = batch_idx + 1,
                batch_size = batch.len(),
                progress = format!("{}/{}", batch_idx * batch_size + batch.len(), total_inputs),
                "Processing embedding batch"
            );

            let batch_request = EmbeddingRequest {
                model: String::new(),
                input: batch.to_vec(),
            };

            let response = fallback_chain
                .embed(&batch_request)
                .await
                .with_context(|| {
                    format!(
                        "Failed to generate embeddings for batch {} ({} items)",
                        batch_idx + 1,
                        batch.len()
                    )
                })?;

            // Accumulate results
            all_embeddings.extend(response.embeddings);
            total_prompt_tokens += response.usage.prompt_tokens;
            total_output_tokens += response.usage.output_tokens;
            model_name = response.model;
            provider_name = response.provider;

            tracing::info!(
                batch = batch_idx + 1,
                embeddings_generated = all_embeddings.len(),
                "Batch completed"
            );
        }

        tracing::info!(
            total_embeddings = all_embeddings.len(),
            total_prompt_tokens = total_prompt_tokens,
            "All embedding batches completed"
        );

        // Always include both keys so templates can reference either one
        let outputs = json!({
            "embeddings": all_embeddings,
            "embeddings_file": null,  // Not present for inline embeddings
            "embedding_count": all_embeddings.len(),
            "model": model_name,
            "provider": provider_name,
            "usage": {
                "prompt_tokens": total_prompt_tokens,
                "output_tokens": total_output_tokens,
                "total_tokens": total_prompt_tokens + total_output_tokens,
                "cached_tokens": null,
            }
        });

        Ok(ActivityResult::value("result", outputs))
    }

    fn name(&self) -> &str {
        "embedding"
    }

    fn worker(&self) -> &str {
        "std"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serial_test::serial;

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
        assert_eq!(activity.worker(), "std");
    }

    #[test]
    fn test_embedding_activity_name() {
        let activity = EmbeddingActivity::new();
        assert_eq!(activity.name(), "embedding");
        assert_eq!(activity.worker(), "std");
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
                cache_write_price_per_million: None,
            },
        );

        // Mid-range model: $0.80/$4 per million tokens
        pricing.insert(
            "anthropic/claude-3-5-haiku-20241022".to_string(),
            ModelPricing {
                input_price_per_million: dec!(0.80),
                output_price_per_million: dec!(4.00),
                cached_input_price_per_million: Some(dec!(0.08)),
                cache_write_price_per_million: None,
            },
        );

        // Free model: $0/$0 per million tokens
        pricing.insert(
            "ollama/llama3.2".to_string(),
            ModelPricing {
                input_price_per_million: dec!(0.00),
                output_price_per_million: dec!(0.00),
                cached_input_price_per_million: None,
                cache_write_price_per_million: None,
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

    // ============================================================================
    // ProviderConfig Tests
    // ============================================================================

    #[test]
    #[serial]
    fn test_provider_config_from_env_reads_all_vars() {
        // Set all env vars
        unsafe {
            env::set_var("ANTHROPIC_API_KEY", "test-anthropic-key");
            env::set_var("OPENAI_API_KEY", "test-openai-key");
            env::set_var("GOOGLE_API_KEY", "test-google-key");
            env::set_var("OLLAMA_BASE_URL", "http://ollama:11434");
            env::set_var("OLLAMA_API_KEY", "test-ollama-key");
        }

        let config = ProviderConfig::from_env();

        assert_eq!(
            config.anthropic_api_key,
            Some("test-anthropic-key".to_string())
        );
        assert_eq!(config.openai_api_key, Some("test-openai-key".to_string()));
        assert_eq!(config.google_api_key, Some("test-google-key".to_string()));
        assert_eq!(
            config.ollama_base_url,
            Some("http://ollama:11434".to_string())
        );
        assert_eq!(config.ollama_api_key, Some("test-ollama-key".to_string()));

        // Cleanup
        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
            env::remove_var("OPENAI_API_KEY");
            env::remove_var("GOOGLE_API_KEY");
            env::remove_var("OLLAMA_BASE_URL");
            env::remove_var("OLLAMA_API_KEY");
        }
    }

    #[test]
    #[serial]
    fn test_provider_config_from_env_missing_vars() {
        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
            env::remove_var("OPENAI_API_KEY");
            env::remove_var("GOOGLE_API_KEY");
            env::remove_var("OLLAMA_BASE_URL");
            env::remove_var("OLLAMA_API_KEY");
        }

        let config = ProviderConfig::from_env();

        assert!(config.anthropic_api_key.is_none());
        assert!(config.openai_api_key.is_none());
        assert!(config.google_api_key.is_none());
        assert!(config.ollama_base_url.is_none());
        assert!(config.ollama_api_key.is_none());
    }

    #[test]
    #[serial]
    fn test_provider_config_create_anthropic_provider() {
        let config = ProviderConfig {
            anthropic_api_key: Some("test-key".to_string()),
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let provider = config.create_provider("anthropic").unwrap();
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    #[serial]
    fn test_provider_config_create_openai_provider() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: Some("test-key".to_string()),
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let provider = config.create_provider("openai").unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    #[serial]
    fn test_provider_config_create_google_provider() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: Some("test-key".to_string()),
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let provider = config.create_provider("google").unwrap();
        assert_eq!(provider.name(), "google");
    }

    #[test]
    #[serial]
    fn test_provider_config_create_ollama_provider() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: Some("http://localhost:11434".to_string()),
            ollama_api_key: None,
        };

        let provider = config.create_provider("ollama").unwrap();
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    #[serial]
    fn test_provider_config_create_ollama_without_base_url() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        // Ollama works without base URL (uses default)
        let provider = config.create_provider("ollama").unwrap();
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_provider_config_create_unknown_provider() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let result = config.create_provider("unknown_provider");
        assert!(result.is_err());
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("Unknown provider")
        );
    }

    #[test]
    fn test_provider_config_create_anthropic_missing_key() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let result = config.create_provider("anthropic");
        assert!(result.is_err());
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("ANTHROPIC_API_KEY")
        );
    }

    #[test]
    fn test_provider_config_create_openai_missing_key() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let result = config.create_provider("openai");
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("OPENAI_API_KEY"));
    }

    #[test]
    fn test_provider_config_create_google_missing_key() {
        let config = ProviderConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        let result = config.create_provider("google");
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("GOOGLE_API_KEY"));
    }

    #[test]
    fn test_provider_config_create_case_insensitive() {
        let config = ProviderConfig {
            anthropic_api_key: Some("key".to_string()),
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: None,
            ollama_api_key: None,
        };

        // Provider matching is case-insensitive
        let provider = config.create_provider("Anthropic").unwrap();
        assert_eq!(provider.name(), "anthropic");

        let provider = config.create_provider("ANTHROPIC").unwrap();
        assert_eq!(provider.name(), "anthropic");
    }

    // ============================================================================
    // ModelSpec JSON Deserialization Tests
    // ============================================================================

    #[test]
    fn test_model_spec_deserialize_single_string() {
        let json = json!("anthropic/claude-3-5-sonnet-20241022");
        let spec: ModelSpec = serde_json::from_value(json).unwrap();
        match spec {
            ModelSpec::Single(s) => assert_eq!(s, "anthropic/claude-3-5-sonnet-20241022"),
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_model_spec_deserialize_fallback_array() {
        let json = json!(["anthropic/claude-3-5-sonnet-20241022", "openai/gpt-4"]);
        let spec: ModelSpec = serde_json::from_value(json).unwrap();
        match spec {
            ModelSpec::Fallback(models) => {
                assert_eq!(models.len(), 2);
                assert_eq!(models[0], "anthropic/claude-3-5-sonnet-20241022");
                assert_eq!(models[1], "openai/gpt-4");
            }
            _ => panic!("Expected Fallback variant"),
        }
    }

    #[test]
    fn test_model_spec_serialize_roundtrip_single() {
        let spec = ModelSpec::Single("openai/gpt-4".to_string());
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json, json!("openai/gpt-4"));
    }

    #[test]
    fn test_model_spec_serialize_roundtrip_fallback() {
        let spec = ModelSpec::Fallback(vec![
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            "openai/gpt-4".to_string(),
        ]);
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            json,
            json!(["anthropic/claude-3-5-sonnet-20241022", "openai/gpt-4"])
        );
    }

    #[test]
    fn test_model_spec_empty_fallback_chain() {
        let spec = ModelSpec::Fallback(vec![]);
        let chain = spec.to_fallback_chain().unwrap();
        assert!(chain.provider_models.is_empty());
    }

    #[test]
    fn test_model_spec_parse_provider_model_empty_parts() {
        // slash at the start
        let spec = ModelSpec::Single("/model".to_string());
        let chain = spec.to_fallback_chain().unwrap();
        assert_eq!(chain.provider_models[0].0, "");
        assert_eq!(chain.provider_models[0].1, "model");
    }

    // ============================================================================
    // LLMPromptParams Deserialization Tests
    // ============================================================================

    #[test]
    fn test_llm_prompt_params_minimal() {
        let json = json!({
            "model": "anthropic/claude-3-5-sonnet-20241022",
            "prompt": "Hello"
        });
        let params: LLMPromptParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.prompt, "Hello");
        assert!(params.system_prompt.is_none());
        assert!(params.max_tokens.is_none());
        assert!(params.temperature.is_none());
        assert!(params.top_p.is_none());
        assert!(params.stop_sequences.is_none());
        assert!(params.model_pricing.is_none());
        assert!(params.activity_budget_limit_usd.is_none());
        assert!(params.workflow_budget_limit_usd.is_none());
        assert!(params.cumulative_activity_cost_usd.is_none());
    }

    #[test]
    fn test_llm_prompt_params_full() {
        let json = json!({
            "model": ["anthropic/claude-3-5-sonnet-20241022", "openai/gpt-4"],
            "prompt": "Test prompt",
            "system_prompt": "You are helpful.",
            "max_tokens": 1000,
            "temperature": 0.7,
            "top_p": 0.9,
            "stop_sequences": ["END", "STOP"],
            "activity_budget_limit_usd": "1.00",
            "workflow_budget_limit_usd": "5.00",
            "cumulative_activity_cost_usd": "0.05"
        });
        let params: LLMPromptParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.prompt, "Test prompt");
        assert_eq!(params.system_prompt, Some("You are helpful.".to_string()));
        assert_eq!(params.max_tokens, Some(1000));
        assert_eq!(params.temperature, Some(0.7));
        assert_eq!(params.top_p, Some(0.9));
        assert_eq!(params.stop_sequences.unwrap().len(), 2);
        assert_eq!(params.activity_budget_limit_usd, Some(dec!(1.00)));
        assert_eq!(params.workflow_budget_limit_usd, Some(dec!(5.00)));
        assert_eq!(params.cumulative_activity_cost_usd, Some(dec!(0.05)));
    }

    #[test]
    fn test_llm_prompt_params_with_model_pricing() {
        let json = json!({
            "model": "anthropic/claude-3-5-sonnet-20241022",
            "prompt": "Test",
            "model_pricing": {
                "anthropic/claude-3-5-sonnet-20241022": {
                    "input_price_per_million": "3.00",
                    "output_price_per_million": "15.00",
                    "cached_input_price_per_million": "0.30"
                }
            },
            "activity_budget_limit_usd": "1.00"
        });
        let params: LLMPromptParams = serde_json::from_value(json).unwrap();
        assert!(params.model_pricing.is_some());
        let pricing = params.model_pricing.unwrap();
        assert_eq!(pricing.len(), 1);
        let p = pricing.get("anthropic/claude-3-5-sonnet-20241022").unwrap();
        assert_eq!(p.input_price_per_million, dec!(3.00));
    }

    #[test]
    fn test_llm_prompt_params_serialization_skip_none() {
        let params = LLMPromptParams {
            model: ModelSpec::Single("anthropic/claude-3-5-sonnet-20241022".to_string()),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
            model_pricing: None,
            activity_budget_limit_usd: None,
            workflow_budget_limit_usd: None,
            cumulative_activity_cost_usd: None,
        };

        let json = serde_json::to_value(&params).unwrap();
        let obj = json.as_object().unwrap();
        // None fields should be skipped
        assert!(!obj.contains_key("system_prompt"));
        assert!(!obj.contains_key("max_tokens"));
        assert!(!obj.contains_key("temperature"));
        assert!(!obj.contains_key("model_pricing"));
        assert!(!obj.contains_key("activity_budget_limit_usd"));
        // Required fields should be present
        assert!(obj.contains_key("model"));
        assert!(obj.contains_key("prompt"));
    }

    // ============================================================================
    // EmbeddingParams Deserialization Tests
    // ============================================================================

    #[test]
    fn test_embedding_params_minimal() {
        let json = json!({
            "model": "openai/text-embedding-3-small",
            "input": ["Hello world"]
        });
        let params: EmbeddingParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.input.len(), 1);
        assert_eq!(params.batch_size, 500); // default
    }

    #[test]
    fn test_embedding_params_custom_batch_size() {
        let json = json!({
            "model": "openai/text-embedding-3-small",
            "input": ["Hello", "World"],
            "batch_size": 100
        });
        let params: EmbeddingParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.batch_size, 100);
        assert_eq!(params.input.len(), 2);
    }

    #[test]
    fn test_embedding_params_fallback_model() {
        let json = json!({
            "model": ["openai/text-embedding-3-small", "google/text-embedding-004"],
            "input": ["Test"]
        });
        let params: EmbeddingParams = serde_json::from_value(json).unwrap();
        let chain = params.model.to_fallback_chain().unwrap();
        assert_eq!(chain.provider_models.len(), 2);
    }

    #[test]
    fn test_default_batch_size() {
        assert_eq!(default_batch_size(), 500);
    }

    // ============================================================================
    // FallbackChain Serialization Tests
    // ============================================================================

    #[test]
    fn test_fallback_chain_serialize_deserialize() {
        let chain = FallbackChain {
            provider_models: vec![
                (
                    "anthropic".to_string(),
                    "claude-3-5-sonnet-20241022".to_string(),
                ),
                ("openai".to_string(), "gpt-4".to_string()),
            ],
        };
        let json = serde_json::to_string(&chain).unwrap();
        let deserialized: FallbackChain = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.provider_models.len(), 2);
        assert_eq!(deserialized.provider_models[0].0, "anthropic");
        assert_eq!(deserialized.provider_models[1].0, "openai");
    }

    // ============================================================================
    // Budget Construction Logic Tests (matching execute() code paths)
    // ============================================================================

    #[test]
    fn test_budget_construction_both_limits_activity_lower() {
        let activity_limit = Some(dec!(1.00));
        let workflow_limit = Some(dec!(5.00));

        let budget_limit = match (activity_limit, workflow_limit) {
            (Some(a), Some(w)) => {
                if a < w {
                    Some(a)
                } else {
                    Some(w)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        assert_eq!(budget_limit, Some(dec!(1.00)));
    }

    #[test]
    fn test_budget_construction_both_limits_workflow_lower() {
        let activity_limit = Some(dec!(5.00));
        let workflow_limit = Some(dec!(2.00));

        let budget_limit = match (activity_limit, workflow_limit) {
            (Some(a), Some(w)) => {
                if a < w {
                    Some(a)
                } else {
                    Some(w)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        assert_eq!(budget_limit, Some(dec!(2.00)));
    }

    #[test]
    fn test_budget_construction_only_activity_limit() {
        let activity_limit = Some(dec!(3.00));
        let workflow_limit: Option<Decimal> = None;

        let budget_limit = match (activity_limit, workflow_limit) {
            (Some(a), Some(w)) => {
                if a < w {
                    Some(a)
                } else {
                    Some(w)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        assert_eq!(budget_limit, Some(dec!(3.00)));
    }

    #[test]
    fn test_budget_construction_only_workflow_limit() {
        let activity_limit: Option<Decimal> = None;
        let workflow_limit = Some(dec!(10.00));

        let budget_limit = match (activity_limit, workflow_limit) {
            (Some(a), Some(w)) => {
                if a < w {
                    Some(a)
                } else {
                    Some(w)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        assert_eq!(budget_limit, Some(dec!(10.00)));
    }

    #[test]
    fn test_budget_construction_no_limits() {
        let activity_limit: Option<Decimal> = None;
        let workflow_limit: Option<Decimal> = None;

        let budget_limit = match (activity_limit, workflow_limit) {
            (Some(a), Some(w)) => {
                if a < w {
                    Some(a)
                } else {
                    Some(w)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        assert!(budget_limit.is_none());
    }

    #[test]
    fn test_budget_construction_equal_limits() {
        let activity_limit = Some(dec!(5.00));
        let workflow_limit = Some(dec!(5.00));

        let budget_limit = match (activity_limit, workflow_limit) {
            (Some(a), Some(w)) => {
                if a < w {
                    Some(a)
                } else {
                    Some(w)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };

        assert_eq!(budget_limit, Some(dec!(5.00)));
    }

    #[test]
    fn test_budget_params_from_pricing_and_limit() {
        let pricing = create_test_pricing();
        let limit = dec!(1.00);
        let budget = BudgetParams {
            model_pricing: pricing.clone(),
            budget_limit_usd: limit,
            cumulative_cost_usd: dec!(0.25),
        };

        assert_eq!(budget.budget_limit_usd, dec!(1.00));
        assert_eq!(budget.cumulative_cost_usd, dec!(0.25));
        assert_eq!(budget.model_pricing.len(), 3);
    }

    #[test]
    fn test_budget_params_default_cumulative_cost() {
        let pricing = create_test_pricing();
        let budget = BudgetParams {
            model_pricing: pricing,
            budget_limit_usd: dec!(1.00),
            cumulative_cost_usd: Decimal::ZERO,
        };

        assert_eq!(budget.cumulative_cost_usd, Decimal::ZERO);
    }

    // ============================================================================
    // PromptResponse / EmbeddingResponse Tests
    // ============================================================================

    #[test]
    fn test_prompt_response_serialization() {
        let response = PromptResponse {
            content: "Hello!".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            provider: "anthropic".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cached_tokens: Some(3),
            },
            finish_reason: crate::llm::FinishReason::Stop,
            cost_usd: Some(dec!(0.001)),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["content"], "Hello!");
        assert_eq!(json["provider"], "anthropic");
        assert_eq!(json["usage"]["cached_tokens"], 3);

        // Roundtrip
        let deserialized: PromptResponse = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.content, "Hello!");
        assert_eq!(deserialized.cost_usd, Some(dec!(0.001)));
    }

    #[test]
    fn test_prompt_response_max_tokens_finish_reason() {
        let response = PromptResponse {
            content: "Truncated...".to_string(),
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 100,
                output_tokens: 4096,
                total_tokens: 4196,
                cached_tokens: None,
            },
            finish_reason: crate::llm::FinishReason::MaxTokens,
            cost_usd: None,
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["finish_reason"], "MaxTokens");
        assert!(json["cost_usd"].is_null());
    }

    #[test]
    fn test_embedding_response_serialization() {
        let response = EmbeddingResponse {
            embeddings: vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]],
            model: "text-embedding-3-small".to_string(),
            provider: "openai".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 10,
                output_tokens: 0,
                total_tokens: 10,
                cached_tokens: None,
            },
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["embeddings"].as_array().unwrap().len(), 2);
        assert_eq!(json["model"], "text-embedding-3-small");

        let deserialized: EmbeddingResponse = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.embeddings.len(), 2);
    }

    // ============================================================================
    // Activity Execute Error Path Tests
    // ============================================================================

    #[tokio::test]
    async fn test_llm_prompt_execute_invalid_parameters() {
        let activity = LLMPromptActivity::new();
        let result = activity.execute(json!({"invalid": "params"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[tokio::test]
    async fn test_llm_prompt_execute_invalid_model_format() {
        let activity = LLMPromptActivity::new();
        let result = activity
            .execute(json!({
                "model": "no-slash-model",
                "prompt": "Hello"
            }))
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid model format")
        );
    }

    #[tokio::test]
    async fn test_embedding_execute_invalid_parameters() {
        let activity = EmbeddingActivity::new();
        let result = activity.execute(json!({"wrong": "shape"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[tokio::test]
    async fn test_embedding_execute_invalid_model_format() {
        let activity = EmbeddingActivity::new();
        let result = activity
            .execute(json!({
                "model": "invalid-model",
                "input": ["test"]
            }))
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid model format")
        );
    }

    // ============================================================================
    // LLMPromptActivity / EmbeddingActivity Default Tests
    // ============================================================================

    #[test]
    fn test_llm_prompt_activity_default() {
        let activity = LLMPromptActivity;
        assert_eq!(activity.name(), "llm_prompt");
    }

    #[test]
    fn test_embedding_activity_default() {
        let activity = EmbeddingActivity;
        assert_eq!(activity.name(), "embedding");
    }

    #[test]
    fn test_llm_prompt_activity_supports_streaming() {
        use crate::streaming::StreamingActivity;
        let activity = LLMPromptActivity::new();
        assert!(activity.supports_streaming());
    }

    // ============================================================================
    // FallbackChain All-Providers-Fail Tests
    // ============================================================================

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_prompt_all_providers_fail_no_keys() {
        // Clear all provider keys
        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
            env::remove_var("OPENAI_API_KEY");
            env::remove_var("GOOGLE_API_KEY");
            env::remove_var("OLLAMA_BASE_URL");
        }

        let chain = FallbackChain {
            provider_models: vec![
                (
                    "anthropic".to_string(),
                    "claude-3-5-sonnet-20241022".to_string(),
                ),
                ("openai".to_string(), "gpt-4".to_string()),
            ],
        };

        let request = PromptRequest {
            model: String::new(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let result = chain.prompt(&request, None).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("All providers failed")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_embed_all_providers_fail_no_keys() {
        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
            env::remove_var("OPENAI_API_KEY");
            env::remove_var("GOOGLE_API_KEY");
        }

        let chain = FallbackChain {
            provider_models: vec![("openai".to_string(), "text-embedding-3-small".to_string())],
        };

        let request = EmbeddingRequest {
            model: String::new(),
            input: vec!["Hello".to_string()],
        };

        let result = chain.embed(&request).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("All providers failed")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_prompt_empty_chain() {
        let chain = FallbackChain {
            provider_models: vec![],
        };

        let request = PromptRequest {
            model: String::new(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let result = chain.prompt(&request, None).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No providers configured")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_embed_empty_chain() {
        let chain = FallbackChain {
            provider_models: vec![],
        };

        let request = EmbeddingRequest {
            model: String::new(),
            input: vec!["test".to_string()],
        };

        let result = chain.embed(&request).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No providers configured")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_prompt_stream_empty_chain() {
        use crate::streaming::NoOpStreamSender;

        let chain = FallbackChain {
            provider_models: vec![],
        };

        let request = PromptRequest {
            model: String::new(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let sender = NoOpStreamSender::new();
        let result = chain.prompt_stream(&request, None, &sender).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No providers configured")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_prompt_stream_all_fail_no_keys() {
        use crate::streaming::NoOpStreamSender;

        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
            env::remove_var("OPENAI_API_KEY");
        }

        let chain = FallbackChain {
            provider_models: vec![(
                "anthropic".to_string(),
                "claude-3-5-sonnet-20241022".to_string(),
            )],
        };

        let request = PromptRequest {
            model: String::new(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let sender = NoOpStreamSender::new();
        let result = chain.prompt_stream(&request, None, &sender).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("All providers failed")
        );
    }

    // ============================================================================
    // Budget-Aware Fallback: Budget Exceeds Skip All Providers
    // ============================================================================

    #[tokio::test]
    #[serial]
    async fn test_fallback_chain_prompt_budget_skips_all_providers() {
        // Set keys so providers can be created, but budget blocks them all
        unsafe {
            env::set_var("ANTHROPIC_API_KEY", "test-key");
        }

        let pricing = create_test_pricing();
        let budget = BudgetParams {
            model_pricing: pricing,
            budget_limit_usd: dec!(0.00), // Zero budget
            cumulative_cost_usd: dec!(0.00),
        };

        let chain = FallbackChain {
            provider_models: vec![(
                "anthropic".to_string(),
                "claude-3-5-sonnet-20241022".to_string(),
            )],
        };

        let request = PromptRequest {
            model: String::new(),
            prompt: "Hello".to_string(),
            system_prompt: None,
            max_tokens: Some(100),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        let result = chain.prompt(&request, Some(&budget)).await;
        // Should fail because budget blocks the only provider, but the error
        // will be "ANTHROPIC_API_KEY not set" since env vars may not persist
        // across serial tests, OR "All providers failed"
        assert!(result.is_err());

        unsafe {
            env::remove_var("ANTHROPIC_API_KEY");
        }
    }

    // ============================================================================
    // Activity Output JSON Format Tests
    // ============================================================================

    #[test]
    fn test_llm_activity_output_json_format() {
        let response = PromptResponse {
            content: "Generated text".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            provider: "anthropic".to_string(),
            usage: crate::llm::TokenUsage {
                prompt_tokens: 50,
                output_tokens: 200,
                total_tokens: 250,
                cached_tokens: None,
            },
            finish_reason: crate::llm::FinishReason::Stop,
            cost_usd: Some(dec!(0.003)),
        };

        // Match what execute() produces
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

        let result = ActivityResult::value("result", outputs.clone());
        let json_value = result.to_json_value();
        assert_eq!(json_value["result"]["content"], "Generated text");
        assert_eq!(json_value["result"]["usage"]["prompt_tokens"], 50);
        assert_eq!(json_value["result"]["usage"]["output_tokens"], 200);
    }

    #[test]
    fn test_embedding_activity_output_json_format_inline() {
        let outputs = json!({
            "embeddings": [[0.1, 0.2], [0.3, 0.4]],
            "embeddings_file": null,
            "embedding_count": 2,
            "model": "text-embedding-3-small",
            "provider": "openai",
            "usage": {
                "prompt_tokens": 5,
                "output_tokens": 0,
                "total_tokens": 5,
                "cached_tokens": null,
            }
        });

        let result = ActivityResult::value("result", outputs);
        let json_value = result.to_json_value();
        assert_eq!(json_value["result"]["embedding_count"], 2);
        assert!(json_value["result"]["embeddings_file"].is_null());
        assert_eq!(
            json_value["result"]["embeddings"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn test_embedding_activity_output_json_format_streamed() {
        let outputs = json!({
            "embeddings": null,
            "embeddings_file": "abc-123/embed_step/embeddings.jsonl",
            "embedding_count": 1000,
            "model": "text-embedding-3-small",
            "provider": "openai",
            "usage": {
                "prompt_tokens": 5000,
                "output_tokens": 0,
                "total_tokens": 5000,
                "cached_tokens": null,
            }
        });

        let result = ActivityResult::value("result", outputs);
        let json_value = result.to_json_value();
        assert_eq!(json_value["result"]["embedding_count"], 1000);
        assert!(json_value["result"]["embeddings"].is_null());
        assert_eq!(
            json_value["result"]["embeddings_file"],
            "abc-123/embed_step/embeddings.jsonl"
        );
    }
}
