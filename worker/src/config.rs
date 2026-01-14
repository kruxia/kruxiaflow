use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Worker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// API server base URL
    pub api_url: String,

    /// Worker unique identifier
    pub worker_id: String,

    /// Activity types this worker can execute (worker.name format)
    pub activity_types: Vec<String>,

    /// Maximum number of activities to poll per request
    pub poll_max_activities: usize,

    /// Polling interval when no activities available
    pub poll_interval: Duration,

    /// Maximum number of concurrent in-flight activities (semaphore-based)
    pub max_concurrent_activities: usize,

    /// Number of concurrent worker tasks (DEPRECATED: use max_concurrent_activities)
    #[deprecated(since = "0.2.0", note = "Use max_concurrent_activities instead")]
    pub concurrency: usize,

    /// Activity execution timeout (default)
    pub activity_timeout: Duration,

    /// Heartbeat interval for long-running activities
    pub heartbeat_interval: Duration,

    /// OAuth client credentials for authentication
    pub client_id: String,
    pub client_secret: String,
}

impl Default for WorkerConfig {
    #[allow(deprecated)]
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080".to_string(),
            worker_id: format!("worker_{}", uuid::Uuid::now_v7()),
            activity_types: vec!["default.echo".to_string()],
            poll_max_activities: 10,
            poll_interval: Duration::from_millis(100),
            max_concurrent_activities: 16,
            concurrency: 4, // Deprecated, kept for backwards compatibility
            activity_timeout: Duration::from_secs(300),
            heartbeat_interval: Duration::from_secs(30),
            client_id: "worker_client".to_string(),
            client_secret: "".to_string(),
        }
    }
}

impl WorkerConfig {
    /// Load configuration from environment variables
    #[allow(deprecated)]
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();

        if let Ok(url) = std::env::var("KRUXIAFLOW_API_URL") {
            config.api_url = url;
        }

        if let Ok(id) = std::env::var("KRUXIAFLOW_WORKER_ID") {
            config.worker_id = id;
        }

        if let Ok(types) = std::env::var("KRUXIAFLOW_ACTIVITY_TYPES") {
            config.activity_types = types.split(',').map(|s| s.trim().to_string()).collect();
        }

        // New config: max_concurrent_activities
        if let Ok(max_activities) = std::env::var("KRUXIAFLOW_WORKER_MAX_ACTIVITIES") {
            config.max_concurrent_activities = max_activities
                .parse()
                .map_err(|_| ConfigError::InvalidMaxConcurrentActivities)?;
        }

        // New config: poll_max_activities
        if let Ok(poll_max) = std::env::var("KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES") {
            config.poll_max_activities = poll_max
                .parse()
                .map_err(|_| ConfigError::InvalidPollMaxActivities)?;
        }

        // Deprecated: KRUXIAFLOW_WORKER_CONCURRENCY
        if let Ok(concurrency) = std::env::var("KRUXIAFLOW_WORKER_CONCURRENCY") {
            tracing::warn!(
                "KRUXIAFLOW_WORKER_CONCURRENCY is deprecated, use KRUXIAFLOW_WORKER_MAX_ACTIVITIES instead"
            );
            config.concurrency = concurrency
                .parse()
                .map_err(|_| ConfigError::InvalidConcurrency)?;
            // Migration: if max_concurrent_activities wasn't explicitly set, compute from old config
            if std::env::var("KRUXIAFLOW_WORKER_MAX_ACTIVITIES").is_err() {
                config.max_concurrent_activities = config.concurrency * config.poll_max_activities;
            }
        }

        if let Ok(client_id) = std::env::var("KRUXIAFLOW_CLIENT_ID") {
            config.client_id = client_id;
        }

