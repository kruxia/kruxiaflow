use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Per-LLM-call usage reported by an external activity on completion or failure.
///
/// Field names are frozen as part of the worker-API contract (worker SDK spec):
/// `provider`, `model`, `input_tokens`, `output_tokens`, `cache_read_tokens`,
/// `cache_creation_tokens`, `cache_storage_token_hours`, `cost_usd`.
/// (`cache_storage_token_hours` was added post-freeze under the additive-only
/// discipline: absent on the wire keeps the prior shape byte-for-byte.)
///
/// Conventions:
/// - `input_tokens` is the full prompt size including cache reads (same
///   convention as `activity_costs.prompt_tokens` for built-in `llm_prompt` rows).
/// - `cache_read_tokens` are billed at the catalog's cached-input price.
/// - `cache_creation_tokens` are billed at the catalog's cache-write price
///   (`llm_models.cache_write_price_per_million`, e.g., 1.25x input for
///   Anthropic); models without one fall back to the input-token price.
/// - `cache_storage_token_hours` (fractional; tokens held x hours held) are
///   billed at the catalog's cache-storage price
///   (`llm_models.cache_storage_price_per_million_token_hours`, e.g., Gemini
///   explicit-caching storage); models without one record the component at 0
///   with a warning — a time-based dimension has no sensible fallback price.
/// - `cost_usd`, when present, is used verbatim instead of server-side computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_tokens: u32,
    #[serde(default)]
    pub cache_creation_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_storage_token_hours: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Decimal>,
}
