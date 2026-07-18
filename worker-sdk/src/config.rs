//! Worker configuration from environment variables or a builder.

use crate::error::ConfigError;
use std::time::Duration;

/// Worker configuration.
///
/// Environment variables (matching the Python SDK):
///
/// | Variable                                 | Required | Default        |
/// |------------------------------------------|----------|----------------|
/// | `KRUXIAFLOW_API_URL`                     | yes      | —              |
/// | `KRUXIAFLOW_CLIENT_ID`                   | yes*     | —              |
/// | `KRUXIAFLOW_CLIENT_SECRET`               | yes*     | —              |
/// | `KRUXIAFLOW_WORKER`                      | no       | inferred from registered activities |
/// | `KRUXIAFLOW_WORKER_ID`                   | no       | auto-generated |
/// | `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES`  | no       | 10             |
/// | `KRUXIAFLOW_WORKER_POLL_INTERVAL`        | no       | 0.1 (seconds)  |
/// | `KRUXIAFLOW_WORKER_MAX_ACTIVITIES`       | no       | 16             |
/// | `KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT`     | no       | 300 (seconds)  |
/// | `KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL`   | no       | 30 (seconds)   |
/// | `KRUXIAFLOW_WORKER_SHUTDOWN_TIMEOUT`     | no       | 30 (seconds)   |
///
/// \* not required when the server runs in dev mode (`--insecure-dev`).
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// API server base URL
    pub api_url: String,

    /// Worker unique identifier
    pub worker_id: String,

    /// Worker type this worker polls for (the workflow definition's
    /// `worker:` field). Empty means: infer from registered activities.
    pub worker: String,

    /// Maximum number of activities to claim per poll request
    pub poll_max_activities: usize,

    /// Polling interval when no activities are available
    pub poll_interval: Duration,

    /// Maximum number of concurrent in-flight activities (semaphore-based)
    pub max_concurrent_activities: usize,

    /// Default activity execution timeout (a queued activity's
    /// `settings.timeout` overrides this per activity)
    pub activity_timeout: Duration,

    /// Heartbeat interval for long-running activities
    pub heartbeat_interval: Duration,

    /// How long graceful shutdown waits for in-flight activities to drain
    /// before failing them as retryable so they re-queue
    pub shutdown_timeout: Duration,

    /// OAuth client id; `None` for a dev-mode server
    pub client_id: Option<String>,

    /// OAuth client secret; `None` for a dev-mode server
    pub client_secret: Option<String>,
}

fn default_worker_id() -> String {
    format!("worker_{}", uuid::Uuid::now_v7())
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080".to_string(),
            worker_id: default_worker_id(),
            worker: String::new(),
            poll_max_activities: 10,
            poll_interval: Duration::from_millis(100),
            max_concurrent_activities: 16,
            activity_timeout: Duration::from_secs(300),
            heartbeat_interval: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(30),
            client_id: None,
            client_secret: None,
        }
    }
}

fn env_nonempty(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

fn env_parse<T: std::str::FromStr>(var: &str) -> Result<Option<T>, ConfigError> {
    match env_nonempty(var) {
        None => Ok(None),
        Some(raw) => raw.parse().map(Some).map_err(|_| ConfigError::InvalidValue {
            var: var.to_string(),
            reason: format!("cannot parse {raw:?}"),
        }),
    }
}

fn env_seconds(var: &str) -> Result<Option<Duration>, ConfigError> {
    match env_parse::<f64>(var)? {
        None => Ok(None),
        Some(seconds) if seconds > 0.0 && seconds.is_finite() => {
            Ok(Some(Duration::from_secs_f64(seconds)))
        }
        Some(_) => Err(ConfigError::InvalidValue {
            var: var.to_string(),
            reason: "must be a positive number of seconds".to_string(),
        }),
    }
}

impl WorkerConfig {
    /// Load configuration from `KRUXIAFLOW_*` environment variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self {
            api_url: env_nonempty("KRUXIAFLOW_API_URL").ok_or(ConfigError::MissingApiUrl)?,
            ..Self::default()
        };

        if let Some(id) = env_nonempty("KRUXIAFLOW_WORKER_ID") {
            config.worker_id = id;
        }
        if let Some(worker) = env_nonempty("KRUXIAFLOW_WORKER") {
            config.worker = worker;
        }
        if let Some(n) = env_parse("KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES")? {
            config.poll_max_activities = n;
        }
        if let Some(n) = env_parse("KRUXIAFLOW_WORKER_MAX_ACTIVITIES")? {
            config.max_concurrent_activities = n;
        }
        if let Some(d) = env_seconds("KRUXIAFLOW_WORKER_POLL_INTERVAL")? {
            config.poll_interval = d;
        }
        if let Some(d) = env_seconds("KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT")? {
            config.activity_timeout = d;
        }
        if let Some(d) = env_seconds("KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL")? {
            config.heartbeat_interval = d;
        }
        if let Some(d) = env_seconds("KRUXIAFLOW_WORKER_SHUTDOWN_TIMEOUT")? {
            config.shutdown_timeout = d;
        }
        config.client_id = env_nonempty("KRUXIAFLOW_CLIENT_ID");
        config.client_secret = env_nonempty("KRUXIAFLOW_CLIENT_SECRET");

        config.validate()?;
        Ok(config)
    }

    /// Start building a configuration programmatically.
    pub fn builder() -> WorkerConfigBuilder {
        WorkerConfigBuilder {
            config: Self::default(),
        }
    }

    /// Validate invariants (counts positive, credentials all-or-nothing).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.api_url.is_empty() {
            return Err(ConfigError::MissingApiUrl);
        }
        if self.max_concurrent_activities == 0 {
            return Err(ConfigError::InvalidValue {
                var: "max_concurrent_activities".to_string(),
                reason: "must be > 0".to_string(),
            });
        }
        if self.poll_max_activities == 0 {
            return Err(ConfigError::InvalidValue {
                var: "poll_max_activities".to_string(),
                reason: "must be > 0".to_string(),
            });
        }
        if self.client_id.is_some() != self.client_secret.is_some() {
            return Err(ConfigError::PartialCredentials);
        }
        Ok(())
    }
}