        if let Ok(client_secret) = std::env::var("KRUXIAFLOW_CLIENT_SECRET") {
            config.client_secret = client_secret;
        }

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration
    #[allow(deprecated)]
    fn validate(&self) -> Result<(), ConfigError> {
        if self.activity_types.is_empty() {
            return Err(ConfigError::NoActivityTypes);
        }

        for activity_type in &self.activity_types {
            if !activity_type.contains('.') {
                return Err(ConfigError::InvalidActivityType(activity_type.clone()));
            }
        }

        if self.max_concurrent_activities == 0 {
            return Err(ConfigError::InvalidMaxConcurrentActivities);
        }

        if self.poll_max_activities == 0 {
            return Err(ConfigError::InvalidPollMaxActivities);
        }

        // Keep for backwards compatibility during deprecation period
        if self.concurrency == 0 {
            return Err(ConfigError::InvalidConcurrency);
        }

        if self.client_secret.is_empty() {
            return Err(ConfigError::MissingClientSecret);
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("No activity types configured")]
    NoActivityTypes,

    #[error("Invalid activity type format: {0} (must be worker.name)")]
    InvalidActivityType(String),

    #[error("Invalid max_concurrent_activities value (must be > 0)")]
    InvalidMaxConcurrentActivities,

    #[error("Invalid poll_max_activities value (must be > 0)")]
    InvalidPollMaxActivities,

    #[error("Invalid concurrency value (must be > 0)")]
    InvalidConcurrency,

    #[error("Missing client secret (KRUXIAFLOW_CLIENT_SECRET required)")]
    MissingClientSecret,
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    // Mutex to serialize tests that modify environment variables
    // This prevents test interference when running in parallel
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Helper to clear and set environment variables for tests
    fn with_env_vars<F>(vars: Vec<(&str, &str)>, test: F)
    where
        F: FnOnce(),
    {
        // Acquire mutex to ensure only one test modifies env vars at a time
        let _lock = ENV_MUTEX.lock().unwrap();

        // Clear relevant environment variables first
        let env_vars = [
            "KRUXIAFLOW_API_URL",
            "KRUXIAFLOW_WORKER_ID",
            "KRUXIAFLOW_ACTIVITY_TYPES",
            "KRUXIAFLOW_WORKER_CONCURRENCY",
            "KRUXIAFLOW_WORKER_MAX_ACTIVITIES",
            "KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES",
            "KRUXIAFLOW_CLIENT_ID",
            "KRUXIAFLOW_CLIENT_SECRET",
        ];

        unsafe {
            for var in &env_vars {
                env::remove_var(var);
            }

            // Set the test variables
            for (key, value) in vars {
                env::set_var(key, value);
            }
        }

        // Run the test
        test();

        // Clean up
        unsafe {
            for var in &env_vars {
                env::remove_var(var);
            }
        }

        // Mutex is automatically released when _lock goes out of scope
    }

    #[test]
    fn test_default_config() {
        let config = WorkerConfig::default();

        assert_eq!(config.api_url, "http://localhost:8080");
        assert!(config.worker_id.starts_with("worker_"));
        assert_eq!(config.activity_types, vec!["default.echo"]);
        assert_eq!(config.poll_max_activities, 10);
        assert_eq!(config.poll_interval, Duration::from_millis(100));
        assert_eq!(config.max_concurrent_activities, 16);
        assert_eq!(config.concurrency, 4); // Deprecated but still set for backwards compatibility
        assert_eq!(config.activity_timeout, Duration::from_secs(300));
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.client_id, "worker_client");
        assert_eq!(config.client_secret, "");
    }

    #[test]
    fn test_from_env_with_defaults() {
        with_env_vars(vec![("KRUXIAFLOW_CLIENT_SECRET", "test_secret")], || {
            let config = WorkerConfig::from_env().unwrap();

            assert_eq!(config.api_url, "http://localhost:8080");
            assert!(config.worker_id.starts_with("worker_"));
            assert_eq!(config.activity_types, vec!["default.echo"]);
            assert_eq!(config.max_concurrent_activities, 16);
            assert_eq!(config.concurrency, 4);
            assert_eq!(config.client_id, "worker_client");
            assert_eq!(config.client_secret, "test_secret");
        });
    }

