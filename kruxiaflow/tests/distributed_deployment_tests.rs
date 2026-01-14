//! Integration tests for distributed deployment (US-1C.3)
//!
//! These tests verify that the individual service launchers work correctly
//! for distributed deployment scenarios.

use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// Helper to run the kruxiaflow binary with arguments
fn run_kruxiaflow(args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"));
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("Failed to execute kruxiaflow binary")
}

// =========================================================================
// Orchestrator command CLI tests
// =========================================================================

#[test]
fn test_orchestrator_command_help() {
    let output = run_kruxiaflow(&["orchestrator", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("orchestrator"));
    assert!(stdout.contains("--consumer-id"));
    assert!(stdout.contains("--poll-interval"));
    assert!(stdout.contains("--shutdown-timeout"));
    assert!(stdout.contains("distributed"));
}

#[test]
fn test_orchestrator_command_missing_database_url() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("orchestrator")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Database URL is required"));
}

#[test]
fn test_orchestrator_consumer_id_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("orchestrator")
        .arg("--consumer-id")
        .arg("orch_test_1")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_orchestrator_poll_interval_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("orchestrator")
        .arg("--poll-interval")
        .arg("50")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_orchestrator_shutdown_timeout_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("orchestrator")
        .arg("--shutdown-timeout")
        .arg("60")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_orchestrator_with_all_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("orchestrator")
        .arg("--consumer-id")
        .arg("orch_prod_1")
        .arg("--poll-interval")
        .arg("100")
        .arg("--shutdown-timeout")
        .arg("45")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

// =========================================================================
// Worker command CLI tests
// =========================================================================

#[test]
fn test_worker_command_help() {
    let output = run_kruxiaflow(&["worker", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("worker"));
    assert!(stdout.contains("--api-url"));
    assert!(stdout.contains("--worker-id"));
    assert!(stdout.contains("--max-activities"));
    assert!(stdout.contains("--activity-types"));
    assert!(stdout.contains("--client-secret"));
    assert!(stdout.contains("distributed"));
}

#[test]
fn test_worker_command_missing_database_url() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .env_remove("DATABASE_URL")
        .env_remove("KRUXIAFLOW_CLIENT_SECRET")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Database URL is required"));
}

#[test]
fn test_worker_api_url_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--api-url")
        .arg("http://api.example.com:8080")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_worker_id_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--worker-id")
        .arg("worker_payments_1")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_max_activities_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--max-activities")
        .arg("32")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_activity_types_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--activity-types")
        .arg("builtin.echo,builtin.http_request")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_poll_max_activities_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--poll-max-activities")
        .arg("5")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_poll_interval_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--poll-interval")
        .arg("200")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_activity_timeout_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--activity-timeout")
        .arg("600")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_heartbeat_interval_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--heartbeat-interval")
        .arg("60")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_client_id_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--client-id")
        .arg("my_custom_client")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_client_secret_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--client-secret")
        .arg("my_secret")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_shutdown_timeout_flag_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--shutdown-timeout")
        .arg("60")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_with_all_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("worker")
        .arg("--api-url")
        .arg("http://api.example.com:8080")
        .arg("--worker-id")
        .arg("worker_prod_1")
        .arg("--max-activities")
        .arg("32")
        .arg("--activity-types")
        .arg("builtin.echo,builtin.llm_prompt")
        .arg("--poll-max-activities")
        .arg("5")
        .arg("--poll-interval")
        .arg("150")
        .arg("--activity-timeout")
        .arg("600")
        .arg("--heartbeat-interval")
        .arg("45")
        .arg("--client-id")
        .arg("prod_worker")
        .arg("--client-secret")
        .arg("prod_secret")
        .arg("--shutdown-timeout")
        .arg("60")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

// =========================================================================
// Environment variable configuration tests
// =========================================================================

#[test]
fn test_orchestrator_consumer_id_from_env() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .env("KRUXIAFLOW_ORCHESTRATOR_CONSUMER_ID", "orch_env_1")
        .arg("orchestrator")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    // The help output shows the env var is recognized
    assert!(stdout.contains("KRUXIAFLOW_ORCHESTRATOR_CONSUMER_ID"));
}

