use anyhow::Result;
use clap::Args;
use kruxiaflow_core::{
    ActivityQueue, EventSource, OrchestratorConfig, PostgresEventSource, PostgresQueue,
    PostgresSubscriptionService, QueueConfig, SubscriptionService, run_orchestrator,
};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Orchestrator command - Launch orchestrator service only
#[derive(Args)]
pub struct OrchestratorCommand {
    /// Orchestrator consumer ID (for event polling checkpoint)
    #[arg(
        long,
        env = "KRUXIAFLOW_ORCHESTRATOR_CONSUMER_ID",
        default_value = "orchestrator_default",
        help = "Unique consumer ID for event checkpointing",
        long_help = "Unique consumer ID for event checkpointing\n\n\
Consumer ID ensures event processing resumes from the correct position\n\
after restart. Each orchestrator instance should have a unique ID.\n\n\
Default: orchestrator_default\n\
Example: --consumer-id orch_prod_1"
    )]
    pub consumer_id: String,

    /// Minimum event polling interval in milliseconds
    #[arg(
        long,
        env = "KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS",
        default_value = "50",
        help = "Minimum event polling interval in milliseconds",
        long_help = "Minimum interval between database polls for new workflow events\n\n\
This is the fastest the orchestrator will poll when actively processing events.\n\
Lower values reduce latency but increase database load.\n\
Default: 50ms (20 polls/sec max)\n\
Example: --poll-interval-min 100"
    )]
    pub poll_interval_min: u64,

    /// Maximum event polling interval in milliseconds
    #[arg(
        long,
        env = "KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS",
        default_value = "1000",
        help = "Maximum event polling interval in milliseconds",
        long_help = "Maximum interval between database polls when the system is idle\n\n\
The orchestrator backs off to this interval when no events are being processed.\n\
Default: 1000ms (1 poll/sec when idle)\n\
Example: --poll-interval-max 2000"
    )]
    pub poll_interval_max: u64,

    /// Backoff multiplier for polling interval
    #[arg(
        long,
        env = "KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER",
        default_value = "1.5",
        help = "Backoff multiplier for polling interval",
        long_help = "Multiplier applied to polling interval after each empty poll\n\n\
Higher values cause faster backoff but higher latency spikes.\n\
Default: 1.5\n\
Example: --backoff-multiplier 2.0"
    )]
    pub backoff_multiplier: f64,

    /// Shutdown timeout in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_SHUTDOWN_TIMEOUT",
        default_value = "30",
        help = "Graceful shutdown timeout in seconds"
    )]
    pub shutdown_timeout: u64,
}

impl OrchestratorCommand {
    pub fn validate(&self) -> Result<()> {
        if self.poll_interval_min == 0 || self.poll_interval_min > 10000 {
            anyhow::bail!("Minimum poll interval must be between 1 and 10000 milliseconds");
        }

        if self.poll_interval_max < self.poll_interval_min || self.poll_interval_max > 60000 {
            anyhow::bail!(
                "Maximum poll interval must be >= minimum ({}) and <= 60000 milliseconds",
                self.poll_interval_min
            );
        }

        if self.backoff_multiplier < 1.0 || self.backoff_multiplier > 10.0 {
            anyhow::bail!("Backoff multiplier must be between 1.0 and 10.0");
        }

        if self.shutdown_timeout < 5 || self.shutdown_timeout > 300 {
            anyhow::bail!("Shutdown timeout must be between 5 and 300 seconds");
        }

        Ok(())
    }
}

