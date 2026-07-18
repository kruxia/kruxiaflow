use anyhow::Result;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

pub struct CostCalculator {
    pool: PgPool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_price_per_million: Decimal,
    pub output_price_per_million: Decimal,
    pub cached_input_price_per_million: Option<Decimal>,
    /// Price for prompt-cache writes (e.g., Anthropic bills 1.25x input for the
    /// 5-minute TTL). None falls back to the input-token price.
    /// serde default keeps older serialized pricing maps deserializable.
    #[serde(default)]
    pub cache_write_price_per_million: Option<Decimal>,
}

impl CostCalculator {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Estimate token count for text using provider-specific heuristics
    /// Uses both character-based and word-based estimates, returns average
    ///
    /// # Provider-specific ratios:
    /// - Anthropic: 3.5 chars/token, 0.85 words/token
    /// - OpenAI: 4.0 chars/token, 0.75 words/token
    /// - Google: 4.0 chars/token, 0.75 words/token
    /// - Ollama: 4.0 chars/token, 0.75 words/token
    /// - Others: 4.0 chars/token, 0.75 words/token (conservative default)
    pub fn estimate_tokens(provider: &str, text: &str) -> u32 {
        let (chars_per_token, words_per_token) = match provider {
            "anthropic" => (3.5, 0.85),
            "openai" | "google" | "ollama" => (4.0, 0.75),
            _ => (4.0, 0.75), // Conservative default
        };

        // Character-based estimate
        let char_estimate = text.len() as f64 / chars_per_token;

        // Word-based estimate (split on whitespace)
        let word_count = text.split_whitespace().count() as f64;
        let word_estimate = word_count / words_per_token;

        // Return average of both estimates
        ((char_estimate + word_estimate) / 2.0).ceil() as u32
    }

