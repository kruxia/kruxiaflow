// core/tests/orchestrator_config_tests.rs
//! Unit tests for OrchestratorConfig

use sqlx::PgPool;
use std::time::Duration;
use streamflow_core::orchestrator::config::OrchestratorConfig;

async fn mock_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
    });
    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database")
}

#[tokio::test]
async fn test_orchestrator_config_new_defaults() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool);

    assert_eq!(config.poll_interval_min, Duration::from_millis(10));
    assert_eq!(config.poll_interval_max, Duration::from_millis(500));
    assert_eq!(config.backoff_multiplier, 1.3);
}

#[tokio::test]
async fn test_orchestrator_config_with_poll_interval() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(50), Duration::from_secs(10));

    assert_eq!(config.poll_interval_min, Duration::from_millis(50));
    assert_eq!(config.poll_interval_max, Duration::from_secs(10));
    assert_eq!(config.backoff_multiplier, 1.3); // Should remain default
}

#[tokio::test]
async fn test_orchestrator_config_with_backoff_multiplier() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool).with_backoff_multiplier(2.0);

    assert_eq!(config.poll_interval_min, Duration::from_millis(10)); // Should remain default
    assert_eq!(config.poll_interval_max, Duration::from_millis(500)); // Should remain default
    assert_eq!(config.backoff_multiplier, 2.0);
}

#[tokio::test]
async fn test_orchestrator_config_builder_chaining() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(20), Duration::from_secs(15))
        .with_backoff_multiplier(3.0);

    assert_eq!(config.poll_interval_min, Duration::from_millis(20));
    assert_eq!(config.poll_interval_max, Duration::from_secs(15));
    assert_eq!(config.backoff_multiplier, 3.0);
}

#[tokio::test]
async fn test_orchestrator_config_with_zero_min_interval() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(0), Duration::from_secs(5));

    assert_eq!(config.poll_interval_min, Duration::from_millis(0));
}

#[tokio::test]
async fn test_orchestrator_config_with_very_large_max_interval() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(10), Duration::from_secs(3600));

    assert_eq!(config.poll_interval_max, Duration::from_secs(3600));
}

#[tokio::test]
async fn test_orchestrator_config_with_fractional_multiplier() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool).with_backoff_multiplier(1.5);

    assert_eq!(config.backoff_multiplier, 1.5);
}

#[tokio::test]
async fn test_orchestrator_config_with_multiplier_one() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool).with_backoff_multiplier(1.0);

    assert_eq!(config.backoff_multiplier, 1.0);
}

#[tokio::test]
async fn test_orchestrator_config_multiple_builder_calls() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(5), Duration::from_secs(1))
        .with_backoff_multiplier(2.5)
        .with_poll_interval(Duration::from_millis(15), Duration::from_secs(3));

    // Last call should win
    assert_eq!(config.poll_interval_min, Duration::from_millis(15));
    assert_eq!(config.poll_interval_max, Duration::from_secs(3));
    assert_eq!(config.backoff_multiplier, 2.5);
}

#[tokio::test]
async fn test_orchestrator_config_backoff_default() {
    let pool = mock_pool().await;
    let config = OrchestratorConfig::new(pool);

    // Default uses optimized backoff multiplier
    assert!((config.backoff_multiplier - 1.3).abs() < 0.01);
}
