use anyhow::Result;
use rust_decimal::Decimal;
use serde::Deserialize;
use sqlx::PgPool;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ProviderCatalog {
    pub providers: Vec<ProviderDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderDefinition {
    pub name: String,
    pub display_name: String,
    pub api_endpoint: Option<String>,
    pub supports_completion: bool,
    pub supports_embeddings: bool,
    pub supports_streaming: bool,
    pub requires_api_key: bool,
    pub models: Vec<ModelDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct ModelDefinition {
    pub name: String,
    pub display_name: String,
    pub input_price_per_million: Decimal,
    pub output_price_per_million: Decimal,
    pub cached_input_price_per_million: Option<Decimal>,
    pub cache_write_price_per_million: Option<Decimal>,
    #[serde(default)]
    pub cache_storage_price_per_million_token_hours: Option<Decimal>,
    pub supports_completion: Option<bool>,
    pub supports_embeddings: Option<bool>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
}

pub async fn load_catalog_from_yaml(pool: &PgPool, yaml_path: &Path) -> Result<()> {
    let yaml_content = tokio::fs::read_to_string(yaml_path).await?;
    let catalog: ProviderCatalog = serde_yaml::from_str(&yaml_content)?;

    // Start transaction
    let mut tx = pool.begin().await?;

    for provider in catalog.providers {
        // Insert provider
        sqlx::query!(
            r#"
            INSERT INTO llm_providers (
                name, display_name, api_endpoint,
                supports_completion, supports_embeddings, supports_streaming,
                requires_api_key
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (name) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                api_endpoint = EXCLUDED.api_endpoint,
                supports_completion = EXCLUDED.supports_completion,
                supports_embeddings = EXCLUDED.supports_embeddings,
                supports_streaming = EXCLUDED.supports_streaming,
                requires_api_key = EXCLUDED.requires_api_key,
                updated_at = NOW()
            "#,
            provider.name,
            provider.display_name,
            provider.api_endpoint,
            provider.supports_completion,
            provider.supports_embeddings,
            provider.supports_streaming,
            provider.requires_api_key
        )
        .execute(&mut *tx)
        .await?;

        // Insert models for this provider
        for model in provider.models {
            sqlx::query!(
                r#"
                INSERT INTO llm_models (
                    provider, name, display_name,
                    input_price_per_million, output_price_per_million,
                    cached_input_price_per_million, cache_write_price_per_million,
                    cache_storage_price_per_million_token_hours,
                    supports_completion, supports_embeddings,
                    context_window, max_output_tokens
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                ON CONFLICT (provider, name) DO UPDATE SET
                    display_name = EXCLUDED.display_name,
                    input_price_per_million = EXCLUDED.input_price_per_million,
                    output_price_per_million = EXCLUDED.output_price_per_million,
                    cached_input_price_per_million = EXCLUDED.cached_input_price_per_million,
                    cache_write_price_per_million = EXCLUDED.cache_write_price_per_million,
                    cache_storage_price_per_million_token_hours = EXCLUDED.cache_storage_price_per_million_token_hours,
                    supports_completion = EXCLUDED.supports_completion,
                    supports_embeddings = EXCLUDED.supports_embeddings,
                    context_window = EXCLUDED.context_window,
                    max_output_tokens = EXCLUDED.max_output_tokens,
                    updated_at = NOW()
                "#,
                provider.name,
                model.name,
                model.display_name,
                model.input_price_per_million,
                model.output_price_per_million,
                model.cached_input_price_per_million,
                model.cache_write_price_per_million,
                model.cache_storage_price_per_million_token_hours,
                model.supports_completion.unwrap_or(true),
                model.supports_embeddings.unwrap_or(false),
                model.context_window,
                model.max_output_tokens
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;

    tracing::info!(
        "Successfully loaded LLM catalog from {}",
        yaml_path.display()
    );

    Ok(())
}
