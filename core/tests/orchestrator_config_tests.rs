// core/tests/orchestrator_config_tests.rs
//! Unit tests for OrchestratorConfig

use kruxiaflow_core::orchestrator::config::OrchestratorConfig;
use sqlx::PgPool;
use std::time::Duration;

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_new_defaults(pool: PgPool) {
    let config = OrchestratorConfig::new(pool);

    assert_eq!(config.poll_interval_min, Duration::from_millis(50));
    assert_eq!(config.poll_interval_max, Duration::from_millis(1000));
    assert_eq!(config.backoff_multiplier, 1.5);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_poll_interval(pool: PgPool) {
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(100), Duration::from_secs(10));

    assert_eq!(config.poll_interval_min, Duration::from_millis(100));
    assert_eq!(config.poll_interval_max, Duration::from_secs(10));
    assert_eq!(config.backoff_multiplier, 1.5); // Should remain default
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_backoff_multiplier(pool: PgPool) {
    let config = OrchestratorConfig::new(pool).with_backoff_multiplier(2.0);

    assert_eq!(config.poll_interval_min, Duration::from_millis(50)); // Should remain default
    assert_eq!(config.poll_interval_max, Duration::from_millis(1000)); // Should remain default
    assert_eq!(config.backoff_multiplier, 2.0);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_builder_chaining(pool: PgPool) {
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(20), Duration::from_secs(15))
        .with_backoff_multiplier(3.0);

    assert_eq!(config.poll_interval_min, Duration::from_millis(20));
    assert_eq!(config.poll_interval_max, Duration::from_secs(15));
    assert_eq!(config.backoff_multiplier, 3.0);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_zero_min_interval(pool: PgPool) {
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(0), Duration::from_secs(5));

    assert_eq!(config.poll_interval_min, Duration::from_millis(0));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_very_large_max_interval(pool: PgPool) {
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(10), Duration::from_secs(3600));

    assert_eq!(config.poll_interval_max, Duration::from_secs(3600));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_fractional_multiplier(pool: PgPool) {
    let config = OrchestratorConfig::new(pool).with_backoff_multiplier(1.5);

    assert_eq!(config.backoff_multiplier, 1.5);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_multiplier_one(pool: PgPool) {
    let config = OrchestratorConfig::new(pool).with_backoff_multiplier(1.0);

    assert_eq!(config.backoff_multiplier, 1.0);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_multiple_builder_calls(pool: PgPool) {
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(5), Duration::from_secs(1))
        .with_backoff_multiplier(2.5)
        .with_poll_interval(Duration::from_millis(15), Duration::from_secs(3));

    // Last call should win
    assert_eq!(config.poll_interval_min, Duration::from_millis(15));
    assert_eq!(config.poll_interval_max, Duration::from_secs(3));
    assert_eq!(config.backoff_multiplier, 2.5);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_backoff_default(pool: PgPool) {
    let config = OrchestratorConfig::new(pool);

    // Default uses moderate backoff multiplier
    assert!((config.backoff_multiplier - 1.5).abs() < 0.01);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_workflow_timeout(pool: PgPool) {
    let config = OrchestratorConfig::new(pool).with_workflow_timeout(Duration::from_secs(600));

    assert_eq!(config.workflow_timeout, Duration::from_secs(600));
    // Other defaults should remain unchanged
    assert_eq!(config.poll_interval_min, Duration::from_millis(50));
    assert_eq!(config.timeout_check_interval, Duration::from_secs(30));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_with_timeout_check_interval(pool: PgPool) {
    let config = OrchestratorConfig::new(pool).with_timeout_check_interval(Duration::from_secs(60));

    assert_eq!(config.timeout_check_interval, Duration::from_secs(60));
    // Other defaults should remain unchanged
    assert_eq!(config.workflow_timeout, Duration::from_secs(300));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_full_builder_chain(pool: PgPool) {
    let config = OrchestratorConfig::new(pool)
        .with_poll_interval(Duration::from_millis(5), Duration::from_secs(2))
        .with_backoff_multiplier(1.5)
        .with_workflow_timeout(Duration::from_secs(900))
        .with_timeout_check_interval(Duration::from_secs(45));

    assert_eq!(config.poll_interval_min, Duration::from_millis(5));
    assert_eq!(config.poll_interval_max, Duration::from_secs(2));
    assert_eq!(config.backoff_multiplier, 1.5);
    assert_eq!(config.workflow_timeout, Duration::from_secs(900));
    assert_eq!(config.timeout_check_interval, Duration::from_secs(45));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_config_default_timeouts(pool: PgPool) {
    let config = OrchestratorConfig::new(pool);

    // Verify default timeout values
    assert_eq!(config.workflow_timeout, Duration::from_secs(300)); // 5 minutes
    assert_eq!(config.timeout_check_interval, Duration::from_secs(30));
}
