/// Built-in activity registration
///
/// This module provides default registration for all built-in activities.
///
use crate::activities::{
    EchoActivity, EmailSendActivity, EmbeddingActivity, HttpRequestActivity, LLMPromptActivity,
    PostgresQueryActivity, PostgresTransactionActivity, new_pool_cache,
};
use crate::registry::ActivityRegistry;
use std::sync::Arc;
use kruxiaflow_core::cache::CacheService;

/// Register all built-in activities
///
/// Returns an ActivityRegistry with all built-in activities pre-registered:
/// - `builtin.echo` - Echo activity (for testing)
/// - `builtin.http_request` - HTTP request activity
/// - `builtin.postgres_query` - PostgreSQL query activity
/// - `builtin.postgres_transaction` - PostgreSQL transaction activity
/// - `builtin.llm_prompt` - LLM prompt completion activity
/// - `builtin.embedding` - LLM embedding generation activity
/// - `builtin.email_send` - Email send activity via SMTP
///
/// # Arguments
///
/// * `cache_service` - Cache service for activity result caching
///
/// # Example
///
/// ```rust,no_run
/// use kruxiaflow_worker::register_builtin_activities;
/// use kruxiaflow_core::cache::NoOpCache;
/// use std::sync::Arc;
///
/// let cache_service = Arc::new(NoOpCache::new());
/// let registry = register_builtin_activities(cache_service);
/// // Registry is ready to use with worker manager
/// ```
pub fn register_builtin_activities(cache_service: Arc<dyn CacheService>) -> ActivityRegistry {
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
    registry.register(Arc::new(LLMPromptActivity::new()));
    registry.register(Arc::new(EmbeddingActivity::new()));

    // Register email activity
    registry.register(Arc::new(EmailSendActivity::new()));

    // Future built-in activities will be registered here:
    // registry.register(Arc::new(S3OperationActivity::new()));

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use kruxiaflow_core::cache::NoOpCache;

    #[test]
    fn test_register_builtin_activities() {
        let cache_service = Arc::new(NoOpCache::new());
        let registry = register_builtin_activities(cache_service);
        let activity_types = registry.activity_types();

        // Verify all built-in activities are registered
        assert!(activity_types.contains(&"builtin.echo".to_string()));
        assert!(activity_types.contains(&"builtin.http_request".to_string()));
        assert!(activity_types.contains(&"builtin.postgres_query".to_string()));
        assert!(activity_types.contains(&"builtin.postgres_transaction".to_string()));
        assert!(activity_types.contains(&"builtin.llm_prompt".to_string()));
        assert!(activity_types.contains(&"builtin.embedding".to_string()));
        assert!(activity_types.contains(&"builtin.email_send".to_string()));

        // Should have exactly 7 activities
        assert_eq!(activity_types.len(), 7);
    }
}