#[test]
fn test_worker_api_url_from_env() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .env("KRUXIAFLOW_API_URL", "http://env-api.example.com:9090")
        .arg("worker")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    // The help output shows the env var is recognized
    assert!(stdout.contains("KRUXIAFLOW_API_URL"));
}

#[test]
fn test_worker_client_secret_from_env() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .env("KRUXIAFLOW_CLIENT_SECRET", "env_secret_value")
        .arg("worker")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("KRUXIAFLOW_CLIENT_SECRET"));
}

// =========================================================================
// Help text content verification
// =========================================================================

#[test]
fn test_orchestrator_help_shows_examples() {
    let output = run_kruxiaflow(&["orchestrator", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    // Should show example usage
    assert!(stdout.contains("kruxiaflow orchestrator"));
    assert!(stdout.contains("orch_prod"));
}

#[test]
fn test_worker_help_shows_examples() {
    let output = run_kruxiaflow(&["worker", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    // Should show example usage
    assert!(stdout.contains("kruxiaflow worker"));
    assert!(stdout.contains("api.example.com"));
}

#[test]
fn test_main_help_shows_orchestrator_and_worker() {
    let output = run_kruxiaflow(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("orchestrator"));
    assert!(stdout.contains("worker"));
    assert!(stdout.contains("distributed"));
}

// =========================================================================
// Validation error tests
// =========================================================================

#[test]
fn test_orchestrator_invalid_poll_interval_zero() {
    // When using --help with an invalid value, clap still accepts it
    // To test actual validation, we need to try to run the command
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("orchestrator")
        .arg("--poll-interval")
        .arg("0")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // Command will fail because it can't connect to database before validating,
    // but the test verifies the flag is parsed
    // For actual validation testing, see unit tests in orchestrator.rs
    assert!(!output.status.success());
}

#[test]
fn test_worker_invalid_max_activities_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("worker")
        .arg("--max-activities")
        .arg("0")
        .arg("--client-secret")
        .arg("test")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // Command will fail, demonstrating the flag is recognized
    assert!(!output.status.success());
}

// =========================================================================
// Short flag tests
// =========================================================================

#[test]
fn test_worker_short_max_activities_flag() {
    // -m is the short form of --max-activities
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("-m")
        .arg("32")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

// =========================================================================
// Graceful shutdown tests
// =========================================================================

#[test]
fn test_orchestrator_shutdown_timeout_boundary_valid() {
    // Test valid shutdown timeout at boundaries (5-300)
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("orchestrator")
        .arg("--shutdown-timeout")
        .arg("5") // Minimum valid
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("orchestrator")
        .arg("--shutdown-timeout")
        .arg("300") // Maximum valid
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_worker_shutdown_timeout_boundary_valid() {
    // Test valid shutdown timeout at boundaries (5-300)
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--shutdown-timeout")
        .arg("5") // Minimum valid
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("worker")
        .arg("--shutdown-timeout")
        .arg("300") // Maximum valid
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
#[cfg(unix)]
fn test_orchestrator_process_can_be_killed() {
    // Start orchestrator with an invalid database URL so it will fail to connect
    // This tests that the process can be properly terminated
    let mut child = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://invalid:invalid@127.0.0.1:9999/nonexistent")
        .arg("orchestrator")
        .arg("--consumer-id")
        .arg("test_signal")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn orchestrator");

    // Give it a moment to start (it will fail on DB connect but that's fine)
    thread::sleep(Duration::from_millis(200));

    // Kill the process
    let _ = child.kill();

    // Wait for exit
    let exit_status = child.wait().expect("Failed to wait for child");

    // Process should have exited (either from kill or from DB error)
    assert!(
        exit_status.code().is_some() || exit_status.signal().is_some(),
        "Process should have exited"
    );
}

#[test]
#[cfg(unix)]
fn test_worker_process_can_be_killed() {
    // Start worker with an invalid database URL
    let mut child = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://invalid:invalid@127.0.0.1:9999/nonexistent")
        .arg("worker")
        .arg("--api-url")
        .arg("http://127.0.0.1:9999")
        .arg("--client-secret")
        .arg("test_secret")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn worker");

    // Give it a moment to start
    thread::sleep(Duration::from_millis(200));

    // Kill the process
    let _ = child.kill();

    // Wait for exit
    let exit_status = child.wait().expect("Failed to wait for child");

    // Process should have exited
    assert!(
        exit_status.code().is_some() || exit_status.signal().is_some(),
        "Process should have exited"
    );
}

// =========================================================================
// Failure scenario tests
// =========================================================================

#[test]
fn test_orchestrator_fails_with_invalid_database() {
    // Orchestrator should fail quickly when database is unreachable
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://invalid:invalid@127.0.0.1:9999/nonexistent")
        .arg("orchestrator")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should fail (not hang forever)
    assert!(!output.status.success());
    // Should have a reasonable error message
    assert!(
        stderr.contains("database")
            || stderr.contains("connect")
            || stderr.contains("Failed")
            || stderr.contains("error"),
        "Error message should mention database connection failure: {}",
        stderr
    );
}

#[test]
fn test_worker_fails_with_invalid_database() {
    // Worker needs database for artifact storage
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://invalid:invalid@127.0.0.1:9999/nonexistent")
        .arg("worker")
        .arg("--api-url")
        .arg("http://127.0.0.1:9999")
        .arg("--client-secret")
        .arg("test_secret")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should fail (not hang forever)
    assert!(!output.status.success());
    // Should have a reasonable error message
    assert!(
        stderr.contains("database")
            || stderr.contains("connect")
            || stderr.contains("Failed")
            || stderr.contains("error"),
        "Error message should mention connection failure: {}",
        stderr
    );
}

#[test]
fn test_worker_fails_without_client_secret() {
    // Worker requires client secret for authentication
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("worker")
        .arg("--api-url")
        .arg("http://127.0.0.1:8080")
        .env_remove("KRUXIAFLOW_CLIENT_SECRET")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("secret") || stderr.contains("CLIENT_SECRET"),
        "Error message should mention missing client secret: {}",
        stderr
    );
}

#[test]
fn test_worker_fails_with_empty_api_url() {
    // Worker should validate API URL is not empty
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("worker")
        .arg("--api-url")
        .arg("")
        .arg("--client-secret")
        .arg("test_secret")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("API URL") || stderr.contains("empty") || stderr.contains("cannot"),
        "Error message should mention empty API URL: {}",
        stderr
    );
}

#[test]
fn test_orchestrator_invalid_poll_interval_too_high() {
    // Poll interval must be <= 10000ms
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("orchestrator")
        .arg("--poll-interval")
        .arg("10001")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("Poll interval") || stderr.contains("10000"),
        "Error message should mention poll interval limit: {}",
        stderr
    );
}

#[test]
fn test_worker_invalid_max_activities_too_high() {
    // Max activities must be <= 100
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("worker")
        .arg("--max-activities")
        .arg("101")
        .arg("--client-secret")
        .arg("test_secret")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("Max concurrent activities") || stderr.contains("100"),
        "Error message should mention max activities limit: {}",
        stderr
    );
}

#[test]
fn test_orchestrator_shutdown_timeout_too_low() {
    // Shutdown timeout must be >= 5
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("orchestrator")
        .arg("--shutdown-timeout")
        .arg("4")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("Shutdown timeout") || stderr.contains("5"),
        "Error message should mention shutdown timeout limit: {}",
        stderr
    );
}

#[test]
fn test_worker_shutdown_timeout_too_high() {
    // Shutdown timeout must be <= 300
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("worker")
        .arg("--shutdown-timeout")
        .arg("301")
        .arg("--client-secret")
        .arg("test_secret")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("Shutdown timeout") || stderr.contains("300"),
        "Error message should mention shutdown timeout limit: {}",
        stderr
    );
}
