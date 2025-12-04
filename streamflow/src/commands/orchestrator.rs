use anyhow::Result;
use clap::Args;
use std::sync::Arc;
use std::time::Duration;
use streamflow_core::{
    ActivityQueue, EventSource, OrchestratorConfig, PostgresEventSource, PostgresQueue,
    QueueConfig, run_orchestrator,
};
use tokio_util::sync::CancellationToken;

/// Orchestrator command - Launch orchestrator service only
#[derive(Args)]
pub struct OrchestratorCommand {
    /// Orchestrator consumer ID (for event polling checkpoint)
    #[arg(
        long,
        env = "STREAMFLOW_ORCHESTRATOR_CONSUMER_ID",
        default_value = "orchestrator_default",
        help = "Unique consumer ID for event checkpointing",
        long_help = "Unique consumer ID for event checkpointing\n\n\
Consumer ID ensures event processing resumes from the correct position\n\
after restart. Each orchestrator instance should have a unique ID.\n\n\
Default: orchestrator_default\n\
Example: --consumer-id orch_prod_1"
    )]
    pub consumer_id: String,

    /// Event polling interval in milliseconds
    #[arg(
        long,
        env = "STREAMFLOW_ORCHESTRATOR_POLL_INTERVAL",
        default_value = "10",
        help = "Event polling interval in milliseconds",
        long_help = "How often the orchestrator polls for new workflow events\n\n\
Lower values reduce latency but increase database load.\n\
Default: 10ms (adaptive backoff to 5s when idle)\n\
Example: --poll-interval 50"
    )]
    pub poll_interval: u64,

    /// Shutdown timeout in seconds
    #[arg(
        long,
        env = "STREAMFLOW_SHUTDOWN_TIMEOUT",
        default_value = "30",
        help = "Graceful shutdown timeout in seconds"
    )]
    pub shutdown_timeout: u64,
}

impl OrchestratorCommand {
    pub fn validate(&self) -> Result<()> {
        if self.poll_interval == 0 || self.poll_interval > 10000 {
            anyhow::bail!("Poll interval must be between 1 and 10000 milliseconds");
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
        poll_interval_ms = cmd.poll_interval,
        "Starting StreamFlow orchestrator"
    );

    // Create shutdown coordinator
    let shutdown_token = CancellationToken::new();

    // Connect to database
    tracing::info!("Connecting to database...");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .min_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    tracing::info!("Database connection established");

    // Create activity queue
    let queue_config = QueueConfig::default();
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

    // Create event source
    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));

    // Create orchestrator config
    let config = OrchestratorConfig::new(pool.clone());

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
            config,
            Some(orch_shutdown_token),
        )
        .await
    });

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
            poll_interval: 10,
            shutdown_timeout: 30,
        }
    }

    #[test]
    fn test_orchestrator_command_defaults() {
        let cmd = valid_orchestrator_command();
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_orchestrator_command_invalid_poll_interval_zero() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval = 0;
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_orchestrator_command_invalid_poll_interval_too_high() {
        let mut cmd = valid_orchestrator_command();
        cmd.poll_interval = 10001;
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
        cmd.poll_interval = 1;
        cmd.shutdown_timeout = 5;
        assert!(cmd.validate().is_ok());

        // Test maximum boundaries
        cmd.poll_interval = 10000;
        cmd.shutdown_timeout = 300;
        assert!(cmd.validate().is_ok());
    }
}
