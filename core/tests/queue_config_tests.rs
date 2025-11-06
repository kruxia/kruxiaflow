// core/tests/queue_config_tests.rs
//! Unit tests for QueueConfig

use serial_test::serial;
use std::time::Duration;
use streamflow_core::queue::config::QueueConfig;

#[test]
fn test_queue_config_default() {
    let config = QueueConfig::default();

    assert_eq!(config.poll_interval, Duration::from_millis(100));
    assert_eq!(config.batch_size, 100);
    assert_eq!(config.default_timeout, Duration::from_secs(60));
    assert_eq!(config.default_max_retries, 3);
    assert_eq!(config.cleanup_interval, Duration::from_secs(60));
    assert_eq!(config.vacuum_interval, Duration::from_secs(300));
}

#[test]
fn test_queue_config_clone() {
    let config1 = QueueConfig::default();
    let config2 = config1.clone();

    assert_eq!(config1.poll_interval, config2.poll_interval);
    assert_eq!(config1.batch_size, config2.batch_size);
    assert_eq!(config1.default_timeout, config2.default_timeout);
    assert_eq!(config1.default_max_retries, config2.default_max_retries);
    assert_eq!(config1.cleanup_interval, config2.cleanup_interval);
    assert_eq!(config1.vacuum_interval, config2.vacuum_interval);
}

#[test]
#[serial]
fn test_queue_config_from_env_with_no_env_vars() {
    // Clear any existing env vars
    unsafe {
        std::env::remove_var("STREAMFLOW_QUEUE_POLL_INTERVAL");
        std::env::remove_var("STREAMFLOW_QUEUE_BATCH_SIZE");
        std::env::remove_var("STREAMFLOW_QUEUE_DEFAULT_TIMEOUT");
        std::env::remove_var("STREAMFLOW_QUEUE_DEFAULT_MAX_RETRIES");
        std::env::remove_var("STREAMFLOW_QUEUE_CLEANUP_INTERVAL");
        std::env::remove_var("STREAMFLOW_QUEUE_VACUUM_INTERVAL");
    }

    let config = QueueConfig::from_env();

    // Should use defaults when env vars not set
    assert_eq!(config.poll_interval, Duration::from_millis(100));
    assert_eq!(config.batch_size, 100);
    assert_eq!(config.default_timeout, Duration::from_secs(60));
    assert_eq!(config.default_max_retries, 3);
}

#[test]
#[serial]
fn test_queue_config_from_env_with_poll_interval_ms() {
    unsafe {
        std::env::set_var("STREAMFLOW_QUEUE_POLL_INTERVAL", "250ms");
    }

    let config = QueueConfig::from_env();

    assert_eq!(config.poll_interval, Duration::from_millis(250));

    unsafe {
        std::env::remove_var("STREAMFLOW_QUEUE_POLL_INTERVAL");
    }
}

#[test]
#[serial]
fn test_queue_config_from_env_with_poll_interval_seconds() {
    unsafe {
        std::env::set_var("STREAMFLOW_QUEUE_POLL_INTERVAL", "2s");
    }

    let config = QueueConfig::from_env();

    assert_eq!(config.poll_interval, Duration::from_secs(2));

    unsafe {
        std::env::remove_var("STREAMFLOW_QUEUE_POLL_INTERVAL");
    }
}

#[test]
#[serial]
fn test_queue_config_from_env_with_poll_interval_minutes() {
    unsafe {
        std::env::set_var("STREAMFLOW_QUEUE_POLL_INTERVAL", "5m");
    }

    let config = QueueConfig::from_env();

    assert_eq!(config.poll_interval, Duration::from_secs(300));

    unsafe {
        std::env::remove_var("STREAMFLOW_QUEUE_POLL_INTERVAL");
    }
}

#[test]
#[serial]
fn test_queue_config_from_env_with_batch_size() {
    unsafe {
        std::env::set_var("STREAMFLOW_QUEUE_BATCH_SIZE", "500");
    }

    let config = QueueConfig::from_env();

    assert_eq!(config.batch_size, 500);

    unsafe {
        std::env::remove_var("STREAMFLOW_QUEUE_BATCH_SIZE");
    }
}

#[test]
fn test_queue_config_custom_values() {
    let config = QueueConfig {
        poll_interval: Duration::from_millis(50),
        batch_size: 250,
        default_timeout: Duration::from_secs(120),
        default_max_retries: 5,
        cleanup_interval: Duration::from_secs(30),
        vacuum_interval: Duration::from_secs(600),
    };

    assert_eq!(config.poll_interval, Duration::from_millis(50));
    assert_eq!(config.batch_size, 250);
    assert_eq!(config.default_timeout, Duration::from_secs(120));
    assert_eq!(config.default_max_retries, 5);
    assert_eq!(config.cleanup_interval, Duration::from_secs(30));
    assert_eq!(config.vacuum_interval, Duration::from_secs(600));
}
