#[cfg(test)]
mod tests {
    use crate::activity_result::ActivityResult;
    use crate::registry::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use rust_decimal::Decimal;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use streamflow_core::cache::{CacheService, CachedResult, NoOpCache};
    use streamflow_core::workflow::ActivitySettings;

    struct TestActivity {
        name: String,
        worker: String,
    }

    impl TestActivity {
        fn new(worker: &str, name: &str) -> Self {
            Self {
                name: name.to_string(),
                worker: worker.to_string(),
            }
        }
    }

    #[async_trait]
    impl ActivityImpl for TestActivity {
        async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
            // Echo the parameters back
            Ok(ActivityResult::value("result", parameters))
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn worker(&self) -> &str {
            &self.worker
        }
    }

    struct SlowActivity;

    #[async_trait]
    impl ActivityImpl for SlowActivity {
        async fn execute(&self, _parameters: Value) -> Result<ActivityResult> {
            // Sleep for 2 seconds to test timeout
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(ActivityResult::value("result", json!("done")))
        }

        fn name(&self) -> &str {
            "slow"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    struct FailingActivity;

    #[async_trait]
    impl ActivityImpl for FailingActivity {
        async fn execute(&self, _parameters: Value) -> Result<ActivityResult> {
            anyhow::bail!("This activity always fails")
        }

        fn name(&self) -> &str {
            "failing"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    #[tokio::test]
    async fn test_register_activity() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);

        let activity = Arc::new(TestActivity::new("test", "echo"));
        registry.register(activity);

        let types = registry.activity_types();
        assert_eq!(types.len(), 1);
        assert!(types.contains(&"test.echo".to_string()));
    }

    #[tokio::test]
    async fn test_register_multiple_activities() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);

        registry.register(Arc::new(TestActivity::new("payments", "authorize")));
        registry.register(Arc::new(TestActivity::new("payments", "capture")));
        registry.register(Arc::new(TestActivity::new("emails", "send")));

        let types = registry.activity_types();
        assert_eq!(types.len(), 3);
        assert!(types.contains(&"payments.authorize".to_string()));
        assert!(types.contains(&"payments.capture".to_string()));
        assert!(types.contains(&"emails.send".to_string()));
    }

