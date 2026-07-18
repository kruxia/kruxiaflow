use anyhow::Result;
use clap::Args;
use kruxiaflow_std_worker::{WorkerConfig, WorkerManager};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Worker command - Launch worker service only
#[derive(Args)]
pub struct WorkerCommand {
    /// API server URL to connect to
    #[arg(
        long,
        env = "KRUXIAFLOW_API_URL",
        default_value = "http://127.0.0.1:8080",
        help = "Kruxia Flow API server URL",
        long_help = "Kruxia Flow API server URL for activity polling\n\n\
Workers connect to the API server to poll for activities,\n\
report heartbeats, and submit results.\n\n\
Default: http://127.0.0.1:8080\n\
Example: --api-url https://kruxiaflow.example.com"
    )]
    pub api_url: String,

    /// Worker ID (auto-generated if not provided)
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER_ID",
        help = "Unique worker identifier",
        long_help = "Unique worker identifier\n\n\
If not provided, a UUID v7 is auto-generated.\n\
Useful for tracking and debugging.\n\n\
Example: --worker-id worker_payments_1"
    )]
    pub worker_id: Option<String>,

    /// Maximum concurrent in-flight activities
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_WORKER_MAX_ACTIVITIES",
        default_value = "16",
        help = "Maximum concurrent in-flight activities",
        long_help = "Maximum number of activities that can execute concurrently\n\n\
Uses semaphore-based concurrency for efficient resource usage.\n\
Activities complete independently without blocking each other.\n\n\
Default: 16\n\
Range: 1-100\n\
Example: --max-activities 32"
    )]
    pub max_activities: usize,

    /// Activity types to handle (comma-separated)
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER_ACTIVITY_TYPES",
        help = "Activity types to handle (comma-separated, default: all built-in)",
        long_help = "Activity types this worker handles\n\n\
If not specified, handles all built-in activity types.\n\
Use to create specialized workers for specific activities.\n\n\
Example: --activity-types std.echo,std.http_request,std.llm_prompt"
    )]
    pub activity_types: Option<String>,

    /// Maximum activities per poll
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES",
        default_value = "5",
        help = "Maximum activities to claim per poll",
        long_help = "Maximum number of activities each worker claims per poll\n\n\
Lower values (1-5) improve work distribution.\n\
Higher values reduce polling overhead.\n\n\
Default: 1\n\
Range: 1-100\n\
Example: --poll-max-activities 5"
    )]
    pub poll_max_activities: usize,

    /// Poll interval in milliseconds
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER_POLL_INTERVAL_MS",
        default_value = "100",
        help = "Activity poll interval in milliseconds"
    )]
    pub poll_interval: u64,

    /// Activity execution timeout in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_ACTIVITY_TIMEOUT",
        default_value = "300",
        help = "Activity execution timeout in seconds"
    )]
    pub activity_timeout: u64,

    /// Heartbeat interval in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_HEARTBEAT_INTERVAL",
        default_value = "30",
        help = "Heartbeat interval for long-running activities"
    )]
    pub heartbeat_interval: u64,

    /// OAuth client ID
    #[arg(
        long,
        env = "KRUXIAFLOW_CLIENT_ID",
        default_value = "kruxiaflow_worker",
        help = "OAuth client ID for API authentication"
    )]
    pub client_id: String,

    /// OAuth client secret
    #[arg(
        long,
        env = "KRUXIAFLOW_CLIENT_SECRET",
        help = "OAuth client secret for API authentication (required)"
    )]
    pub client_secret: Option<String>,

    /// Shutdown timeout in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_SHUTDOWN_TIMEOUT",
        default_value = "30",
        help = "Graceful shutdown timeout in seconds"
    )]
    pub shutdown_timeout: u64,
}

