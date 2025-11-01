use super::error::{HealthCheckError, Result};
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

#[cfg(test)]
mod tests {
    // Unit tests would go here
    // For proper unit tests, we'd need to mock the database pool
    //
    // Integration tests for these functions are in:
    // tests/health_integration_tests.rs
}
