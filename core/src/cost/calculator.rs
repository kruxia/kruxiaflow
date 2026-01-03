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
                cached_input_price_per_million
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
                cached_input_price_per_million
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
    pub async fn batch_get_pricing(
        &self,
        models: &[(String, String)], // Vec of (provider, model)
    ) -> Result<HashMap<(String, String), ModelPricing>> {
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
                cached_input_price_per_million
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
            let key = (row.provider, row.model);
            pricing_map.insert(
                key,
                ModelPricing {
                    input_price_per_million: row.input_price_per_million,
                    output_price_per_million: row.output_price_per_million,
                    cached_input_price_per_million: row.cached_input_price_per_million,
                },
            );
        }

        Ok(pricing_map)
    }

    /// Calculate cost from pricing data
    fn calculate_cost_from_pricing(
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
        };

        // All 1000 prompt tokens are cached
        // Cost = (0 × 3.00 / 1M) + (1000 × 0.30 / 1M) + (500 × 15.00 / 1M)
        //      = 0 + 0.0003 + 0.0075
        //      = 0.0078
        let cost = CostCalculator::calculate_cost_from_pricing(&pricing, 1000, 500, Some(1000));
        assert_eq!(cost, Decimal::from_str("0.0078").unwrap());
    }

    // ============================================================================
    // Integration Tests (Require Database)
    // ============================================================================

    async fn setup_test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });

        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    #[tokio::test]
    async fn test_calculate_llm_cost_model_not_found() {
        let pool = setup_test_pool().await;
        let calculator = CostCalculator::new(pool);

        let result = calculator
            .calculate_llm_cost("nonexistent", "model", 1000, 500, None)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Model not found"));
    }

    #[tokio::test]
    async fn test_estimate_llm_cost_model_not_found() {
        let pool = setup_test_pool().await;
        let calculator = CostCalculator::new(pool);

        let result = calculator
            .estimate_llm_cost("nonexistent", "model", "test prompt", 500)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Model not found"));
    }

    #[tokio::test]
    async fn test_batch_get_pricing_empty() {
        let pool = setup_test_pool().await;
        let calculator = CostCalculator::new(pool);

        let result = calculator.batch_get_pricing(&[]).await.unwrap();

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_batch_get_pricing_nonexistent_models() {
        let pool = setup_test_pool().await;
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
