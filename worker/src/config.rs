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
    pub max_activities_per_poll: usize,

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
            max_activities_per_poll: 10,
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
