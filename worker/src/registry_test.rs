#[cfg(test)]
mod tests {
    use crate::activity_result::ActivityResult;
    use crate::registry::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;
    use streamflow_core::cache::NoOpCache;

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
}
