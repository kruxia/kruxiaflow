use crate::queue::{QueueConfig, Result};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

pub struct QueueMonitor {
    pool: PgPool,
    config: QueueConfig,
}

impl QueueMonitor {
    pub fn new(pool: PgPool, config: QueueConfig) -> Self {
        Self { pool, config }
    }

    /// Run the monitoring tasks (cleanup and vacuum)
    pub async fn run(self: Arc<Self>) {
        let cleanup_handle = {
            let monitor = Arc::clone(&self);
            tokio::spawn(async move {
                monitor.run_cleanup_loop().await;
            })
        };

        let vacuum_handle = {
            let monitor = Arc::clone(&self);
            tokio::spawn(async move {
                monitor.run_vacuum_loop().await;
            })
        };

        // Wait for both tasks (they run forever unless cancelled)
        let _ = tokio::join!(cleanup_handle, vacuum_handle);
    }

    /// Cleanup thread for activities that exceeded max_retries
    async fn run_cleanup_loop(&self) {
        let mut interval = interval(self.config.cleanup_interval);
        info!(
            interval_secs = self.config.cleanup_interval.as_secs(),
            "Started failed activity cleanup thread"
        );

        loop {
            interval.tick().await;

            if let Err(e) = self.cleanup_failed_activities().await {
                error!(error = %e, "Failed to cleanup failed activities");
            }
        }
    }

    /// Delete activities that exceeded max_retries and are timed out
    async fn cleanup_failed_activities(&self) -> Result<()> {
        let result = sqlx::query!(
            r#"
            DELETE FROM activity_queue
            WHERE status = 'running'::activity_status
              AND NOW() > claimed_at + timeout_duration
              AND retry_count >= max_retries
            RETURNING id, workflow_id, activity_key, namespace, name, retry_count
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        if !result.is_empty() {
            warn!(
                count = result.len(),
                "Cleaned up failed activities that exceeded max retries"
            );

            for row in result {
                warn!(
                    activity_id = %row.id,
                    workflow_id = %row.workflow_id,
                    activity_key = %row.activity_key,
                    namespace = %row.namespace,
                    name = %row.name,
                    retry_count = row.retry_count,
                    "Activity permanently failed after max retries"
                );

                // TODO: Publish failure event when EventSource is available (US-1.2)
                // For now, just log the failure
            }
        } else {
            debug!("No failed activities to cleanup");
        }

        Ok(())
    }

    /// Vacuum thread to prevent table bloat
    async fn run_vacuum_loop(&self) {
        let mut interval = interval(self.config.vacuum_interval);
        info!(
            interval_secs = self.config.vacuum_interval.as_secs(),
            "Started vacuum monitor thread"
        );

        loop {
            interval.tick().await;

            if let Err(e) = self.vacuum_queue_table().await {
                error!(error = %e, "Failed to vacuum activity_queue table");
            }
        }
    }

    /// Run VACUUM ANALYZE on activity_queue table
    async fn vacuum_queue_table(&self) -> Result<()> {
        debug!("Running VACUUM ANALYZE on activity_queue");

        // VACUUM cannot run in a transaction, so we use a raw query
        sqlx::query("VACUUM ANALYZE activity_queue")
            .execute(&self.pool)
            .await?;

        debug!("VACUUM ANALYZE completed on activity_queue");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_monitor_construction() {
        let config = QueueConfig::default();
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@localhost:5432/streamflow".to_string()
        });

        // Only run if database is available
        if let Ok(pool) = PgPool::connect(&database_url).await {
            let _monitor = QueueMonitor::new(pool, config);
            // Just verify it constructs successfully
        }
    }
}