    /// Estimate cost for LLM request before execution
    /// Returns estimated cost in USD
    ///
    /// # Arguments
    /// * `provider` - LLM provider name (e.g., "openai", "anthropic")
    /// * `model` - Model name (e.g., "gpt-4o", "claude-3-5-sonnet-20241022")
    /// * `prompt` - The prompt text to estimate
    /// * `max_tokens` - Maximum completion tokens (conservative: assumes full usage)
    ///
    /// # Returns
    /// Estimated cost in USD, using simple token estimation heuristic
    pub async fn estimate_llm_cost(
        &self,
        provider: &str,
        model: &str,
        prompt: &str,
        max_tokens: u32,
    ) -> Result<Decimal> {
        // Fetch pricing from database
        let pricing = sqlx::query_as!(
            ModelPricing,
            r#"
            SELECT
                input_price_per_million,
                output_price_per_million,
                cached_input_price_per_million,
                cache_write_price_per_million
            FROM llm_models
            WHERE provider = $1 AND name = $2
            "#,
            provider,
            model
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Model not found: {}/{}", provider, model))?;

        // Estimate prompt tokens using provider-specific heuristics
        let prompt_tokens = Self::estimate_tokens(provider, prompt);

        // Conservative: assume full max_tokens usage for completion
        let completion_tokens = max_tokens;

        Ok(Self::calculate_cost_from_pricing(
            &pricing,
            prompt_tokens,
            completion_tokens,
            None, // Don't assume caching for estimates
        ))
    }

    /// Calculate cost for single LLM usage
    /// Returns cost in USD
    pub async fn calculate_llm_cost(
        &self,
        provider: &str,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        cached_tokens: Option<u32>,
    ) -> Result<Decimal> {
        // Fetch pricing from database - only pricing fields
        let pricing = sqlx::query_as!(
            ModelPricing,
            r#"
            SELECT
                input_price_per_million,
                output_price_per_million,
                cached_input_price_per_million,
                cache_write_price_per_million
            FROM llm_models
            WHERE provider = $1 AND name = $2
            "#,
            provider,
            model
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Model not found: {}/{}", provider, model))?;

        Ok(Self::calculate_cost_from_pricing(
            &pricing,
            prompt_tokens,
            completion_tokens,
            cached_tokens,
        ))
    }

    /// Batch calculate costs for multiple models
    /// Efficient for calculating costs across many activities at once
    /// Returns HashMap with keys in "provider/model" format for JSON serializability
    pub async fn batch_get_pricing(
        &self,
        models: &[(String, String)], // Vec of (provider, model)
    ) -> Result<HashMap<String, ModelPricing>> {
        if models.is_empty() {
            return Ok(HashMap::new());
        }

        let providers: Vec<String> = models.iter().map(|(p, _)| p.clone()).collect();
        let model_names: Vec<String> = models.iter().map(|(_, m)| m.clone()).collect();

        // Batch query using array patterns (similar to search endpoint)
        let results = sqlx::query!(
            r#"
            SELECT
                provider,
                name as model,
                input_price_per_million,
                output_price_per_million,
                cached_input_price_per_million,
                cache_write_price_per_million
            FROM llm_models
            WHERE (provider, name) IN (
                SELECT UNNEST($1::text[]), UNNEST($2::text[])
            )
            "#,
            &providers,
            &model_names
        )
        .fetch_all(&self.pool)
        .await?;

        let mut pricing_map = HashMap::new();
        for row in results {
            // Use "provider/model" format as key for JSON serializability
            let key = format!("{}/{}", row.provider, row.model);
            pricing_map.insert(
                key,
                ModelPricing {
                    input_price_per_million: row.input_price_per_million,
                    output_price_per_million: row.output_price_per_million,
                    cached_input_price_per_million: row.cached_input_price_per_million,
                    cache_write_price_per_million: row.cache_write_price_per_million,
                },
            );
        }

        Ok(pricing_map)
    }

    /// Calculate cost for a reported usage entry from pricing data.
    ///
    /// `input_tokens` includes cache reads (same convention as
    /// `calculate_cost_from_pricing`). `cache_creation_tokens` are billed at
    /// the catalog's cache-write price when present (e.g., 1.25x input for
    /// Anthropic's 5-minute TTL), falling back to the input-token price when
    /// the catalog has no cache-write price for the model.
    pub fn calculate_usage_entry_cost(
        pricing: &ModelPricing,
        entry: &crate::cost::UsageEntry,
    ) -> Decimal {
        let base = Self::calculate_cost_from_pricing(
            pricing,
            entry.input_tokens,
            entry.output_tokens,
            Some(entry.cache_read_tokens),
        );
        let cache_write_price = pricing
            .cache_write_price_per_million
            .unwrap_or(pricing.input_price_per_million);
        let cache_creation_cost = Decimal::from(entry.cache_creation_tokens) * cache_write_price
            / Decimal::from(1_000_000);
        base + cache_creation_cost
    }

    /// Calculate cost from pricing data
    pub fn calculate_cost_from_pricing(
        pricing: &ModelPricing,
        prompt_tokens: u32,
        completion_tokens: u32,
        cached_tokens: Option<u32>,
    ) -> Decimal {
        let one_million = Decimal::from(1_000_000);

        // Calculate input cost
        let input_cost = if let (Some(cached_price), Some(cached)) =
            (pricing.cached_input_price_per_million, cached_tokens)
        {
            // Use cached price for cached tokens, regular price for remaining
            let regular_tokens = prompt_tokens.saturating_sub(cached);
            (Decimal::from(regular_tokens) * pricing.input_price_per_million / one_million)
                + (Decimal::from(cached) * cached_price / one_million)
        } else {
            Decimal::from(prompt_tokens) * pricing.input_price_per_million / one_million
        };

        // Calculate output cost
        let output_cost =
            Decimal::from(completion_tokens) * pricing.output_price_per_million / one_million;

        input_cost + output_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use std::str::FromStr;

    // ============================================================================
    // Unit Tests (No Database Required)
    // ============================================================================

    #[test]
    fn test_estimate_tokens_anthropic() {
        let text = "Hello world, this is a test.";
        // 29 chars, 6 words
        // char_estimate = 29 / 3.5 = 8.29
        // word_estimate = 6 / 0.85 = 7.06
        // average = (8.29 + 7.06) / 2 = 7.67, ceil = 8
        let tokens = CostCalculator::estimate_tokens("anthropic", text);
        assert_eq!(tokens, 8);
    }

    #[test]
    fn test_estimate_tokens_openai() {
        let text = "Hello world, this is a test.";
        // 29 chars, 6 words
        // char_estimate = 29 / 4.0 = 7.25
        // word_estimate = 6 / 0.75 = 8.0
        // average = (7.25 + 8.0) / 2 = 7.625, ceil = 8
        let tokens = CostCalculator::estimate_tokens("openai", text);
        assert_eq!(tokens, 8);
    }

    #[test]
    fn test_estimate_tokens_google() {
        let text = "Hello world, this is a test.";
        let tokens = CostCalculator::estimate_tokens("google", text);
        assert_eq!(tokens, 8);
    }

    #[test]
    fn test_estimate_tokens_ollama() {
        let text = "Hello world, this is a test.";
        let tokens = CostCalculator::estimate_tokens("ollama", text);
        assert_eq!(tokens, 8);
    }

    #[test]
    fn test_estimate_tokens_unknown_provider() {
        let text = "Hello world, this is a test.";
        // Unknown provider uses default: 4.0 chars/token, 0.75 words/token
        let tokens = CostCalculator::estimate_tokens("unknown-provider", text);
        assert_eq!(tokens, 8);
    }

    #[test]
    fn test_estimate_tokens_empty_string() {
        let tokens = CostCalculator::estimate_tokens("openai", "");
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_estimate_tokens_longer_text() {
        let text = "This is a much longer test message with multiple sentences. \
                    It should estimate a higher token count than the short message.";
        // Should be significantly more than 8 tokens
        let tokens = CostCalculator::estimate_tokens("openai", text);
        assert!(tokens > 20, "Expected > 20 tokens, got {}", tokens);
    }

    #[test]
    fn test_calculate_cost_from_pricing_basic() {
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: None,
            cache_write_price_per_million: None,
        };

        // 1000 prompt tokens, 500 completion tokens
        // Cost = (1000 × 3.00 / 1M) + (500 × 15.00 / 1M)
        //      = 0.003 + 0.0075
        //      = 0.0105
        let cost = CostCalculator::calculate_cost_from_pricing(&pricing, 1000, 500, None);
        assert_eq!(cost, Decimal::from_str("0.0105").unwrap());
    }

    #[test]
    fn test_calculate_cost_from_pricing_with_caching() {
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: Some(dec!(0.30)),
            cache_write_price_per_million: None,
        };

        // 1000 prompt tokens (600 cached), 500 completion tokens
        // Cost = (400 × 3.00 / 1M) + (600 × 0.30 / 1M) + (500 × 15.00 / 1M)
        //      = 0.0012 + 0.00018 + 0.0075
        //      = 0.00888
        let cost = CostCalculator::calculate_cost_from_pricing(&pricing, 1000, 500, Some(600));
        assert_eq!(cost, Decimal::from_str("0.00888").unwrap());
    }

    #[test]
    fn test_calculate_cost_from_pricing_zero_tokens() {
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: None,
            cache_write_price_per_million: None,
        };

        let cost = CostCalculator::calculate_cost_from_pricing(&pricing, 0, 0, None);
        assert_eq!(cost, Decimal::ZERO);
    }

    #[test]
    fn test_calculate_cost_from_pricing_large_numbers() {
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: None,
            cache_write_price_per_million: None,
        };

        // 100,000 prompt tokens, 50,000 completion tokens
        // Cost = (100000 × 3.00 / 1M) + (50000 × 15.00 / 1M)
        //      = 0.30 + 0.75
        //      = 1.05
        let cost = CostCalculator::calculate_cost_from_pricing(&pricing, 100_000, 50_000, None);
        assert_eq!(cost, Decimal::from_str("1.05").unwrap());
    }

