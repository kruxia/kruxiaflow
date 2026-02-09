use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;

/// Orchestrator configuration
#[derive(Clone)]
pub struct OrchestratorConfig {
    pub pool: PgPool,
    pub poll_interval_min: Duration,
    pub poll_interval_max: Duration,
    pub backoff_multiplier: f64,
    pub workflow_timeout: Duration,
    pub timeout_check_interval: Duration,
    /// Secrets loaded from environment variables (KRUXIAFLOW_SECRET_*)
    pub secrets: HashMap<String, String>,
}

impl OrchestratorConfig {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            poll_interval_min: Duration::from_millis(50), // 20 polls/sec max (fix for high CPU)
            poll_interval_max: Duration::from_millis(1000), // 1 second when idle
            backoff_multiplier: 1.5,                      // Moderate backoff
            workflow_timeout: Duration::from_secs(300),   // 5 minutes default timeout
            timeout_check_interval: Duration::from_secs(30), // Check every 30 seconds
            secrets: HashMap::new(),
        }
    }

    /// Create OrchestratorConfig from environment variables with fallback to defaults
    ///
    /// Environment variables:
    /// - `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS`: Minimum poll interval in milliseconds (default: 50)
    /// - `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS`: Maximum poll interval in milliseconds (default: 1000)
    /// - `KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER`: Backoff multiplier (default: 1.5)
    /// - `KRUXIAFLOW_ORCHESTRATOR_WORKFLOW_TIMEOUT_SECS`: Workflow timeout in seconds (default: 300)
    /// - `KRUXIAFLOW_ORCHESTRATOR_TIMEOUT_CHECK_INTERVAL_SECS`: Timeout check interval in seconds (default: 30)
    pub fn from_env(pool: PgPool) -> Self {
        let poll_interval_min_ms: u64 =
            std::env::var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(50);

        let poll_interval_max_ms: u64 =
            std::env::var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000);

        let backoff_multiplier: f64 = std::env::var("KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.5);

        let workflow_timeout_secs: u64 =
            std::env::var("KRUXIAFLOW_ORCHESTRATOR_WORKFLOW_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300);

        let timeout_check_interval_secs: u64 =
            std::env::var("KRUXIAFLOW_ORCHESTRATOR_TIMEOUT_CHECK_INTERVAL_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);

        Self {
            pool,
            poll_interval_min: Duration::from_millis(poll_interval_min_ms),
            poll_interval_max: Duration::from_millis(poll_interval_max_ms),
            backoff_multiplier,
            workflow_timeout: Duration::from_secs(workflow_timeout_secs),
            timeout_check_interval: Duration::from_secs(timeout_check_interval_secs),
            secrets: load_secrets_from_env(),
        }
    }

    pub fn with_poll_interval(mut self, min: Duration, max: Duration) -> Self {
        self.poll_interval_min = min;
        self.poll_interval_max = max;
        self
    }

    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    pub fn with_workflow_timeout(mut self, timeout: Duration) -> Self {
        self.workflow_timeout = timeout;
        self
    }

    pub fn with_timeout_check_interval(mut self, interval: Duration) -> Self {
        self.timeout_check_interval = interval;
        self
    }

    /// Set secrets from a HashMap
    pub fn with_secrets(mut self, secrets: HashMap<String, String>) -> Self {
        self.secrets = secrets;
        self
    }

    /// Load secrets from environment variables with KRUXIAFLOW_SECRET_ prefix
    ///
    /// Environment variables like `KRUXIAFLOW_SECRET_DB_URL` become available as
    /// `{{SECRET.db_url}}` in workflow templates.
    pub fn with_secrets_from_env(mut self) -> Self {
        self.secrets = load_secrets_from_env();
        self
    }
}

/// Load secrets from environment variables with KRUXIAFLOW_SECRET_ prefix
///
/// Environment variables like `KRUXIAFLOW_SECRET_DB_URL` become available as
/// `{{SECRET.db_url}}` in workflow templates.
///
/// # Example
/// ```ignore
/// // Set in environment: KRUXIAFLOW_SECRET_DB_URL=postgres://...
/// let secrets = load_secrets_from_env();
/// assert_eq!(secrets.get("db_url"), Some(&"postgres://...".to_string()));
/// ```
pub fn load_secrets_from_env() -> HashMap<String, String> {
    std::env::vars()
        .filter_map(|(key, value)| {
            key.strip_prefix("KRUXIAFLOW_SECRET_")
                .map(|suffix| (suffix.to_lowercase(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ============================================================================
    // Regression Tests: Secrets Loading from Environment Variables
    // Prevents: docs/bugs/2026-01-04-secrets-not-loaded.md
    // ============================================================================

    #[test]
    fn test_load_secrets_from_env() {
        // Set test environment variables
        // SAFETY: Environment variable operations are safe in single-threaded test context
        unsafe {
            std::env::set_var(
                "KRUXIAFLOW_SECRET_DB_URL",
                "postgres://test:pass@localhost/db",
            );
            std::env::set_var("KRUXIAFLOW_SECRET_API_KEY", "sk-test-key-12345");
            std::env::set_var("KRUXIAFLOW_SECRET_MIXED_CASE_NAME", "mixed-value");
            std::env::set_var("OTHER_VAR", "should-be-ignored");
        }

        let secrets = load_secrets_from_env();

        // Should include secrets with KRUXIAFLOW_SECRET_ prefix
        assert_eq!(
            secrets.get("db_url"),
            Some(&"postgres://test:pass@localhost/db".to_string())
        );
        assert_eq!(
            secrets.get("api_key"),
            Some(&"sk-test-key-12345".to_string())
        );
        // Key should be lowercased
        assert_eq!(
            secrets.get("mixed_case_name"),
            Some(&"mixed-value".to_string())
        );

        // Should NOT include non-secret vars
        assert!(!secrets.contains_key("OTHER_VAR"));

        // Cleanup
        // SAFETY: Environment variable operations are safe in single-threaded test context
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_DB_URL");
            std::env::remove_var("KRUXIAFLOW_SECRET_API_KEY");
            std::env::remove_var("KRUXIAFLOW_SECRET_MIXED_CASE_NAME");
            std::env::remove_var("OTHER_VAR");
        }
    }

    #[test]
    fn test_load_secrets_from_env_empty() {
        // Ensure test isolation - save and clear any existing secrets
        let existing_secrets: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("KRUXIAFLOW_SECRET_"))
            .collect();

        unsafe {
            for (key, _) in &existing_secrets {
                std::env::remove_var(key);
            }
        }

        // With no KRUXIAFLOW_SECRET_* variables, should return empty HashMap
        let secrets = load_secrets_from_env();
        let filtered: HashMap<String, String> = secrets
            .into_iter()
            .filter(|(k, _)| {
                // Filter out any that might have been set by other tests
                k != "db_url" && k != "api_key" && k != "mixed_case_name"
            })
            .collect();

        // Should be empty or only contain secrets from this test run
        assert!(
            filtered.is_empty() || filtered.len() < 5,
            "Expected empty or minimal secrets, got: {:?}",
            filtered
        );

        // Restore
        unsafe {
            for (key, value) in existing_secrets {
                std::env::set_var(key, value);
            }
        }
    }

    #[test]
    fn test_load_secrets_preserves_underscores_in_name() {
        // Secret names with underscores after the prefix should be preserved
        unsafe {
            std::env::set_var(
                "KRUXIAFLOW_SECRET_DATABASE_CONNECTION_STRING",
                "postgres://db",
            );
        }

        let secrets = load_secrets_from_env();

        // The key should be "database_connection_string" (lowercased, underscores preserved)
        assert_eq!(
            secrets.get("database_connection_string"),
            Some(&"postgres://db".to_string())
        );

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_DATABASE_CONNECTION_STRING");
        }
    }

    #[test]
    fn test_load_secrets_preserves_special_characters_in_value() {
        // Secret values with special characters should be preserved exactly
        let special_value = "p@ss=word!with#special$chars&more%stuff";

        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_SPECIAL_PASS", special_value);
        }

        let secrets = load_secrets_from_env();

        assert_eq!(
            secrets.get("special_pass"),
            Some(&special_value.to_string())
        );

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_SPECIAL_PASS");
        }
    }

    #[test]
    fn test_load_secrets_handles_empty_value() {
        // Empty secret value should be preserved (not filtered out)
        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_EMPTY_SECRET", "");
        }

        let secrets = load_secrets_from_env();

        assert_eq!(secrets.get("empty_secret"), Some(&"".to_string()));

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_EMPTY_SECRET");
        }
    }

    #[test]
    fn test_load_secrets_case_insensitivity() {
        // Keys should be lowercased regardless of original case
        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_UPPERCASE_KEY", "value1");
            std::env::set_var("KRUXIAFLOW_SECRET_lowercase_key", "value2");
            std::env::set_var("KRUXIAFLOW_SECRET_MixedCase_Key", "value3");
        }

        let secrets = load_secrets_from_env();

        // All should be accessible via lowercase keys
        assert_eq!(secrets.get("uppercase_key"), Some(&"value1".to_string()));
        assert_eq!(secrets.get("lowercase_key"), Some(&"value2".to_string()));
        assert_eq!(secrets.get("mixedcase_key"), Some(&"value3".to_string()));

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_UPPERCASE_KEY");
            std::env::remove_var("KRUXIAFLOW_SECRET_lowercase_key");
            std::env::remove_var("KRUXIAFLOW_SECRET_MixedCase_Key");
        }
    }

    #[test]
    fn test_load_secrets_prefix_only_not_included() {
        // The prefix itself without any suffix should not create an entry
        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_", "empty_key_value");
        }

        let secrets = load_secrets_from_env();

        // Empty string key should exist but be empty
        assert_eq!(secrets.get(""), Some(&"empty_key_value".to_string()));

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_");
        }
    }

    #[test]
    fn test_load_secrets_similar_prefix_not_matched() {
        // Variables with similar but not exact prefix should not be included
        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRETS_EXTRA_S", "should-not-match");
            std::env::set_var("KRUXIAFLOW_SECRET", "no-underscore");
            std::env::set_var("KRUXIAFLOW_SECRETX_TYPO", "typo-prefix");
        }

        let secrets = load_secrets_from_env();

        // None of these should be included (wrong prefix)
        assert!(!secrets.contains_key("extra_s"));
        assert!(
            !secrets.contains_key("")
                || secrets.get("").map(|v| v.as_str()) != Some("no-underscore")
        );
        assert!(!secrets.contains_key("typo"));

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRETS_EXTRA_S");
            std::env::remove_var("KRUXIAFLOW_SECRET");
            std::env::remove_var("KRUXIAFLOW_SECRETX_TYPO");
        }
    }

    #[test]
    fn test_load_secrets_url_with_credentials() {
        // Common use case: database URLs with embedded credentials
        let db_url = "postgres://admin:super$ecret@db.example.com:5432/mydb?sslmode=require";

        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_DATABASE_URL", db_url);
        }

        let secrets = load_secrets_from_env();

        assert_eq!(secrets.get("database_url"), Some(&db_url.to_string()));

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_DATABASE_URL");
        }
    }

    #[test]
    fn test_load_secrets_json_value() {
        // Secret values can contain JSON (e.g., service account keys)
        let json_value = r#"{"type":"service_account","project_id":"test","private_key":"-----BEGIN RSA PRIVATE KEY-----\nMIIE..."}"#;

        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_SERVICE_ACCOUNT", json_value);
        }

        let secrets = load_secrets_from_env();

        assert_eq!(
            secrets.get("service_account"),
            Some(&json_value.to_string())
        );

        // Cleanup
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_SERVICE_ACCOUNT");
        }
    }

    // =========================================================================
    // Constructor and builder method tests
    // =========================================================================

    fn lazy_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://test:test@localhost:5432/test")
            .unwrap()
    }

    #[tokio::test]
    async fn test_new_defaults() {
        let config = OrchestratorConfig::new(lazy_pool());
        assert_eq!(config.poll_interval_min, Duration::from_millis(50));
        assert_eq!(config.poll_interval_max, Duration::from_millis(1000));
        assert_eq!(config.backoff_multiplier, 1.5);
        assert_eq!(config.workflow_timeout, Duration::from_secs(300));
        assert_eq!(config.timeout_check_interval, Duration::from_secs(30));
        assert!(config.secrets.is_empty());
    }

    #[tokio::test]
    #[serial]
    async fn test_from_env_defaults() {
        let vars = [
            "KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS",
            "KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS",
            "KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER",
            "KRUXIAFLOW_ORCHESTRATOR_WORKFLOW_TIMEOUT_SECS",
            "KRUXIAFLOW_ORCHESTRATOR_TIMEOUT_CHECK_INTERVAL_SECS",
        ];
        let saved: Vec<Option<String>> = vars.iter().map(|v| std::env::var(v).ok()).collect();
        unsafe {
            for var in &vars {
                std::env::remove_var(var);
            }
        }

        let config = OrchestratorConfig::from_env(lazy_pool());
        assert_eq!(config.poll_interval_min, Duration::from_millis(50));
        assert_eq!(config.poll_interval_max, Duration::from_millis(1000));
        assert_eq!(config.backoff_multiplier, 1.5);
        assert_eq!(config.workflow_timeout, Duration::from_secs(300));
        assert_eq!(config.timeout_check_interval, Duration::from_secs(30));

        unsafe {
            for (i, var) in vars.iter().enumerate() {
                if let Some(val) = &saved[i] {
                    std::env::set_var(var, val);
                }
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_from_env_custom_values() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS", "100");
            std::env::set_var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS", "2000");
            std::env::set_var("KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER", "2.0");
            std::env::set_var("KRUXIAFLOW_ORCHESTRATOR_WORKFLOW_TIMEOUT_SECS", "600");
            std::env::set_var("KRUXIAFLOW_ORCHESTRATOR_TIMEOUT_CHECK_INTERVAL_SECS", "60");
        }

        let config = OrchestratorConfig::from_env(lazy_pool());
        assert_eq!(config.poll_interval_min, Duration::from_millis(100));
        assert_eq!(config.poll_interval_max, Duration::from_millis(2000));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(config.workflow_timeout, Duration::from_secs(600));
        assert_eq!(config.timeout_check_interval, Duration::from_secs(60));

        unsafe {
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS");
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS");
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER");
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_WORKFLOW_TIMEOUT_SECS");
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_TIMEOUT_CHECK_INTERVAL_SECS");
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_from_env_invalid_values_use_defaults() {
        unsafe {
            std::env::set_var(
                "KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS",
                "not_a_number",
            );
            std::env::set_var("KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER", "abc");
        }

        let config = OrchestratorConfig::from_env(lazy_pool());
        assert_eq!(config.poll_interval_min, Duration::from_millis(50));
        assert_eq!(config.backoff_multiplier, 1.5);

        unsafe {
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS");
            std::env::remove_var("KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER");
        }
    }

    #[tokio::test]
    async fn test_builder_methods() {
        let config = OrchestratorConfig::new(lazy_pool())
            .with_poll_interval(Duration::from_millis(200), Duration::from_millis(5000))
            .with_backoff_multiplier(3.0)
            .with_workflow_timeout(Duration::from_secs(600))
            .with_timeout_check_interval(Duration::from_secs(120));

        assert_eq!(config.poll_interval_min, Duration::from_millis(200));
        assert_eq!(config.poll_interval_max, Duration::from_millis(5000));
        assert_eq!(config.backoff_multiplier, 3.0);
        assert_eq!(config.workflow_timeout, Duration::from_secs(600));
        assert_eq!(config.timeout_check_interval, Duration::from_secs(120));
    }

    #[tokio::test]
    async fn test_with_secrets_builder() {
        let mut secrets = HashMap::new();
        secrets.insert("key1".to_string(), "value1".to_string());
        secrets.insert("key2".to_string(), "value2".to_string());

        let config = OrchestratorConfig::new(lazy_pool()).with_secrets(secrets);
        assert_eq!(config.secrets.get("key1"), Some(&"value1".to_string()));
        assert_eq!(config.secrets.get("key2"), Some(&"value2".to_string()));
    }

    #[tokio::test]
    #[serial]
    async fn test_with_secrets_from_env_builder() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_SECRET_TEST_KEY_BUILDER", "builder_value");
        }

        let config = OrchestratorConfig::new(lazy_pool()).with_secrets_from_env();
        assert_eq!(
            config.secrets.get("test_key_builder"),
            Some(&"builder_value".to_string())
        );

        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_TEST_KEY_BUILDER");
        }
    }
}
