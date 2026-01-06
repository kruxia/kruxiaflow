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
            poll_interval_min: Duration::from_millis(10),
            poll_interval_max: Duration::from_millis(500), // Optimized for low latency
            backoff_multiplier: 1.3,                       // Optimized for faster convergence
            workflow_timeout: Duration::from_secs(300),    // 5 minutes default timeout
            timeout_check_interval: Duration::from_secs(30), // Check every 30 seconds
            secrets: HashMap::new(),
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
        assert!(secrets.get("OTHER_VAR").is_none());

        // Cleanup
        // SAFETY: Environment variable operations are safe in single-threaded test context
        unsafe {
            std::env::remove_var("KRUXIAFLOW_SECRET_DB_URL");
            std::env::remove_var("KRUXIAFLOW_SECRET_API_KEY");
            std::env::remove_var("KRUXIAFLOW_SECRET_MIXED_CASE_NAME");
            std::env::remove_var("OTHER_VAR");
        }
    }
}