impl WorkerCommand {
    pub fn validate(&self) -> Result<()> {
        if self.max_activities == 0 || self.max_activities > 100 {
            anyhow::bail!("Max concurrent activities must be between 1 and 100");
        }

        if self.poll_max_activities == 0 || self.poll_max_activities > 100 {
            anyhow::bail!("Max activities per poll must be between 1 and 100");
        }

        if self.client_secret.is_none() {
            anyhow::bail!("Client secret required (--client-secret or KRUXIAFLOW_CLIENT_SECRET)");
        }

        if self.api_url.is_empty() {
            anyhow::bail!("API URL cannot be empty");
        }

        if self.shutdown_timeout < 5 || self.shutdown_timeout > 300 {
            anyhow::bail!("Shutdown timeout must be between 5 and 300 seconds");
        }

        Ok(())
    }
}

/// Load a secret from environment, supporting Docker secrets pattern.
/// Checks for `{name}_FILE` first (reads file contents), then falls back to `{name}` direct value.
fn load_secret(name: &str) -> Option<String> {
    // First check for _FILE variant (Docker secrets pattern)
    let file_var = format!("{}_FILE", name);
    if let Ok(file_path) = std::env::var(&file_var) {
        match std::fs::read_to_string(&file_path) {
            Ok(contents) => {
                tracing::debug!("Loaded {} from file: {}", name, file_path);
                return Some(contents.trim().to_string());
            }
            Err(e) => {
                tracing::warn!("Failed to read {} from {}: {}", file_var, file_path, e);
            }
        }
    }

    // Fall back to direct environment variable
    std::env::var(name).ok()
}

