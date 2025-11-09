use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Worker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// API server base URL
    pub api_url: String,

    /// Worker unique identifier
    pub worker_id: String,

    /// Activity types this worker can execute (namespace.name format)
    pub activity_types: Vec<String>,

    /// Maximum number of activities to poll per request
    pub poll_max_activities: usize,

    /// Polling interval when no activities available
    pub poll_interval: Duration,

    /// Number of concurrent worker tasks
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
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080".to_string(),
            worker_id: format!("worker_{}", uuid::Uuid::now_v7()),
            activity_types: vec!["default.echo".to_string()],
            poll_max_activities: 10,
            poll_interval: Duration::from_millis(100),
            concurrency: 4,
            activity_timeout: Duration::from_secs(300),
            heartbeat_interval: Duration::from_secs(30),
            client_id: "worker_client".to_string(),
            client_secret: "".to_string(),
        }
    }
}

impl WorkerConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();

        if let Ok(url) = std::env::var("STREAMFLOW_API_URL") {
            config.api_url = url;
        }

        if let Ok(id) = std::env::var("STREAMFLOW_WORKER_ID") {
            config.worker_id = id;
        }

        if let Ok(types) = std::env::var("STREAMFLOW_ACTIVITY_TYPES") {
            config.activity_types = types.split(',').map(|s| s.trim().to_string()).collect();
        }

        if let Ok(concurrency) = std::env::var("STREAMFLOW_WORKER_CONCURRENCY") {
            config.concurrency = concurrency
                .parse()
                .map_err(|_| ConfigError::InvalidConcurrency)?;
        }

        if let Ok(client_id) = std::env::var("STREAMFLOW_CLIENT_ID") {
            config.client_id = client_id;
        }

        if let Ok(client_secret) = std::env::var("STREAMFLOW_CLIENT_SECRET") {
            config.client_secret = client_secret;
        }

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration
    fn validate(&self) -> Result<(), ConfigError> {
        if self.activity_types.is_empty() {
            return Err(ConfigError::NoActivityTypes);
        }

        for activity_type in &self.activity_types {
            if !activity_type.contains('.') {
                return Err(ConfigError::InvalidActivityType(activity_type.clone()));
            }
        }

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

    #[error("Invalid activity type format: {0} (must be namespace.name)")]
    InvalidActivityType(String),

    #[error("Invalid concurrency value (must be > 0)")]
    InvalidConcurrency,

    #[error("Missing client secret (STREAMFLOW_CLIENT_SECRET required)")]
    MissingClientSecret,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Helper to clear and set environment variables for tests
    fn with_env_vars<F>(vars: Vec<(&str, &str)>, test: F)
    where
        F: FnOnce(),
    {
        // Clear relevant environment variables first
        let env_vars = [
            "STREAMFLOW_API_URL",
            "STREAMFLOW_WORKER_ID",
            "STREAMFLOW_ACTIVITY_TYPES",
            "STREAMFLOW_WORKER_CONCURRENCY",
            "STREAMFLOW_CLIENT_ID",
            "STREAMFLOW_CLIENT_SECRET",
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
    }

    #[test]
    fn test_default_config() {
        let config = WorkerConfig::default();

        assert_eq!(config.api_url, "http://localhost:8080");
        assert!(config.worker_id.starts_with("worker_"));
        assert_eq!(config.activity_types, vec!["default.echo"]);
        assert_eq!(config.poll_max_activities, 10);
        assert_eq!(config.poll_interval, Duration::from_millis(100));
        assert_eq!(config.concurrency, 4);
        assert_eq!(config.activity_timeout, Duration::from_secs(300));
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.client_id, "worker_client");
        assert_eq!(config.client_secret, "");
    }

    #[test]
    fn test_from_env_with_defaults() {
        with_env_vars(vec![("STREAMFLOW_CLIENT_SECRET", "test_secret")], || {
            let config = WorkerConfig::from_env().unwrap();

            assert_eq!(config.api_url, "http://localhost:8080");
            assert!(config.worker_id.starts_with("worker_"));
            assert_eq!(config.activity_types, vec!["default.echo"]);
            assert_eq!(config.concurrency, 4);
            assert_eq!(config.client_id, "worker_client");
            assert_eq!(config.client_secret, "test_secret");
        });
    }

    #[test]
    fn test_from_env_with_custom_values() {
        with_env_vars(
            vec![
                ("STREAMFLOW_API_URL", "http://api.example.com:9090"),
                ("STREAMFLOW_WORKER_ID", "custom_worker_123"),
                ("STREAMFLOW_ACTIVITY_TYPES", "ns1.activity1, ns2.activity2"),
                ("STREAMFLOW_WORKER_CONCURRENCY", "8"),
                ("STREAMFLOW_CLIENT_ID", "custom_client"),
                ("STREAMFLOW_CLIENT_SECRET", "super_secret"),
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
                ("STREAMFLOW_WORKER_CONCURRENCY", "not_a_number"),
                ("STREAMFLOW_CLIENT_SECRET", "secret"),
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
            "Invalid activity type format: test.bad (must be namespace.name)"
        );

        let err = ConfigError::InvalidConcurrency;
        assert_eq!(err.to_string(), "Invalid concurrency value (must be > 0)");

        let err = ConfigError::MissingClientSecret;
        assert_eq!(
            err.to_string(),
            "Missing client secret (STREAMFLOW_CLIENT_SECRET required)"
        );
    }

    #[test]
    fn test_from_env_with_single_activity_type() {
        with_env_vars(
            vec![
                ("STREAMFLOW_ACTIVITY_TYPES", "namespace.single"),
                ("STREAMFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();
                assert_eq!(config.activity_types, vec!["namespace.single"]);
            },
        );
    }

    #[test]
    fn test_from_env_activity_types_with_spaces() {
        with_env_vars(
            vec![
                (
                    "STREAMFLOW_ACTIVITY_TYPES",
                    "ns1.act1 , ns2.act2  ,  ns3.act3",
                ),
                ("STREAMFLOW_CLIENT_SECRET", "secret"),
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
