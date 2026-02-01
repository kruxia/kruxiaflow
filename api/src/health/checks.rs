use super::error::{HealthCheckError, Result};
use super::responses::PoolMetricsResponse;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::timeout;

/// Check database health by executing a simple query
///
/// Executes `SELECT 1` to verify database connectivity. Times out after 5 seconds.
///
/// # Arguments
/// * `pool` - PostgreSQL connection pool
///
/// # Returns
/// * `Ok(())` if database is healthy
/// * `Err(HealthCheckError)` if check fails or times out
pub async fn check_database_health(pool: &PgPool) -> Result<()> {
    // Simple query to verify connectivity with 5 second timeout
    let result = timeout(
        Duration::from_secs(5),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
    )
    .await;

    match result {
        Ok(Ok(1)) => Ok(()),
        Ok(Ok(_)) => Err(HealthCheckError::UnexpectedResult),
        Ok(Err(e)) => Err(HealthCheckError::DatabaseError(e)),
        Err(_) => Err(HealthCheckError::Timeout),
    }
}

/// Check event source health
///
/// For MVP (PostgreSQL-based EventSource), this delegates to database health check.
/// Future implementations (Kafka, NATS) would check broker connectivity.
///
/// # Arguments
/// * `pool` - PostgreSQL connection pool (used by PostgresEventSource)
///
/// # Returns
/// * `Ok(())` if event source is healthy
/// * `Err(HealthCheckError)` if check fails
pub async fn check_event_source_health(pool: &PgPool) -> Result<()> {
    // For MVP with PostgresEventSource, verify database connectivity
    // Future: Add health_check() method to EventSource trait
    check_database_health(pool).await.map_err(|e| match e {
        HealthCheckError::DatabaseError(db_err) => {
            HealthCheckError::EventSourceError(db_err.to_string())
        }
        HealthCheckError::Timeout => HealthCheckError::EventSourceError("timeout".to_string()),
        _ => HealthCheckError::EventSourceError(e.to_string()),
    })
}

/// Check activity queue health
///
/// For MVP (PostgreSQL-based ActivityQueue), this verifies queue table accessibility.
/// Future implementations (SQS, RabbitMQ) would check queue service connectivity.
///
/// # Arguments
/// * `pool` - PostgreSQL connection pool (used by PostgresQueue)
///
/// # Returns
/// * `Ok(())` if activity queue is healthy
/// * `Err(HealthCheckError)` if check fails
pub async fn check_activity_queue_health(pool: &PgPool) -> Result<()> {
    // For MVP with PostgresQueue, verify queue table accessibility
    // Use lightweight query that doesn't require reading actual data
    let result = timeout(
        Duration::from_secs(5),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM activity_queue LIMIT 1").fetch_one(pool),
    )
    .await;

    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(HealthCheckError::QueueError(e.to_string())),
        Err(_) => Err(HealthCheckError::QueueError("timeout".to_string())),
    }
}

/// Get connection pool metrics
///
/// Returns current connection pool statistics including size, active/idle connections,
/// and utilization percentage.
///
/// # Arguments
/// * `pool` - PostgreSQL connection pool
///
/// # Returns
/// * `PoolMetricsResponse` with current pool statistics
pub fn get_pool_metrics(pool: &PgPool) -> PoolMetricsResponse {
    let size = pool.size();
    let idle = pool.num_idle() as u32;
    let active = size - idle;
    let max_connections = pool.options().get_max_connections();

    let utilization_percent = if max_connections > 0 {
        (active as f64 / max_connections as f64) * 100.0
    } else {
        0.0
    };

    // Determine health status based on utilization
    let status = if utilization_percent > 90.0 {
        "critical"
    } else if utilization_percent > 80.0 {
        "warning"
    } else {
        "healthy"
    };

    PoolMetricsResponse {
        size,
        idle,
        active,
        max_connections,
        utilization_percent,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });
        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    #[tokio::test]
    async fn test_check_database_health_success() {
        let pool = test_pool().await;
        let result = check_database_health(&pool).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_check_event_source_health_success() {
        let pool = test_pool().await;
        let result = check_event_source_health(&pool).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_check_activity_queue_health_success() {
        let pool = test_pool().await;
        let result = check_activity_queue_health(&pool).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_pool_metrics_returns_valid_data() {
        let pool = test_pool().await;
        let metrics = get_pool_metrics(&pool);
        assert!(metrics.max_connections > 0);
        assert!(metrics.utilization_percent >= 0.0);
        assert!(metrics.utilization_percent <= 100.0);
        assert!(
            metrics.status == "healthy"
                || metrics.status == "warning"
                || metrics.status == "critical"
        );
    }

    #[tokio::test]
    async fn test_get_pool_metrics_size_consistency() {
        let pool = test_pool().await;
        let metrics = get_pool_metrics(&pool);
        assert_eq!(metrics.active + metrics.idle, metrics.size);
    }

    #[test]
    fn test_health_check_error_display() {
        let err = HealthCheckError::Timeout;
        assert_eq!(err.to_string(), "Health check timeout");

        let err = HealthCheckError::UnexpectedResult;
        assert_eq!(err.to_string(), "Unexpected result from health check");

        let err = HealthCheckError::EventSourceError("conn refused".to_string());
        assert!(err.to_string().contains("conn refused"));

        let err = HealthCheckError::QueueError("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
    }
}