/// Builder for [`WorkerConfig`].
pub struct WorkerConfigBuilder {
    config: WorkerConfig,
}

impl WorkerConfigBuilder {
    /// Set the API server base URL.
    pub fn api_url(mut self, api_url: impl Into<String>) -> Self {
        self.config.api_url = api_url.into();
        self
    }

    /// Set the worker unique identifier.
    pub fn worker_id(mut self, worker_id: impl Into<String>) -> Self {
        self.config.worker_id = worker_id.into();
        self
    }

    /// Set the worker type to poll for.
    pub fn worker(mut self, worker: impl Into<String>) -> Self {
        self.config.worker = worker.into();
        self
    }

    /// Set OAuth client credentials.
    pub fn credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.config.client_id = Some(client_id.into());
        self.config.client_secret = Some(client_secret.into());
        self
    }

    /// Set the maximum activities claimed per poll request.
    pub fn poll_max_activities(mut self, n: usize) -> Self {
        self.config.poll_max_activities = n;
        self
    }

    /// Set the polling interval used when no activities are available.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.config.poll_interval = interval;
        self
    }

    /// Set the maximum concurrent in-flight activities.
    pub fn max_concurrent_activities(mut self, n: usize) -> Self {
        self.config.max_concurrent_activities = n;
        self
    }

    /// Set the default activity execution timeout.
    pub fn activity_timeout(mut self, timeout: Duration) -> Self {
        self.config.activity_timeout = timeout;
        self
    }

    /// Set the heartbeat interval for long-running activities.
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.config.heartbeat_interval = interval;
        self
    }

    /// Set the graceful-shutdown drain deadline.
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.config.shutdown_timeout = timeout;
        self
    }

    /// Validate and produce the configuration.
    pub fn build(self) -> Result<WorkerConfig, ConfigError> {
        self.config.validate()?;
        Ok(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-mutating tests
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    const ENV_VARS: &[&str] = &[
        "KRUXIAFLOW_API_URL",
        "KRUXIAFLOW_WORKER_ID",
        "KRUXIAFLOW_WORKER",
        "KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES",
        "KRUXIAFLOW_WORKER_POLL_INTERVAL",
        "KRUXIAFLOW_WORKER_MAX_ACTIVITIES",
        "KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT",
        "KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL",
        "KRUXIAFLOW_WORKER_SHUTDOWN_TIMEOUT",
        "KRUXIAFLOW_CLIENT_ID",
        "KRUXIAFLOW_CLIENT_SECRET",
    ];

    fn with_env_vars<F: FnOnce()>(vars: Vec<(&str, &str)>, test: F) {
        let _lock = ENV_MUTEX.lock().unwrap();
        unsafe {
            for var in ENV_VARS {
                std::env::remove_var(var);
            }
            for (key, value) in vars {
                std::env::set_var(key, value);
            }
        }
        test();
        unsafe {
            for var in ENV_VARS {
                std::env::remove_var(var);
            }
        }
    }

    #[test]
    fn defaults() {
        let config = WorkerConfig::default();
        assert_eq!(config.poll_max_activities, 10);
        assert_eq!(config.poll_interval, Duration::from_millis(100));
        assert_eq!(config.max_concurrent_activities, 16);
        assert_eq!(config.activity_timeout, Duration::from_secs(300));
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.shutdown_timeout, Duration::from_secs(30));
        assert!(config.worker_id.starts_with("worker_"));
        assert!(config.client_id.is_none());
    }

    #[test]
    fn from_env_requires_api_url() {
        with_env_vars(vec![], || {
            assert!(matches!(
                WorkerConfig::from_env(),
                Err(ConfigError::MissingApiUrl)
            ));
        });
    }

    #[test]
    fn from_env_full() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_API_URL", "http://api.example.com:9090"),
                ("KRUXIAFLOW_WORKER_ID", "custom_worker_123"),
                ("KRUXIAFLOW_WORKER", "custom"),
                ("KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES", "8"),
                ("KRUXIAFLOW_WORKER_POLL_INTERVAL", "0.5"),
                ("KRUXIAFLOW_WORKER_MAX_ACTIVITIES", "32"),
                ("KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT", "120"),
                ("KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL", "15"),
                ("KRUXIAFLOW_WORKER_SHUTDOWN_TIMEOUT", "10"),
                ("KRUXIAFLOW_CLIENT_ID", "client"),
                ("KRUXIAFLOW_CLIENT_SECRET", "secret"),
            ],
            || {
                let config = WorkerConfig::from_env().unwrap();
                assert_eq!(config.api_url, "http://api.example.com:9090");
                assert_eq!(config.worker_id, "custom_worker_123");
                assert_eq!(config.worker, "custom");
                assert_eq!(config.poll_max_activities, 8);
                assert_eq!(config.poll_interval, Duration::from_millis(500));
                assert_eq!(config.max_concurrent_activities, 32);
                assert_eq!(config.activity_timeout, Duration::from_secs(120));
                assert_eq!(config.heartbeat_interval, Duration::from_secs(15));
                assert_eq!(config.shutdown_timeout, Duration::from_secs(10));
                assert_eq!(config.client_id.as_deref(), Some("client"));
                assert_eq!(config.client_secret.as_deref(), Some("secret"));
            },
        );
    }

    #[test]
    fn from_env_without_credentials_is_valid() {
        with_env_vars(vec![("KRUXIAFLOW_API_URL", "http://localhost:8080")], || {
            let config = WorkerConfig::from_env().unwrap();
            assert!(config.client_id.is_none());
            assert!(config.client_secret.is_none());
        });
    }

    #[test]
    fn from_env_partial_credentials_rejected() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_API_URL", "http://localhost:8080"),
                ("KRUXIAFLOW_CLIENT_ID", "client"),
            ],
            || {
                assert!(matches!(
                    WorkerConfig::from_env(),
                    Err(ConfigError::PartialCredentials)
                ));
            },
        );
    }

    #[test]
    fn from_env_invalid_number() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_API_URL", "http://localhost:8080"),
                ("KRUXIAFLOW_WORKER_MAX_ACTIVITIES", "not_a_number"),
            ],
            || {
                assert!(matches!(
                    WorkerConfig::from_env(),
                    Err(ConfigError::InvalidValue { .. })
                ));
            },
        );
    }

    #[test]
    fn from_env_rejects_nonpositive_interval() {
        with_env_vars(
            vec![
                ("KRUXIAFLOW_API_URL", "http://localhost:8080"),
                ("KRUXIAFLOW_WORKER_POLL_INTERVAL", "0"),
            ],
            || {
                assert!(matches!(
                    WorkerConfig::from_env(),
                    Err(ConfigError::InvalidValue { .. })
                ));
            },
        );
    }

    #[test]
    fn builder_roundtrip() {
        let config = WorkerConfig::builder()
            .api_url("http://localhost:9999")
            .worker("demo")
            .worker_id("w1")
            .credentials("id", "secret")
            .poll_max_activities(3)
            .poll_interval(Duration::from_millis(50))
            .max_concurrent_activities(4)
            .activity_timeout(Duration::from_secs(60))
            .heartbeat_interval(Duration::from_secs(10))
            .shutdown_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        assert_eq!(config.api_url, "http://localhost:9999");
        assert_eq!(config.worker, "demo");
        assert_eq!(config.max_concurrent_activities, 4);
    }

    #[test]
    fn builder_rejects_zero_concurrency() {
        assert!(
            WorkerConfig::builder()
                .max_concurrent_activities(0)
                .build()
                .is_err()
        );
    }
}
