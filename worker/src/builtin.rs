/// Built-in activity registration
///
/// This module provides default registration for all built-in activities.
///
use crate::activities::{EchoActivity, HttpRequestActivity, PostgresQueryActivity};
use crate::registry::ActivityRegistry;
use std::sync::Arc;

/// Register all built-in activities
///
/// Returns an ActivityRegistry with all built-in activities pre-registered:
/// - `builtin.echo` - Echo activity (for testing)
/// - `builtin.http_request` - HTTP request activity
/// - `builtin.postgres_query` - PostgreSQL query activity
///
/// # Example
///
/// ```rust,no_run
/// use streamflow_worker::register_builtin_activities;
///
/// let registry = register_builtin_activities();
/// // Registry is ready to use with worker manager
/// ```
pub fn register_builtin_activities() -> ActivityRegistry {
    let mut registry = ActivityRegistry::new();

    // Register echo activity (for testing)
    registry.register(Arc::new(EchoActivity));

    // Register HTTP request activity
    registry.register(Arc::new(HttpRequestActivity::new()));

    // Register PostgreSQL query activity
    registry.register(Arc::new(PostgresQueryActivity::new()));

    // Future built-in activities will be registered here:
    // registry.register(Arc::new(LlmPromptActivity::new()));
    // registry.register(Arc::new(S3OperationActivity::new()));

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_builtin_activities() {
        let registry = register_builtin_activities();
        let activity_types = registry.activity_types();

        // Verify all built-in activities are registered
        assert!(activity_types.contains(&"builtin.echo".to_string()));
        assert!(activity_types.contains(&"builtin.http_request".to_string()));
        assert!(activity_types.contains(&"builtin.postgres_query".to_string()));

        // Should have exactly 3 activities
        assert_eq!(activity_types.len(), 3);
    }
}