    #[test]
    fn test_calculate_cost_from_pricing_all_cached() {
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: Some(dec!(0.30)),
            cache_write_price_per_million: None,
        };

        // All 1000 prompt tokens are cached
        // Cost = (0 × 3.00 / 1M) + (1000 × 0.30 / 1M) + (500 × 15.00 / 1M)
        //      = 0 + 0.0003 + 0.0075
        //      = 0.0078
        let cost = CostCalculator::calculate_cost_from_pricing(&pricing, 1000, 500, Some(1000));
        assert_eq!(cost, Decimal::from_str("0.0078").unwrap());
    }

    // ============================================================================
    // Regression Tests: Model Pricing JSON Serialization
    // Prevents: docs/bugs/2026-01-04-model-pricing-tuple-key-serialization.md
    // ============================================================================

    #[test]
    fn test_model_pricing_json_serialization() {
        // Verify ModelPricing struct serializes to JSON correctly
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: Some(dec!(0.30)),
            cache_write_price_per_million: None,
        };

        let json = serde_json::to_value(&pricing).expect("ModelPricing should serialize to JSON");
        assert!(json.is_object());

        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("input_price_per_million"));
        assert!(obj.contains_key("output_price_per_million"));
        assert!(obj.contains_key("cached_input_price_per_million"));
    }

    #[test]
    fn test_model_pricing_json_roundtrip() {
        // Verify ModelPricing can roundtrip through JSON
        let pricing = ModelPricing {
            input_price_per_million: dec!(3.00),
            output_price_per_million: dec!(15.00),
            cached_input_price_per_million: Some(dec!(0.30)),
            cache_write_price_per_million: None,
        };

        let json = serde_json::to_string(&pricing).expect("Should serialize");
        let roundtrip: ModelPricing = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(
            roundtrip.input_price_per_million,
            pricing.input_price_per_million
        );
        assert_eq!(
            roundtrip.output_price_per_million,
            pricing.output_price_per_million
        );
        assert_eq!(
            roundtrip.cached_input_price_per_million,
            pricing.cached_input_price_per_million
        );
    }

    #[test]
    fn test_model_pricing_hashmap_string_keys_serializable() {
        // Regression test: HashMap<String, ModelPricing> should serialize to JSON
        // This is the FIXED behavior after the bug was resolved
        let mut pricing_map: HashMap<String, ModelPricing> = HashMap::new();

        pricing_map.insert(
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            ModelPricing {
                input_price_per_million: dec!(3.00),
                output_price_per_million: dec!(15.00),
                cached_input_price_per_million: Some(dec!(0.30)),
                cache_write_price_per_million: None,
            },
        );
        pricing_map.insert(
            "anthropic/claude-3-5-haiku-20241022".to_string(),
            ModelPricing {
                input_price_per_million: dec!(0.80),
                output_price_per_million: dec!(4.00),
                cached_input_price_per_million: Some(dec!(0.08)),
                cache_write_price_per_million: None,
            },
        );

        // This MUST succeed - verifies the fix is in place
        let json = serde_json::to_value(&pricing_map)
            .expect("HashMap<String, ModelPricing> must serialize to JSON");

        assert!(json.is_object(), "JSON should be an object");
        let obj = json.as_object().unwrap();
        assert!(
            obj.contains_key("anthropic/claude-3-5-sonnet-20241022"),
            "Should have sonnet key"
        );
        assert!(
            obj.contains_key("anthropic/claude-3-5-haiku-20241022"),
            "Should have haiku key"
        );
    }

    #[test]
    fn test_model_pricing_hashmap_tuple_keys_not_serializable() {
        // Documents the BROKEN behavior that was fixed
        // HashMap<(String, String), ModelPricing> CANNOT serialize to JSON
        // because JSON only supports string keys
        let mut pricing_map: HashMap<(String, String), ModelPricing> = HashMap::new();

        pricing_map.insert(
            (
                "anthropic".to_string(),
                "claude-3-5-sonnet-20241022".to_string(),
            ),
            ModelPricing {
                input_price_per_million: dec!(3.00),
                output_price_per_million: dec!(15.00),
                cached_input_price_per_million: Some(dec!(0.30)),
                cache_write_price_per_million: None,
            },
        );

        // This MUST fail - JSON doesn't support tuple keys
        let result = serde_json::to_value(&pricing_map);
        assert!(
            result.is_err(),
            "HashMap with tuple keys should NOT serialize to JSON"
        );

        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("key must be a string"),
            "Error should mention string keys: {}",
            err
        );
    }

    #[test]
    fn test_model_pricing_hashmap_string_keys_roundtrip() {
        // Verify HashMap<String, ModelPricing> can roundtrip through JSON
        let mut pricing_map: HashMap<String, ModelPricing> = HashMap::new();

        pricing_map.insert(
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            ModelPricing {
                input_price_per_million: dec!(3.00),
                output_price_per_million: dec!(15.00),
                cached_input_price_per_million: Some(dec!(0.30)),
                cache_write_price_per_million: None,
            },
        );
        pricing_map.insert(
            "ollama/llama3.2".to_string(),
            ModelPricing {
                input_price_per_million: dec!(0.00),
                output_price_per_million: dec!(0.00),
                cached_input_price_per_million: None,
                cache_write_price_per_million: None,
            },
        );

        let json = serde_json::to_string(&pricing_map).expect("Should serialize");
        let roundtrip: HashMap<String, ModelPricing> =
            serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(roundtrip.len(), 2);
        assert!(roundtrip.contains_key("anthropic/claude-3-5-sonnet-20241022"));
        assert!(roundtrip.contains_key("ollama/llama3.2"));

        let sonnet = roundtrip
            .get("anthropic/claude-3-5-sonnet-20241022")
            .unwrap();
        assert_eq!(sonnet.input_price_per_million, dec!(3.00));
        assert_eq!(sonnet.output_price_per_million, dec!(15.00));
    }

    #[test]
    fn test_model_pricing_key_format_provider_slash_model() {
        // Verify the key format is exactly "provider/model"
        // This is important for the orchestrator to access pricing correctly
        let provider = "anthropic";
        let model = "claude-3-5-sonnet-20241022";
        let key = format!("{}/{}", provider, model);

        assert_eq!(key, "anthropic/claude-3-5-sonnet-20241022");

        // Verify we can parse the key back to provider/model
        let parts: Vec<&str> = key.splitn(2, '/').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], provider);
        assert_eq!(parts[1], model);
    }

    // ============================================================================
    // Integration Tests (Require Database)
    // ============================================================================

    #[sqlx::test(migrations = "../migrations")]
    async fn test_calculate_llm_cost_model_not_found(pool: PgPool) {
        let calculator = CostCalculator::new(pool);

        let result = calculator
            .calculate_llm_cost("nonexistent", "model", 1000, 500, None)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Model not found"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_estimate_llm_cost_model_not_found(pool: PgPool) {
        let calculator = CostCalculator::new(pool);

        let result = calculator
            .estimate_llm_cost("nonexistent", "model", "test prompt", 500)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Model not found"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_batch_get_pricing_empty(pool: PgPool) {
        let calculator = CostCalculator::new(pool);

        let result = calculator.batch_get_pricing(&[]).await.unwrap();

        assert!(result.is_empty());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_batch_get_pricing_nonexistent_models(pool: PgPool) {
        let calculator = CostCalculator::new(pool);

        let models = vec![
            ("nonexistent1".to_string(), "model1".to_string()),
            ("nonexistent2".to_string(), "model2".to_string()),
        ];

        let result = calculator.batch_get_pricing(&models).await.unwrap();

        // Should return empty map for nonexistent models
        assert!(result.is_empty());
    }
}