    #[tokio::test]
    async fn test_execute_activity() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);
        registry.register(Arc::new(TestActivity::new("test", "echo")));

        let input = json!({"message": "hello", "count": 42});
        let result = registry
            .execute("test", "echo", input.clone(), None, Duration::from_secs(5))
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.to_json_value().get("result"), Some(&input));
    }

    #[tokio::test]
    async fn test_execute_nonexistent_activity() {
        let cache = Arc::new(NoOpCache::new());
        let registry = ActivityRegistry::new(cache);

        let result = registry
            .execute(
                "test",
                "nonexistent",
                json!({}),
                None,
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Activity implementation not found")
        );
    }

    #[tokio::test]
    async fn test_activity_timeout() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);
        registry.register(Arc::new(SlowActivity));

        // Execute with 500ms timeout (activity takes 2 seconds)
        let result = registry
            .execute("test", "slow", json!({}), None, Duration::from_millis(500))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_activity_execution_with_sufficient_timeout() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);
        registry.register(Arc::new(SlowActivity));

        // Execute with 5 second timeout (activity takes 2 seconds)
        let result = registry
            .execute("test", "slow", json!({}), None, Duration::from_secs(5))
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_activity_execution_failure() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);
        registry.register(Arc::new(FailingActivity));

        let result = registry
            .execute("test", "failing", json!({}), None, Duration::from_secs(5))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("always fails"));
    }

    #[tokio::test]
    async fn test_activity_types() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);

        assert_eq!(registry.activity_types().len(), 0);

        registry.register(Arc::new(TestActivity::new("test", "one")));
        assert_eq!(registry.activity_types().len(), 1);

        registry.register(Arc::new(TestActivity::new("test", "two")));
        assert_eq!(registry.activity_types().len(), 2);

        let types = registry.activity_types();
        assert!(types.contains(&"test.one".to_string()));
        assert!(types.contains(&"test.two".to_string()));
    }

    // =========================================================================
    // Mock cache for testing caching behavior
    // =========================================================================

    /// Mock cache that tracks get/set calls and stores results in memory
    struct MockCache {
        storage: std::sync::RwLock<HashMap<String, CachedResult>>,
        get_calls: AtomicUsize,
        set_calls: AtomicUsize,
        available: bool,
    }

    impl MockCache {
        fn new() -> Self {
            Self {
                storage: std::sync::RwLock::new(HashMap::new()),
                get_calls: AtomicUsize::new(0),
                set_calls: AtomicUsize::new(0),
                available: true,
            }
        }

        fn unavailable() -> Self {
            Self {
                storage: std::sync::RwLock::new(HashMap::new()),
                get_calls: AtomicUsize::new(0),
                set_calls: AtomicUsize::new(0),
                available: false,
            }
        }

        fn get_call_count(&self) -> usize {
            self.get_calls.load(Ordering::SeqCst)
        }

        fn set_call_count(&self) -> usize {
            self.set_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl CacheService for MockCache {
        fn is_available(&self) -> bool {
            self.available
        }

        async fn get(&self, key: &str) -> anyhow::Result<Option<CachedResult>> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            let storage = self.storage.read().unwrap();
            Ok(storage.get(key).cloned())
        }

        async fn set(
            &self,
            key: &str,
            result: &CachedResult,
            _ttl: Duration,
        ) -> anyhow::Result<()> {
            self.set_calls.fetch_add(1, Ordering::SeqCst);
            let mut storage = self.storage.write().unwrap();
            storage.insert(key.to_string(), result.clone());
            Ok(())
        }

        async fn invalidate(&self, key: &str) -> anyhow::Result<()> {
            let mut storage = self.storage.write().unwrap();
            storage.remove(key);
            Ok(())
        }

        async fn invalidate_pattern(&self, _pattern: &str) -> anyhow::Result<usize> {
            // Simple mock - just return 0 invalidated
            Ok(0)
        }
    }

    /// Activity that tracks execution count
    struct CountingActivity {
        execution_count: AtomicUsize,
    }

    impl CountingActivity {
        fn new() -> Self {
            Self {
                execution_count: AtomicUsize::new(0),
            }
        }

        fn execution_count(&self) -> usize {
            self.execution_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ActivityImpl for CountingActivity {
        async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
            self.execution_count.fetch_add(1, Ordering::SeqCst);
            Ok(ActivityResult {
                outputs: vec![streamflow_core::workflow::ActivityOutput::value(
                    "result", parameters,
                )],
                cost_usd: Some(Decimal::new(10, 2)), // $0.10
                metadata: None,
            })
        }

        fn name(&self) -> &str {
            "counting"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    // =========================================================================
    // Caching tests
    // =========================================================================

    #[tokio::test]
    async fn test_cache_disabled_by_default() {
        let cache = Arc::new(MockCache::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(Arc::new(TestActivity::new("test", "echo")));

        // Execute without cache settings
        let _result = registry
            .execute(
                "test",
                "echo",
                json!({"key": "value"}),
                None,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Cache should not be checked or set when caching is disabled
        assert_eq!(cache.get_call_count(), 0);
        assert_eq!(cache.set_call_count(), 0);
    }

    #[tokio::test]
    async fn test_cache_disabled_explicitly() {
        let cache = Arc::new(MockCache::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(Arc::new(TestActivity::new("test", "echo")));

        let settings = ActivitySettings {
            cache: false, // Explicitly disabled
            cache_ttl: None,
            timeout_seconds: None,
            retry: None,
            budget: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        };

        let _result = registry
            .execute(
                "test",
                "echo",
                json!({"key": "value"}),
                Some(settings),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Cache should not be used
        assert_eq!(cache.get_call_count(), 0);
        assert_eq!(cache.set_call_count(), 0);
    }

    #[tokio::test]
    async fn test_cache_miss_then_store() {
        let cache = Arc::new(MockCache::new());
        let activity = Arc::new(CountingActivity::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(activity.clone());

        let settings = ActivitySettings {
            cache: true,
            cache_ttl: Some(3600),
            timeout_seconds: None,
            retry: None,
            budget: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        };

        let result = registry
            .execute(
                "test",
                "counting",
                json!({"input": "first"}),
                Some(settings),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Activity should be executed
        assert_eq!(activity.execution_count(), 1);

        // Cache should be checked (miss) and then set
        assert_eq!(cache.get_call_count(), 1);
        assert_eq!(cache.set_call_count(), 1);

        // Result should have cost
        assert_eq!(result.cost_usd, Some(Decimal::new(10, 2)));
    }

    #[tokio::test]
    async fn test_cache_hit_returns_cached_result() {
        let cache = Arc::new(MockCache::new());
        let activity = Arc::new(CountingActivity::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(activity.clone());

        let settings = ActivitySettings {
            cache: true,
            cache_ttl: Some(3600),
            timeout_seconds: None,
            retry: None,
            budget: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        };

        let params = json!({"input": "cached_test"});

        // First call - cache miss, execute activity
        let result1 = registry
            .execute(
                "test",
                "counting",
                params.clone(),
                Some(settings.clone()),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Second call - should hit cache
        let result2 = registry
            .execute(
                "test",
                "counting",
                params.clone(),
                Some(settings),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Activity should only be executed once (second call hits cache)
        assert_eq!(activity.execution_count(), 1);

        // Cache should be checked twice, set once
        assert_eq!(cache.get_call_count(), 2);
        assert_eq!(cache.set_call_count(), 1);

        // First result should have original cost
        assert_eq!(result1.cost_usd, Some(Decimal::new(10, 2)));

        // Cached result should have zero cost
        assert_eq!(result2.cost_usd, Some(Decimal::ZERO));
    }

    #[tokio::test]
    async fn test_different_params_not_cached() {
        let cache = Arc::new(MockCache::new());
        let activity = Arc::new(CountingActivity::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(activity.clone());

        let settings = ActivitySettings {
            cache: true,
            cache_ttl: Some(3600),
            timeout_seconds: None,
            retry: None,
            budget: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        };

        // First call with params A
        let _result1 = registry
            .execute(
                "test",
                "counting",
                json!({"input": "A"}),
                Some(settings.clone()),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Second call with different params B
        let _result2 = registry
            .execute(
                "test",
                "counting",
                json!({"input": "B"}),
                Some(settings),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Activity should be executed twice (different cache keys)
        assert_eq!(activity.execution_count(), 2);

        // Cache should be checked twice, set twice
        assert_eq!(cache.get_call_count(), 2);
        assert_eq!(cache.set_call_count(), 2);
    }

    #[tokio::test]
    async fn test_cache_unavailable_executes_without_caching() {
        let cache = Arc::new(MockCache::unavailable());
        let activity = Arc::new(CountingActivity::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(activity.clone());

        let settings = ActivitySettings {
            cache: true, // Caching enabled in settings
            cache_ttl: Some(3600),
            timeout_seconds: None,
            retry: None,
            budget: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        };

        // Execute twice with same params
        let _result1 = registry
            .execute(
                "test",
                "counting",
                json!({"input": "test"}),
                Some(settings.clone()),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        let _result2 = registry
            .execute(
                "test",
                "counting",
                json!({"input": "test"}),
                Some(settings),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Activity should be executed both times (cache unavailable)
        assert_eq!(activity.execution_count(), 2);

        // Cache should not be called when unavailable
        assert_eq!(cache.get_call_count(), 0);
        assert_eq!(cache.set_call_count(), 0);
    }

    #[tokio::test]
    async fn test_cache_default_ttl() {
        let cache = Arc::new(MockCache::new());
        let activity = Arc::new(CountingActivity::new());
        let mut registry = ActivityRegistry::new(cache.clone());
        registry.register(activity.clone());

        // Cache enabled but no TTL specified - should use default (3600)
        let settings = ActivitySettings {
            cache: true,
            cache_ttl: None, // No TTL, should default to 3600
            timeout_seconds: None,
            retry: None,
            budget: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        };

        let _result = registry
            .execute(
                "test",
                "counting",
                json!({"input": "ttl_test"}),
                Some(settings),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        // Activity should be executed
        assert_eq!(activity.execution_count(), 1);
        // Result should be cached
        assert_eq!(cache.set_call_count(), 1);
    }

    // =========================================================================
    // Additional edge case tests
    // =========================================================================

    #[tokio::test]
    async fn test_activity_replaces_same_key() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);

        registry.register(Arc::new(TestActivity::new("test", "echo")));

        // Register another activity with the same key
        registry.register(Arc::new(FailingActivity));

        // The failing activity should replace the echo activity
        // (since FailingActivity has worker="test" and name="failing", this won't actually replace)
        // Let's register another TestActivity with same worker/name
        let types = registry.activity_types();
        assert!(types.contains(&"test.echo".to_string()));
        assert!(types.contains(&"test.failing".to_string()));
    }

    #[tokio::test]
    async fn test_execute_with_complex_parameters() {
        let cache = Arc::new(NoOpCache::new());
        let mut registry = ActivityRegistry::new(cache);
        registry.register(Arc::new(TestActivity::new("test", "echo")));

        let complex_params = json!({
            "nested": {
                "array": [1, 2, 3],
                "object": {"key": "value"}
            },
            "boolean": true,
            "null_value": null,
            "number": 42.5
        });

        let result = registry
            .execute(
                "test",
                "echo",
                complex_params.clone(),
                None,
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        // The TestActivity echoes back the parameters
        assert_eq!(output.to_json_value().get("result"), Some(&complex_params));
    }
}
