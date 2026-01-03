use crate::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ProviderResponse {
    pub name: String,
    pub display_name: String,
    pub api_endpoint: Option<String>,
    pub supports_completion: bool,
    pub supports_embeddings: bool,
    pub supports_streaming: bool,
    pub requires_api_key: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModelResponse {
    pub id: uuid::Uuid,
    pub provider: String,
    pub name: String,
    pub display_name: String,
    pub input_price_per_million: Decimal,
    pub output_price_per_million: Decimal,
    pub cached_input_price_per_million: Option<Decimal>,
    pub supports_completion: bool,
    pub supports_embeddings: bool,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
}

/// GET /api/v1/llm/providers
/// List all LLM providers
///
/// Returns all available LLM providers with their capabilities
/// (completion, embeddings, streaming support).
///
/// # Response
/// - 200 OK: List of providers (may be empty if catalog not seeded)
/// - 500 Internal Server Error: Database query failed
///
/// # Performance
/// Target: <10ms P99 latency
#[utoipa::path(
    get,
    path = "/api/v1/llm/providers",
    tag = "LLM Catalog",
    responses(
        (status = 200, description = "List of LLM providers", body = Vec<ProviderResponse>),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderResponse>>, StatusCode> {
    let providers = sqlx::query_as!(
        ProviderResponse,
        r#"
        SELECT name, display_name, api_endpoint,
               supports_completion, supports_embeddings, supports_streaming,
               requires_api_key
        FROM llm_providers
        ORDER BY name
        "#
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch providers: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(providers))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ModelSearchCriterion {
    /// Provider name (e.g., "anthropic", "openai", "google", "ollama")
    pub provider: Option<String>,
    /// Model name (e.g., "gpt-4o", "claude-3-5-sonnet-20241022")
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ModelSearchRequest {
    /// List of search criteria (supports batch lookup)
    pub models: Vec<ModelSearchCriterion>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModelSearchResponse {
    /// Matched models with pricing information
    pub models: Vec<ModelResponse>,
}

/// POST /api/v1/llm/models/search
/// Search for models by provider/model name
///
/// Supports batch lookup with flexible filtering. Each search criterion
/// can specify provider, model, or both. Returns all models matching
/// any of the criteria (OR logic).
///
/// # Examples
/// - `{"models": [{"provider": "anthropic"}]}` - All Anthropic models
/// - `{"models": [{"model": "gpt-4o"}]}` - GPT-4o from any provider
/// - `{"models": [{"provider": "openai", "model": "gpt-4o"}]}` - Specific model
/// - `{"models": []}` - Returns empty array
///
/// # Response
/// - 200 OK: Matching models (may be empty array)
/// - 500 Internal Server Error: Database query failed
///
/// # Performance
/// Target: <20ms P99 latency for batch queries with <10 criteria
#[utoipa::path(
    post,
    path = "/api/v1/llm/models/search",
    tag = "LLM Catalog",
    request_body = ModelSearchRequest,
    responses(
        (status = 200, description = "Search results", body = ModelSearchResponse),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn search_models(
    State(state): State<AppState>,
    Json(request): Json<ModelSearchRequest>,
) -> Result<Json<ModelSearchResponse>, StatusCode> {
    if request.models.is_empty() {
        return Ok(Json(ModelSearchResponse { models: vec![] }));
    }

    // Build arrays for ANY query
    let mut providers: Vec<Option<String>> = Vec::new();
    let mut models: Vec<Option<String>> = Vec::new();

    for criterion in &request.models {
        providers.push(criterion.provider.clone());
        models.push(criterion.model.clone());
    }

    // Use array comparison to match any of the models in a single query
    // This works by creating parallel arrays where index N represents criterion N
    let results = sqlx::query_as!(
        ModelResponse,
        r#"
        WITH search_criteria AS (
            SELECT
                UNNEST($1::text[]) as provider_filter,
                UNNEST($2::text[]) as model_filter
        )
        SELECT DISTINCT
            id, provider, name, display_name,
            input_price_per_million, output_price_per_million,
            cached_input_price_per_million,
            supports_completion, supports_embeddings,
            context_window, max_output_tokens
        FROM llm_models
        WHERE EXISTS (
            SELECT 1 FROM search_criteria c
            WHERE (c.provider_filter IS NULL OR provider = c.provider_filter)
              AND (c.model_filter IS NULL OR name = c.model_filter)
        )
        ORDER BY provider, name
        "#,
        &providers as &[Option<String>],
        &models as &[Option<String>]
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to search models: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ModelSearchResponse { models: results }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    async fn setup_test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });

        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    async fn setup_test_state() -> AppState {
        use kruxiaflow_core::cache::NoOpCache;

        let pool = setup_test_pool().await;
        let auth_service = Arc::new(crate::state::tests::MockAuthService);
        let activity_queue = Arc::new(crate::state::tests::MockActivityQueue);
        let event_source = Arc::new(crate::state::tests::MockEventSource);
        let workflow_storage = Arc::new(crate::state::tests::MockWorkflowStorage);
        let cache_service = Arc::new(NoOpCache::new());
        let shutdown_token = CancellationToken::new();

        AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        )
    }

    #[tokio::test]
    async fn test_list_providers() {
        let state = setup_test_state().await;

        // Should not panic even if table is empty
        let result = list_providers(State(state)).await;

        assert!(result.is_ok(), "list_providers should not fail");
    }

    #[tokio::test]
    async fn test_search_models_empty() {
        let state = setup_test_state().await;

        let request = ModelSearchRequest { models: vec![] };

        let result = search_models(State(state), Json(request)).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().0.models.len(), 0);
    }

    #[tokio::test]
    async fn test_search_models_by_provider() {
        let state = setup_test_state().await;

        let request = ModelSearchRequest {
            models: vec![ModelSearchCriterion {
                provider: Some("anthropic".to_string()),
                model: None,
            }],
        };

        let result = search_models(State(state), Json(request)).await;

        assert!(result.is_ok(), "search_models should not fail");
    }

    #[tokio::test]
    async fn test_search_models_by_model_name() {
        let state = setup_test_state().await;

        let request = ModelSearchRequest {
            models: vec![ModelSearchCriterion {
                provider: None,
                model: Some("gpt-4".to_string()),
            }],
        };

        let result = search_models(State(state), Json(request)).await;

        assert!(result.is_ok(), "search_models should not fail");
    }

    #[tokio::test]
    async fn test_search_models_batch() {
        let state = setup_test_state().await;

        let request = ModelSearchRequest {
            models: vec![
                ModelSearchCriterion {
                    provider: Some("anthropic".to_string()),
                    model: Some("claude-3-5-sonnet-20241022".to_string()),
                },
                ModelSearchCriterion {
                    provider: Some("openai".to_string()),
                    model: Some("gpt-4o".to_string()),
                },
            ],
        };

        let result = search_models(State(state), Json(request)).await;

        assert!(result.is_ok(), "Batch search should not fail");
    }
}
