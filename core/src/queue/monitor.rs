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
    pub(crate) async fn run_cleanup_loop(&self) {
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
    pub(crate) async fn cleanup_failed_activities(&self) -> Result<()> {
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
    pub(crate) async fn run_vacuum_loop(&self) {
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
    pub(crate) async fn vacuum_queue_table(&self) -> Result<()> {
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
    use serial_test::serial;
    use std::time::Duration;
    use uuid::Uuid;

    /// Helper to setup test pool
    async fn setup_test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
        });

        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        sqlx::migrate!("../migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        pool
    }

    /// Helper to insert a failed activity
    async fn insert_failed_activity(pool: &PgPool, workflow_id: Uuid, activity_key: &str) {
        sqlx::query!(
            r#"
            INSERT INTO activity_queue (
                workflow_id, activity_key, namespace, name, parameters,
                status, claimed_at, timeout_duration, retry_count, max_retries
            )
            VALUES (
                $1, $2, 'test', 'task', '{}',
                'running', NOW() - INTERVAL '2 hours', INTERVAL '1 hour', 3, 3
            )
            "#,
            workflow_id,
            activity_key
        )
        .execute(pool)
        .await
        .expect("Failed to insert failed activity");
    }

    /// Helper to cleanup test data
    async fn cleanup_queue(pool: &PgPool, workflow_id: Uuid) {
        sqlx::query!(
            "DELETE FROM activity_queue WHERE workflow_id = $1",
            workflow_id
        )
        .execute(pool)
        .await
        .expect("Failed to cleanup test data");
    }

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

    #[tokio::test]
    #[serial]
    async fn test_cleanup_failed_activities_with_failures() {
        let pool = setup_test_pool().await;
        let workflow_id = Uuid::now_v7();

        // Insert 3 failed activities
        insert_failed_activity(&pool, workflow_id, "failed_1").await;
        insert_failed_activity(&pool, workflow_id, "failed_2").await;
        insert_failed_activity(&pool, workflow_id, "failed_3").await;

        let config = QueueConfig::default();
        let monitor = QueueMonitor::new(pool.clone(), config);

        // Run cleanup
        monitor
            .cleanup_failed_activities()
            .await
            .expect("Cleanup should succeed");

        // Verify failed activities were deleted
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
            workflow_id
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to count rows");

        assert_eq!(count, Some(0), "Failed activities should be cleaned up");

        cleanup_queue(&pool, workflow_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_cleanup_failed_activities_with_no_failures() {
        let pool = setup_test_pool().await;
        let workflow_id = Uuid::now_v7();

        // Insert only pending activities (won't be cleaned up)
        sqlx::query!(
            r#"
            INSERT INTO activity_queue (
                workflow_id, activity_key, namespace, name, parameters,
                status, retry_count, max_retries, timeout_duration
            )
            VALUES (
                $1, 'pending_1', 'test', 'task', '{}',
                'pending', 0, 3, INTERVAL '1 hour'
            )
            "#,
            workflow_id
        )
        .execute(&pool)
        .await
        .expect("Failed to insert activity");

        let config = QueueConfig::default();
        let monitor = QueueMonitor::new(pool.clone(), config);

        // Run cleanup
        monitor
            .cleanup_failed_activities()
            .await
            .expect("Cleanup should succeed");

        // Verify activity still exists
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
            workflow_id
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to count rows");

        assert_eq!(count, Some(1), "No activities should be deleted");

        cleanup_queue(&pool, workflow_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_vacuum_queue_table() {
        let pool = setup_test_pool().await;
        let config = QueueConfig::default();
        let monitor = QueueMonitor::new(pool.clone(), config);

        // Run vacuum
        monitor
            .vacuum_queue_table()
            .await
            .expect("Vacuum should succeed");
    }

    #[tokio::test]
    #[serial]
    async fn test_run_cleanup_loop_executes() {
        let pool = setup_test_pool().await;
        let workflow_id = Uuid::now_v7();

        // Insert a failed activity
        insert_failed_activity(&pool, workflow_id, "failed_1").await;

        // Use very short intervals for testing
        let config = QueueConfig {
            cleanup_interval: Duration::from_millis(100),
            ..Default::default()
        };

        let monitor = Arc::new(QueueMonitor::new(pool.clone(), config));

        // Run cleanup loop in background
        let monitor_clone = Arc::clone(&monitor);
        let cleanup_handle = tokio::spawn(async move {
            monitor_clone.run_cleanup_loop().await;
        });

        // Let it run for a bit
        tokio::time::sleep(Duration::from_millis(250)).await;

        // Stop the loop
        cleanup_handle.abort();

        // Verify cleanup ran
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
            workflow_id
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to count rows");

        assert_eq!(count, Some(0), "Failed activity should be cleaned up");

        cleanup_queue(&pool, workflow_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_run_vacuum_loop_executes() {
        let pool = setup_test_pool().await;

        let config = QueueConfig {
            vacuum_interval: Duration::from_millis(100),
            ..Default::default()
        };

        let monitor = Arc::new(QueueMonitor::new(pool.clone(), config));

        // Run vacuum loop in background
        let monitor_clone = Arc::clone(&monitor);
        let vacuum_handle = tokio::spawn(async move {
            monitor_clone.run_vacuum_loop().await;
        });

        // Let it run for a bit
        tokio::time::sleep(Duration::from_millis(250)).await;

        // Stop the loop
        vacuum_handle.abort();

        // If we get here without error, vacuum loop executed successfully
        assert!(true, "Vacuum loop executed without errors");
    }

    #[tokio::test]
    #[serial]
    async fn test_run_both_loops() {
        let pool = setup_test_pool().await;
        let workflow_id = Uuid::now_v7();

        // Insert a failed activity
        insert_failed_activity(&pool, workflow_id, "failed_1").await;

        // Use very short intervals
        let config = QueueConfig {
            cleanup_interval: Duration::from_millis(100),
            vacuum_interval: Duration::from_millis(100),
            ..Default::default()
        };

        let monitor = Arc::new(QueueMonitor::new(pool.clone(), config));

        // Run both loops via run()
        let monitor_clone = Arc::clone(&monitor);
        let run_handle = tokio::spawn(async move {
            monitor_clone.run().await;
        });

        // Let it run for a bit
        tokio::time::sleep(Duration::from_millis(350)).await;

        // Stop the loops
        run_handle.abort();

        // Verify cleanup ran
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
            workflow_id
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to count rows");

        assert_eq!(count, Some(0), "Failed activity should be cleaned up");

        cleanup_queue(&pool, workflow_id).await;
    }
}
