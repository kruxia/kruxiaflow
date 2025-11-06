//! Integration tests for the StreamFlow CLI
//!
//! These tests verify the command-line interface and main() flow.
//! Note: These tests do not start actual servers, but verify command parsing and validation.

use std::process::{Command, Output};

/// Helper to run the streamflow binary with arguments
fn run_streamflow(args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_streamflow"));
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("Failed to execute streamflow binary")
}

#[test]
fn test_cli_help() {
    // Test that --help flag works
    let output = run_streamflow(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("StreamFlow"));
    assert!(stdout.contains("workflow orchestration"));
    assert!(stdout.contains("api"));
    assert!(stdout.contains("--database-url"));
}

#[test]
fn test_cli_version() {
    // Test that --version flag works
    let output = run_streamflow(&["--version"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("streamflow"));
}

#[test]
fn test_api_command_help() {
    // Test that 'api --help' shows API-specific help
    let output = run_streamflow(&["api", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("API"));
    assert!(stdout.contains("--port"));
    assert!(stdout.contains("--bind"));
}

#[test]
fn test_api_command_missing_database_url() {
    // Test that API command fails without database URL
    // Clear DATABASE_URL to ensure test isolation
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("api")
        .env_remove("DATABASE_URL")
        .env_remove("STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM")
        .output()
        .expect("Failed to execute streamflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Database URL is required"));
}

#[test]
fn test_log_level_flag() {
    // Test that --log-level flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("--log-level")
        .arg("debug")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute streamflow binary");

    // Should succeed with --help even with --log-level
    assert!(output.status.success());
}

#[test]
fn test_log_format_flag() {
    // Test that --log-format flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("--log-format")
        .arg("json")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute streamflow binary");

    // Should succeed with --help even with --log-format
    assert!(output.status.success());
}

#[test]
fn test_invalid_command() {
    // Test that invalid command is rejected
    let output = run_streamflow(&["invalid-command"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    // Clap should provide an error message about invalid subcommand
    assert!(
        stderr.contains("invalid")
            || stderr.contains("unrecognized")
            || stderr.contains("unexpected")
    );
}

#[test]
fn test_database_url_via_cli() {
    // Test that --database-url flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute streamflow binary");

    // Should succeed with --help even with --database-url
    assert!(output.status.success());
}

#[test]
fn test_global_flags_before_subcommand() {
    // Test that global flags can appear before subcommand
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("--log-level")
        .arg("trace")
        .arg("--log-format")
        .arg("json")
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute streamflow binary");

    assert!(output.status.success());
}

#[test]
fn test_api_port_flag() {
    // Test that --port flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("api")
        .arg("--port")
        .arg("9090")
        .arg("--help")
        .output()
        .expect("Failed to execute streamflow binary");

    assert!(output.status.success());
}

#[test]
fn test_api_bind_flag() {
    // Test that --bind flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_streamflow"))
        .arg("api")
        .arg("--bind")
        .arg("127.0.0.1")
        .arg("--help")
        .output()
        .expect("Failed to execute streamflow binary");

    assert!(output.status.success());
}
