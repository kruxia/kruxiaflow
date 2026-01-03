//! Integration tests for the Kruxia Flow CLI
//!
//! These tests verify the command-line interface and main() flow.
//! Note: These tests do not start actual servers, but verify command parsing and validation.

use std::process::{Command, Output};

/// Helper to run the kruxiaflow binary with arguments
fn run_kruxiaflow(args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"));
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("Failed to execute kruxiaflow binary")
}

#[test]
fn test_cli_help() {
    // Test that --help flag works
    let output = run_kruxiaflow(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Kruxia Flow"));
    assert!(stdout.contains("workflow orchestration"));
    assert!(stdout.contains("api"));
    assert!(stdout.contains("--database-url"));
}

#[test]
fn test_cli_version() {
    // Test that --version flag works
    let output = run_kruxiaflow(&["--version"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("kruxiaflow"));
}

#[test]
fn test_api_command_help() {
    // Test that 'api --help' shows API-specific help
    let output = run_kruxiaflow(&["api", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("API"));
    assert!(stdout.contains("--port"));
    assert!(stdout.contains("--bind"));
}

#[test]
fn test_api_command_missing_database_url() {
    // Test that API command fails without database URL
    // Clear DATABASE_URL to ensure test isolation
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("api")
        .env_remove("DATABASE_URL")
        .env_remove("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Database URL is required"));
}

#[test]
fn test_log_level_flag() {
    // Test that --log-level flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--log-level")
        .arg("debug")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // Should succeed with --help even with --log-level
    assert!(output.status.success());
}

#[test]
fn test_log_format_flag() {
    // Test that --log-format flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--log-format")
        .arg("json")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // Should succeed with --help even with --log-format
    assert!(output.status.success());
}

#[test]
fn test_invalid_command() {
    // Test that invalid command is rejected
    let output = run_kruxiaflow(&["invalid-command"]);
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
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // Should succeed with --help even with --database-url
    assert!(output.status.success());
}

#[test]
fn test_global_flags_before_subcommand() {
    // Test that global flags can appear before subcommand
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--log-level")
        .arg("trace")
        .arg("--log-format")
        .arg("json")
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("api")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_api_port_flag() {
    // Test that --port flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("api")
        .arg("--port")
        .arg("9090")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_api_bind_flag() {
    // Test that --bind flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("api")
        .arg("--bind")
        .arg("127.0.0.1")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

// =========================================================================
// Migrate command tests
// =========================================================================

#[test]
fn test_migrate_command_help() {
    // Test that 'migrate --help' shows migrate-specific help
    let output = run_kruxiaflow(&["migrate", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("migration"));
    assert!(stdout.contains("--status"));
    assert!(stdout.contains("--dry-run"));
}

#[test]
fn test_migrate_command_missing_database_url() {
    // Test that migrate command fails without database URL
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("migrate")
        .env_remove("DATABASE_URL")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Database URL is required"));
}

#[test]
fn test_migrate_status_flag_accepted() {
    // Test that --status flag is accepted (will fail on connection, but parsing works)
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("migrate")
        .arg("--status")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // --help should succeed even with --status
    assert!(output.status.success());
}

#[test]
fn test_migrate_dry_run_flag_accepted() {
    // Test that --dry-run flag is accepted (will fail on connection, but parsing works)
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("migrate")
        .arg("--dry-run")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    // --help should succeed even with --dry-run
    assert!(output.status.success());
}

#[test]
fn test_migrate_with_database_url_flag() {
    // Test that --database-url flag works with migrate command
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("migrate")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

// =========================================================================
// Seed-client command tests
// =========================================================================

#[test]
fn test_seed_client_command_help() {
    // Test that 'seed-client --help' shows seed-client-specific help
    let output = run_kruxiaflow(&["seed-client", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("OAuth"));
    assert!(stdout.contains("--client-id"));
    assert!(stdout.contains("--client-secret"));
    assert!(stdout.contains("--force"));
}

#[test]
fn test_seed_client_command_missing_database_url() {
    // Test that seed-client command fails without database URL
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("seed-client")
        .env_remove("DATABASE_URL")
        .env_remove("KRUXIAFLOW_CLIENT_SECRET")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Database URL is required"));
}

#[test]
fn test_seed_client_force_flag_accepted() {
    // Test that --force flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("seed-client")
        .arg("--force")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_seed_client_client_id_flag_accepted() {
    // Test that --client-id flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("seed-client")
        .arg("--client-id")
        .arg("my-custom-client")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_seed_client_client_secret_flag_accepted() {
    // Test that --client-secret flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("seed-client")
        .arg("--client-secret")
        .arg("my-secret")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_seed_client_with_database_url_flag() {
    // Test that --database-url flag works with seed-client command
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("--database-url")
        .arg("postgres://localhost/test")
        .arg("seed-client")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

// =========================================================================
// Serve command startup flag tests
// =========================================================================

#[test]
fn test_serve_migrate_flag_accepted() {
    // Test that --migrate flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("serve")
        .arg("--migrate")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_serve_seed_client_flag_accepted() {
    // Test that --seed-client flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("serve")
        .arg("--seed-client")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_serve_db_connect_timeout_flag_accepted() {
    // Test that --db-connect-timeout flag is accepted
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("serve")
        .arg("--db-connect-timeout")
        .arg("120")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_serve_combined_startup_flags() {
    // Test that --migrate and --seed-client can be combined
    let output = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .arg("serve")
        .arg("--migrate")
        .arg("--seed-client")
        .arg("--db-connect-timeout")
        .arg("90")
        .arg("--help")
        .output()
        .expect("Failed to execute kruxiaflow binary");

    assert!(output.status.success());
}

#[test]
fn test_serve_help_shows_startup_flags() {
    // Test that serve --help shows the startup flags
    let output = run_kruxiaflow(&["serve", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("--migrate"));
    assert!(stdout.contains("--seed-client"));
    assert!(stdout.contains("--db-connect-timeout"));
}
