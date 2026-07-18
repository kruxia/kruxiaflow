/// Standard worker activity registration
///
/// This module provides default registration for all standard worker activities.
///
use crate::activities::{
    EchoActivity, EmailSendActivity, EmbeddingActivity, HttpRequestActivity, LLMPromptActivity,
    PostgresQueryActivity, PostgresTransactionActivity, new_pool_cache,
};
use crate::registry::ActivityRegistry;
use kruxiaflow_core::cache::CacheService;
use std::sync::Arc;

/// Register all standard worker activities
///
/// Returns an ActivityRegistry with all standard worker activities pre-registered:
/// - `std.echo` - Echo activity (for testing)
/// - `std.http_request` - HTTP request activity
/// - `std.postgres_query` - PostgreSQL query activity
/// - `std.postgres_transaction` - PostgreSQL transaction activity
/// - `std.llm_prompt` - LLM prompt completion activity
/// - `std.embedding` - LLM embedding generation activity
/// - `std.email_send` - Email send activity via SMTP
///
/// # Arguments
///
/// * `cache_service` - Cache service for activity result caching
///
/// # Example
///
/// ```rust,no_run
/// use kruxiaflow_std_worker::register_std_activities;
/// use kruxiaflow_core::cache::NoOpCache;
/// use std::sync::Arc;
///
/// let cache_service = Arc::new(NoOpCache::new());
/// let registry = register_std_activities(cache_service);
/// // Registry is ready to use with worker manager
/// ```
pub fn register_std_activities(cache_service: Arc<dyn CacheService>) -> ActivityRegistry {
    let mut registry = ActivityRegistry::new(cache_service);

    // Register echo activity (for testing)
    registry.register(Arc::new(EchoActivity));

    // Register HTTP request activity
    registry.register(Arc::new(HttpRequestActivity::new()));

    // Create shared PostgreSQL connection pool cache
    let pg_pool_cache = new_pool_cache();

    // Register PostgreSQL activities (share pool cache)
    registry.register(Arc::new(PostgresQueryActivity::new(pg_pool_cache.clone())));
    registry.register(Arc::new(PostgresTransactionActivity::new(pg_pool_cache)));

    // Register LLM activities
    let llm_activity = Arc::new(LLMPromptActivity::new());
    registry.register(llm_activity.clone());
    registry.register_streaming("std", "llm_prompt", llm_activity);
    registry.register(Arc::new(EmbeddingActivity::new()));

    // Register email activity
    registry.register(Arc::new(EmailSendActivity::new()));

    // Future standard worker activities will be registered here:
    // registry.register(Arc::new(S3OperationActivity::new()));

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use kruxiaflow_core::cache::NoOpCache;

    #[test]
    fn test_register_std_activities() {
        let cache_service = Arc::new(NoOpCache::new());
        let registry = register_std_activities(cache_service);
        let activity_types = registry.activity_types();

        // Verify all standard worker activities are registered
        assert!(activity_types.contains(&"std.echo".to_string()));
        assert!(activity_types.contains(&"std.http_request".to_string()));
        assert!(activity_types.contains(&"std.postgres_query".to_string()));
        assert!(activity_types.contains(&"std.postgres_transaction".to_string()));
        assert!(activity_types.contains(&"std.llm_prompt".to_string()));
        assert!(activity_types.contains(&"std.embedding".to_string()));
        assert!(activity_types.contains(&"std.email_send".to_string()));

        // Should have exactly 7 activities
        assert_eq!(activity_types.len(), 7);
    }

    #[test]
    fn test_llm_prompt_registered_as_streaming() {
        let cache_service = Arc::new(NoOpCache::new());
        let registry = register_std_activities(cache_service);

        // LLM prompt should be registered as a streaming activity
        let streaming = registry.get_streaming("std", "llm_prompt");
        assert!(
            streaming.is_some(),
            "LLMPromptActivity should be registered as a streaming activity"
        );
    }

    #[test]
    fn test_non_streaming_activities_not_registered_as_streaming() {
        let cache_service = Arc::new(NoOpCache::new());
        let registry = register_std_activities(cache_service);

        // Other activities should NOT be registered as streaming
        assert!(registry.get_streaming("std", "echo").is_none());
        assert!(registry.get_streaming("std", "http_request").is_none());
        assert!(registry.get_streaming("std", "embedding").is_none());
    }
}
