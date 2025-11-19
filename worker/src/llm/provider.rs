use async_trait::async_trait;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// LLM provider interface
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Provider name (anthropic, openai, google, etc.)
    fn name(&self) -> &str;

    /// Generate response from prompt
    async fn prompt(&self, request: &PromptRequest) -> Result<PromptResponse>;

    /// Generate streaming prompt response (post-MVP)
    async fn prompt_stream(
        &self,
        request: &PromptRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PromptChunk>> + Send>>>;

    /// Generate embeddings
    async fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse>;
}

/// Prompt request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    pub model: String,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop_sequences: Option<Vec<String>>,
}

/// Prompt response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub finish_reason: FinishReason,
    // NOTE: No cost_usd field - orchestrator calculates cost
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub cached_tokens: Option<u32>, // For providers with prompt caching
}

/// Prompt finish reason
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    MaxTokens,
    ContentFilter,
    Error,
}

/// Streaming chunk (post-MVP)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptChunk {
    pub content: String,
    pub finish_reason: Option<FinishReason>,
}

/// Embedding request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
}

/// Embedding response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub embeddings: Vec<Vec<f64>>,
    pub model: String,
    pub usage: TokenUsage,
    // NOTE: No cost_usd field - orchestrator calculates cost
}

pub type Result<T> = std::result::Result<T, LLMError>;

#[derive(Debug, thiserror::Error)]
pub enum LLMError {
    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Invalid model: {0}")]
    InvalidModel(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Authentication failed")]
    AuthenticationFailed,

    #[error("Insufficient quota")]
    InsufficientQuota,

    #[error("Request error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}