/// Execute worker command
pub async fn execute(mut cmd: WorkerCommand, database_url: String) -> Result<()> {
    // Load secrets from files if _FILE variants are set (Docker secrets pattern)
    if cmd.client_secret.is_none() {
        cmd.client_secret = load_secret("KRUXIAFLOW_CLIENT_SECRET");
    }

    cmd.validate()?;

    let worker_id = cmd
        .worker_id
        .clone()
        .unwrap_or_else(|| format!("worker_{}", Uuid::now_v7()));

    tracing::info!(
        worker_id = %worker_id,
        api_url = %cmd.api_url,
        max_concurrent_activities = cmd.max_activities,
        "Starting Kruxia Flow worker with semaphore-based concurrency"
    );

    // Connect to database for workflow storage access
    tracing::info!("Connecting to database...");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    tracing::info!("Database connection established");

    // Create workflow storage for artifact access
    let workflow_storage: Arc<dyn kruxiaflow_core::WorkflowStorage> =
        Arc::new(kruxiaflow_core::PostgresStorage::new(pool.clone()));

    // Create cache service based on environment configuration
    let cache_config = crate::config::CacheConfig::new();
    cache_config.log_config();
    let cache_service = cache_config.create_cache_service().await;

    // Create activity registry with built-in activities
    let registry = if let Some(ref types_str) = cmd.activity_types {
        // Filter registry to only specified types
        let requested_types: Vec<&str> = types_str.split(',').map(|s| s.trim()).collect();
        let full_registry = kruxiaflow_std_worker::register_std_activities(cache_service);

        // Log which types are available vs requested
        let available_types = full_registry.activity_types();
        tracing::info!(
            requested = ?requested_types,
            available = ?available_types,
            "Filtering activity types"
        );

        // For MVP, use full registry (filtering can be added later)
        // TODO: Implement registry filtering in kruxiaflow_worker
        full_registry
    } else {
        kruxiaflow_std_worker::register_std_activities(cache_service)
    };

    tracing::info!(
        activity_types = ?registry.activity_types(),
        "Activity registry initialized"
    );

    let config = WorkerConfig {
        api_url: cmd.api_url.clone(),
        worker_id: worker_id.clone(),
        worker: "std".to_string(),
        poll_max_activities: cmd.poll_max_activities,
        poll_interval: Duration::from_millis(cmd.poll_interval),
        max_concurrent_activities: cmd.max_activities,
        activity_timeout: Duration::from_secs(cmd.activity_timeout),
        heartbeat_interval: Duration::from_secs(cmd.heartbeat_interval),
        shutdown_timeout: Duration::from_secs(cmd.shutdown_timeout),
        client_id: Some(cmd.client_id.clone()),
        client_secret: cmd.client_secret.clone(),
    };

    let manager = WorkerManager::new(config, registry, workflow_storage);
    let handles = manager.start().await?;

    tracing::info!(
        worker_id = %worker_id,
        tasks = handles.len(),
        "Worker ready, polling for activities"
    );

    // Wait for shutdown signal
    let shutdown_signal = crate::signals::wait_for_shutdown();
    shutdown_signal.await;

    tracing::info!("Shutdown signal received, stopping workers...");

    // Stop workers
    for handle in handles {
        handle.abort();
    }

    // Brief drain period
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Close database pool
    pool.close().await;

    tracing::info!("Worker shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn valid_worker_command() -> WorkerCommand {
        WorkerCommand {
            api_url: "http://127.0.0.1:8080".to_string(),
            worker_id: Some("worker_test".to_string()),
            max_activities: 16,
            activity_types: None,
            poll_max_activities: 1,
            poll_interval: 100,
            activity_timeout: 300,
            heartbeat_interval: 30,
            client_id: "kruxiaflow_worker".to_string(),
            client_secret: Some("secret".to_string()),
            shutdown_timeout: 30,
        }
    }

    #[test]
    fn test_worker_command_defaults() {
        let cmd = valid_worker_command();
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_worker_command_missing_secret() {
        let mut cmd = valid_worker_command();
        cmd.client_secret = None;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_invalid_max_activities_zero() {
        let mut cmd = valid_worker_command();
        cmd.max_activities = 0;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_invalid_max_activities_over_100() {
        let mut cmd = valid_worker_command();
        cmd.max_activities = 101;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_empty_api_url() {
        let mut cmd = valid_worker_command();
        cmd.api_url = "".to_string();
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_invalid_poll_max_activities_zero() {
        let mut cmd = valid_worker_command();
        cmd.poll_max_activities = 0;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_invalid_poll_max_activities_over_100() {
        let mut cmd = valid_worker_command();
        cmd.poll_max_activities = 101;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_invalid_shutdown_timeout_too_low() {
        let mut cmd = valid_worker_command();
        cmd.shutdown_timeout = 4;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_invalid_shutdown_timeout_too_high() {
        let mut cmd = valid_worker_command();
        cmd.shutdown_timeout = 301;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_worker_command_valid_boundaries() {
        // Test minimum boundaries
        let mut cmd = valid_worker_command();
        cmd.max_activities = 1;
        cmd.poll_max_activities = 1;
        cmd.shutdown_timeout = 5;
        assert!(cmd.validate().is_ok());

        // Test maximum boundaries
        cmd.max_activities = 100;
        cmd.poll_max_activities = 100;
        cmd.shutdown_timeout = 300;
        assert!(cmd.validate().is_ok());
    }

    // =========================================================================
    // load_secret tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_load_secret_from_env_var() {
        unsafe {
            std::env::set_var("TEST_WORKER_SECRET_A", "direct_value");
            std::env::remove_var("TEST_WORKER_SECRET_A_FILE");
        }

        let result = load_secret("TEST_WORKER_SECRET_A");
        assert_eq!(result, Some("direct_value".to_string()));

        unsafe {
            std::env::remove_var("TEST_WORKER_SECRET_A");
        }
    }

    #[test]
    #[serial]
    fn test_load_secret_from_file() {
        let file_path = std::env::temp_dir().join("kruxiaflow_test_worker_secret_b.txt");
        std::fs::write(&file_path, "file_value\n").unwrap();

        unsafe {
            std::env::set_var("TEST_WORKER_SECRET_B_FILE", file_path.to_str().unwrap());
            std::env::set_var("TEST_WORKER_SECRET_B", "ignored");
        }

        let result = load_secret("TEST_WORKER_SECRET_B");
        assert_eq!(result, Some("file_value".to_string()));

        unsafe {
            std::env::remove_var("TEST_WORKER_SECRET_B_FILE");
            std::env::remove_var("TEST_WORKER_SECRET_B");
        }
        let _ = std::fs::remove_file(&file_path);
    }

    #[test]
    #[serial]
    fn test_load_secret_file_trims_whitespace() {
        let file_path = std::env::temp_dir().join("kruxiaflow_test_worker_secret_c.txt");
        std::fs::write(&file_path, "  trimmed  \n").unwrap();

        unsafe {
            std::env::set_var("TEST_WORKER_SECRET_C_FILE", file_path.to_str().unwrap());
        }

        let result = load_secret("TEST_WORKER_SECRET_C");
        assert_eq!(result, Some("trimmed".to_string()));

        unsafe {
            std::env::remove_var("TEST_WORKER_SECRET_C_FILE");
        }
        let _ = std::fs::remove_file(&file_path);
    }

    #[test]
    #[serial]
    fn test_load_secret_not_set() {
        unsafe {
            std::env::remove_var("TEST_WORKER_SECRET_D");
            std::env::remove_var("TEST_WORKER_SECRET_D_FILE");
        }

        let result = load_secret("TEST_WORKER_SECRET_D");
        assert_eq!(result, None);
    }

    #[test]
    #[serial]
    fn test_load_secret_file_not_found_falls_back() {
        unsafe {
            std::env::set_var("TEST_WORKER_SECRET_E_FILE", "/nonexistent/path.txt");
            std::env::set_var("TEST_WORKER_SECRET_E", "fallback");
        }

        let result = load_secret("TEST_WORKER_SECRET_E");
        assert_eq!(result, Some("fallback".to_string()));

        unsafe {
            std::env::remove_var("TEST_WORKER_SECRET_E_FILE");
            std::env::remove_var("TEST_WORKER_SECRET_E");
        }
    }

    // =========================================================================
    // Additional worker command tests
    // =========================================================================

    #[test]
    fn test_worker_command_with_activity_types() {
        let mut cmd = valid_worker_command();
        cmd.activity_types = Some("std.echo,std.llm_prompt".to_string());
        assert!(cmd.validate().is_ok());
        assert_eq!(
            cmd.activity_types,
            Some("std.echo,std.llm_prompt".to_string())
        );
    }

    #[test]
    fn test_worker_command_no_worker_id() {
        let mut cmd = valid_worker_command();
        cmd.worker_id = None;
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_worker_command_custom_timeouts() {
        let mut cmd = valid_worker_command();
        cmd.activity_timeout = 600;
        cmd.heartbeat_interval = 60;
        cmd.poll_interval = 200;
        assert!(cmd.validate().is_ok());
        assert_eq!(cmd.activity_timeout, 600);
        assert_eq!(cmd.heartbeat_interval, 60);
        assert_eq!(cmd.poll_interval, 200);
    }

    #[test]
    fn test_worker_command_validation_order_max_activities_first() {
        let mut cmd = valid_worker_command();
        cmd.max_activities = 0;
        cmd.poll_max_activities = 0;
        cmd.client_secret = None;
        cmd.api_url = "".to_string();

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Max concurrent activities")
        );
    }

    #[test]
    fn test_worker_command_validation_order_poll_after_max() {
        let mut cmd = valid_worker_command();
        cmd.poll_max_activities = 0;
        cmd.client_secret = None;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Max activities per poll")
        );
    }

    #[test]
    fn test_worker_command_validation_order_secret_after_poll() {
        let mut cmd = valid_worker_command();
        cmd.client_secret = None;
        cmd.api_url = "".to_string();

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Client secret"));
    }

    #[test]
    fn test_worker_command_validation_order_api_url_after_secret() {
        let mut cmd = valid_worker_command();
        cmd.api_url = "".to_string();
        cmd.shutdown_timeout = 1;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API URL"));
    }
}