    #[test]
    fn test_from_env_with_new_config_values() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_API_URL", "http://api.example.com:9090"),
                ("KRUXIAFLOW_WORKER_ID", "custom_worker_123"),
                ("KRUXIAFLOW_ACTIVITY_TYPES", "ns1.activity1, ns2.activity2"),
                ("KRUXIAFLOW_WORKER_MAX_ACTIVITIES", "32"),
                ("KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES", "8"),
                ("KRUXIAFLOW_CLIENT_ID", "custom_client"),
                ("KRUXIAFLOW_CLIENT_SECRET", "super_secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();

                assert_eq!(config.api_url, "http://api.example.com:9090");
                assert_eq!(config.worker_id, "custom_worker_123");
                assert_eq!(
                    config.activity_types,
                    vec!["ns1.activity1", "ns2.activity2"]
                );
                assert_eq!(config.max_concurrent_activities, 32);
                assert_eq!(config.poll_max_activities, 8);
                assert_eq!(config.client_id, "custom_client");
                assert_eq!(config.client_secret, "super_secret");
            },
        );
    }

    #[test]
    fn test_from_env_deprecated_concurrency_migration() {
        // When using deprecated KRUXIAFLOW_WORKER_CONCURRENCY without MAX_ACTIVITIES,
        // max_concurrent_activities should be computed as concurrency * poll_max_activities
        with_env_vars(
            vec![
                ("KRUXIAFLOW_WORKER_CONCURRENCY", "8"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();

                assert_eq!(config.concurrency, 8);
                // max_concurrent_activities = concurrency * poll_max_activities = 8 * 10 = 80
                assert_eq!(config.max_concurrent_activities, 80);
            },
        );
    }

    #[test]
    fn test_from_env_max_activities_overrides_migration() {
        // When both are set, MAX_ACTIVITIES takes precedence
        with_env_vars(
            vec![
                ("KRUXIAFLOW_WORKER_CONCURRENCY", "8"),
                ("KRUXIAFLOW_WORKER_MAX_ACTIVITIES", "24"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();

                assert_eq!(config.concurrency, 8);
                assert_eq!(config.max_concurrent_activities, 24); // Not computed from concurrency
            },
        );
    }

    #[test]
    fn test_from_env_invalid_max_concurrent_activities() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_WORKER_MAX_ACTIVITIES", "not_a_number"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let result = WorkerConfig::from_env();
                assert!(result.is_err());
                assert!(matches!(
                    result,
                    Err(ConfigError::InvalidMaxConcurrentActivities)
                ));
            },
        );
    }

    #[test]
    fn test_from_env_invalid_poll_max_activities() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES", "not_a_number"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let result = WorkerConfig::from_env();
                assert!(result.is_err());
                assert!(matches!(result, Err(ConfigError::InvalidPollMaxActivities)));
            },
        );
    }

    #[test]
    fn test_from_env_with_custom_values() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_API_URL", "http://api.example.com:9090"),
                ("KRUXIAFLOW_WORKER_ID", "custom_worker_123"),
                ("KRUXIAFLOW_ACTIVITY_TYPES", "ns1.activity1, ns2.activity2"),
                ("KRUXIAFLOW_WORKER_CONCURRENCY", "8"),
                ("KRUXIAFLOW_CLIENT_ID", "custom_client"),
                ("KRUXIAFLOW_CLIENT_SECRET", "super_secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();

                assert_eq!(config.api_url, "http://api.example.com:9090");
                assert_eq!(config.worker_id, "custom_worker_123");
                assert_eq!(
                    config.activity_types,
                    vec!["ns1.activity1", "ns2.activity2"]
                );
                assert_eq!(config.concurrency, 8);
                assert_eq!(config.client_id, "custom_client");
                assert_eq!(config.client_secret, "super_secret");
            },
        );
    }

    #[test]
    fn test_from_env_invalid_concurrency() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_WORKER_CONCURRENCY", "not_a_number"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let result = WorkerConfig::from_env();
                assert!(result.is_err());
                assert!(matches!(result, Err(ConfigError::InvalidConcurrency)));
            },
        );
    }

    #[test]
    fn test_from_env_missing_client_secret() {
        with_env_vars(vec![], || {
            let result = WorkerConfig::from_env();
            assert!(result.is_err());
            assert!(matches!(result, Err(ConfigError::MissingClientSecret)));
        });
    }

    #[test]
    fn test_validate_no_activity_types() {
        let mut config = WorkerConfig::default();
        config.activity_types = vec![];
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::NoActivityTypes)));
    }

    #[test]
    fn test_validate_invalid_activity_type_format() {
        let mut config = WorkerConfig::default();
        config.activity_types = vec!["invalid_format".to_string()];
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_err());
        match result {
            Err(ConfigError::InvalidActivityType(msg)) => {
                assert_eq!(msg, "invalid_format");
            }
            _ => panic!("Expected InvalidActivityType error"),
        }
    }

    #[test]
    fn test_validate_multiple_activity_types_one_invalid() {
        let mut config = WorkerConfig::default();
        config.activity_types = vec![
            "valid.activity".to_string(),
            "invalid".to_string(),
            "also.valid".to_string(),
        ];
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_err());
        match result {
            Err(ConfigError::InvalidActivityType(msg)) => {
                assert_eq!(msg, "invalid");
            }
            _ => panic!("Expected InvalidActivityType error"),
        }
    }

    #[test]
    fn test_validate_zero_max_concurrent_activities() {
        let mut config = WorkerConfig::default();
        config.max_concurrent_activities = 0;
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(ConfigError::InvalidMaxConcurrentActivities)
        ));
    }

    #[test]
    fn test_validate_zero_poll_max_activities() {
        let mut config = WorkerConfig::default();
        config.poll_max_activities = 0;
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::InvalidPollMaxActivities)));
    }

    #[test]
    fn test_validate_zero_concurrency() {
        let mut config = WorkerConfig::default();
        config.concurrency = 0;
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::InvalidConcurrency)));
    }

    #[test]
    fn test_validate_missing_client_secret() {
        let mut config = WorkerConfig::default();
        config.client_secret = "".to_string();

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result, Err(ConfigError::MissingClientSecret)));
    }

    #[test]
    fn test_validate_valid_config() {
        let mut config = WorkerConfig::default();
        config.client_secret = "secret".to_string();

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_display_messages() {
        let err = ConfigError::NoActivityTypes;
        assert_eq!(err.to_string(), "No activity types configured");

        let err = ConfigError::InvalidActivityType("test.bad".to_string());
        assert_eq!(
            err.to_string(),
            "Invalid activity type format: test.bad (must be worker.name)"
        );

        let err = ConfigError::InvalidMaxConcurrentActivities;
        assert_eq!(
            err.to_string(),
            "Invalid max_concurrent_activities value (must be > 0)"
        );

        let err = ConfigError::InvalidPollMaxActivities;
        assert_eq!(
            err.to_string(),
            "Invalid poll_max_activities value (must be > 0)"
        );

        let err = ConfigError::InvalidConcurrency;
        assert_eq!(err.to_string(), "Invalid concurrency value (must be > 0)");

        let err = ConfigError::MissingClientSecret;
        assert_eq!(
            err.to_string(),
            "Missing client secret (KRUXIAFLOW_CLIENT_SECRET required)"
        );
    }

    #[test]
    fn test_from_env_with_single_activity_type() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_ACTIVITY_TYPES", "worker.single"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();
                assert_eq!(config.activity_types, vec!["worker.single"]);
            },
        );
    }

    #[test]
    fn test_from_env_activity_types_with_spaces() {
        with_env_vars(
            vec![
                (
                    "KRUXIAFLOW_ACTIVITY_TYPES",
                    "ns1.act1 , ns2.act2  ,  ns3.act3",
                ),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();
                assert_eq!(
                    config.activity_types,
                    vec!["ns1.act1", "ns2.act2", "ns3.act3"]
                );
            },
        );
    }
}
