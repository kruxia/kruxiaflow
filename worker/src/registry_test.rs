#[cfg(test)]
mod tests {
    use crate::registry::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;

    struct TestActivity {
        name: String,
        namespace: String,
    }

    impl TestActivity {
        fn new(namespace: &str, name: &str) -> Self {
            Self {
                name: name.to_string(),
                namespace: namespace.to_string(),
            }
        }
    }

    #[async_trait]
    impl ActivityImpl for TestActivity {
        async fn execute(&self, parameters: Value) -> Result<Value> {
            // Echo the parameters back
            Ok(parameters)
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn namespace(&self) -> &str {
            &self.namespace
        }
    }

    struct SlowActivity;

    #[async_trait]
    impl ActivityImpl for SlowActivity {
        async fn execute(&self, _parameters: Value) -> Result<Value> {
            // Sleep for 2 seconds to test timeout
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(json!({"result": "done"}))
        }

        fn name(&self) -> &str {
            "slow"
        }

        fn namespace(&self) -> &str {
            "test"
        }
    }

    struct FailingActivity;

    #[async_trait]
    impl ActivityImpl for FailingActivity {
        async fn execute(&self, _parameters: Value) -> Result<Value> {
            anyhow::bail!("This activity always fails")
        }

        fn name(&self) -> &str {
            "failing"
        }

        fn namespace(&self) -> &str {
            "test"
        }
    }

    #[tokio::test]
    async fn test_register_activity() {
        let mut registry = ActivityRegistry::new();

        let activity = Arc::new(TestActivity::new("test", "echo"));
        registry.register(activity);

        let types = registry.activity_types();
        assert_eq!(types.len(), 1);
        assert!(types.contains(&"test.echo".to_string()));
    }

    #[tokio::test]
    async fn test_register_multiple_activities() {
        let mut registry = ActivityRegistry::new();

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
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(TestActivity::new("test", "echo")));

        let input = json!({"message": "hello", "count": 42});
        let result = registry
            .execute("test", "echo", input.clone(), Duration::from_secs(5))
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output, input);
    }

    #[tokio::test]
    async fn test_execute_nonexistent_activity() {
        let registry = ActivityRegistry::new();

        let result = registry
            .execute("test", "nonexistent", json!({}), Duration::from_secs(5))
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
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(SlowActivity));

        // Execute with 500ms timeout (activity takes 2 seconds)
        let result = registry
            .execute("test", "slow", json!({}), Duration::from_millis(500))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_activity_execution_with_sufficient_timeout() {
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(SlowActivity));

        // Execute with 5 second timeout (activity takes 2 seconds)
        let result = registry
            .execute("test", "slow", json!({}), Duration::from_secs(5))
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_activity_execution_failure() {
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(FailingActivity));

        let result = registry
            .execute("test", "failing", json!({}), Duration::from_secs(5))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("always fails"));
    }

    #[tokio::test]
    async fn test_activity_types() {
        let mut registry = ActivityRegistry::new();

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