/// Execute orchestrator command
pub async fn execute(cmd: OrchestratorCommand, database_url: String) -> Result<()> {
    cmd.validate()?;

    tracing::info!(
        consumer_id = %cmd.consumer_id,
        poll_interval_min_ms = cmd.poll_interval_min,
        poll_interval_max_ms = cmd.poll_interval_max,
        backoff_multiplier = cmd.backoff_multiplier,
        "Starting Kruxia Flow orchestrator"
    );

    // Create shutdown coordinator
    let shutdown_token = CancellationToken::new();

    // Connect to database
    tracing::info!("Connecting to database...");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(50)
        .min_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    tracing::info!("Database connection established");

    // Create activity queue
    let queue_config = QueueConfig::from_env();
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

    // Create event source
    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));

    // Create subscription service
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    // Create orchestrator config from CLI parameters
    let config = OrchestratorConfig::new(pool.clone())
        .with_poll_interval(
            Duration::from_millis(cmd.poll_interval_min),
            Duration::from_millis(cmd.poll_interval_max),
        )
        .with_backoff_multiplier(cmd.backoff_multiplier);

    tracing::info!(
        consumer_id = %cmd.consumer_id,
        "Orchestrator ready, starting event loop"
    );

    // Spawn orchestrator task
    let orch_shutdown_token = shutdown_token.clone();
    let orchestrator_handle = tokio::spawn(async move {
        run_orchestrator(
            event_source,
            activity_queue,
            subscription_service,
            config,
            Some(orch_shutdown_token),
        )
        .await
    });

    // Spawn recurring-schedule loop alongside the orchestrator (env-gated;
    // multiple instances are safe — SKIP LOCKED claims + idempotent
    // unique_key submissions)
    let scheduler_handle = {
        let pool = pool.clone();
        let token = shutdown_token.clone();
        tokio::spawn(async move {
            kruxiaflow_core::run_scheduler(
                pool,
                kruxiaflow_core::SchedulerConfig::from_env(),
                Some(token),
            )
            .await;
        })
    };

    // Wait for shutdown signal
    let shutdown_signal = crate::signals::wait_for_shutdown();
    shutdown_signal.await;

    tracing::info!("Shutdown signal received, stopping orchestrator...");
    shutdown_token.cancel();

    // Wait for orchestrator to stop
    let shutdown_timeout = Duration::from_secs(cmd.shutdown_timeout);
    match tokio::time::timeout(shutdown_timeout, orchestrator_handle).await {
        Ok(Ok(Ok(()))) => tracing::info!("Orchestrator stopped gracefully"),
        Ok(Ok(Err(e))) => tracing::warn!("Orchestrator error during shutdown: {}", e),
        Ok(Err(e)) => tracing::warn!("Orchestrator task error: {}", e),
        Err(_) => tracing::warn!("Orchestrator shutdown timeout, forcing stop"),
    }

    // Wait for scheduler to stop (exits via shutdown token)
    match tokio::time::timeout(Duration::from_secs(5), scheduler_handle).await {
        Ok(Ok(())) => tracing::info!("Scheduler stopped gracefully"),
        Ok(Err(e)) => tracing::warn!("Scheduler task error: {}", e),
        Err(_) => tracing::warn!("Scheduler shutdown timeout, forcing stop"),
    }

    // Close database pool
    tracing::info!("Closing database pool...");
    pool.close().await;

    tracing::info!("Orchestrator shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_orchestrator_command() -> OrchestratorCommand {
        OrchestratorCommand {
            consumer_id: "orch_test".to_string(),
            poll_interval_min: 50,
            poll_interval_max: 1000,
            backoff_multiplier: 1.5,
            shutdown_timeout: 30,
        }
    }

    #[test]
    fn test_orchestrator_command_defaults() {
        let cmd = valid_orchestrator_command();
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_orchestrator_command_invalid_poll_interval_min_zero() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_min = 0;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_poll_interval_min_too_high() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_min = 10001;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_poll_interval_max_less_than_min() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_max = 10; // Less than min (50)
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_poll_interval_max_too_high() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_max = 60001;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_backoff_multiplier_too_low() {
        let mut cmd = valid_orchestrator_command();
        cmd.backoff_multiplier = 0.5;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_backoff_multiplier_too_high() {
        let mut cmd = valid_orchestrator_command();
        cmd.backoff_multiplier = 11.0;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_shutdown_timeout_too_low() {
        let mut cmd = valid_orchestrator_command();
        cmd.shutdown_timeout = 4;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_shutdown_timeout_too_high() {
        let mut cmd = valid_orchestrator_command();
        cmd.shutdown_timeout = 301;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_valid_boundaries() {
        // Test minimum boundaries
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_min = 1;
        cmd.poll_interval_max = 1; // Can equal min
        cmd.backoff_multiplier = 1.0;
        cmd.shutdown_timeout = 5;
        assert!(cmd.validate().is_ok());

        // Test maximum boundaries
        cmd.poll_interval_min = 10000;
        cmd.poll_interval_max = 60000;
        cmd.backoff_multiplier = 10.0;
        cmd.shutdown_timeout = 300;
        assert!(cmd.validate().is_ok());
    }

    // =========================================================================
    // Validation order tests
    // =========================================================================

    #[test]
    fn test_orchestrator_command_validation_order_min_poll_first() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_min = 0;
        cmd.poll_interval_max = 0;
        cmd.backoff_multiplier = 0.0;
        cmd.shutdown_timeout = 0;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Minimum poll interval")
        );
    }

    #[test]
    fn test_orchestrator_command_validation_order_max_poll_after_min() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval_max = 10; // Less than min (50)
        cmd.backoff_multiplier = 0.0;
        cmd.shutdown_timeout = 0;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Maximum poll interval")
        );
    }

    #[test]
    fn test_orchestrator_command_validation_order_backoff_after_max_poll() {
        let mut cmd = valid_orchestrator_command();
        cmd.backoff_multiplier = 0.5;
        cmd.shutdown_timeout = 0;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Backoff multiplier")
        );
    }

    #[test]
    fn test_orchestrator_command_validation_order_shutdown_after_backoff() {
        let mut cmd = valid_orchestrator_command();
        cmd.shutdown_timeout = 1;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Shutdown timeout"));
    }

    // =========================================================================
    // Construction tests
    // =========================================================================

    #[test]
    fn test_orchestrator_command_custom_consumer_id() {
        let cmd = OrchestratorCommand {
            consumer_id: "orch_prod_1".to_string(),
            poll_interval_min: 100,
            poll_interval_max: 5000,
            backoff_multiplier: 2.0,
            shutdown_timeout: 60,
        };

        assert!(cmd.validate().is_ok());
        assert_eq!(cmd.consumer_id, "orch_prod_1");
    }

    #[test]
    fn test_orchestrator_command_poll_interval_max_equals_min() {
        let cmd = OrchestratorCommand {
            consumer_id: "test".to_string(),
            poll_interval_min: 500,
            poll_interval_max: 500, // Equal to min
            backoff_multiplier: 1.0,
            shutdown_timeout: 30,
        };

        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_orchestrator_command_error_message_includes_min_interval() {
        let cmd = OrchestratorCommand {
            consumer_id: "test".to_string(),
            poll_interval_min: 200,
            poll_interval_max: 100, // Less than min
            backoff_multiplier: 1.5,
            shutdown_timeout: 30,
        };

        let result = cmd.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("200")); // Should include the min value
    }
}
