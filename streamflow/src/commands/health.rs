use anyhow::Result;
use clap::Args;
use std::time::Duration;

/// Health check command for container health probes
///
/// Performs an HTTP GET request to the health endpoint and returns
/// appropriate exit codes for Docker/Kubernetes health checks.
///
/// Exit codes:
/// - 0: Service is healthy (200 OK)
/// - 1: Service is unhealthy or unreachable
#[derive(Args)]
pub struct HealthCommand {
    /// Health endpoint URL to check
    #[arg(
        long,
        default_value = "http://localhost:8080/health",
        help = "URL of the health endpoint to check"
    )]
    pub url: String,

    /// Request timeout in seconds
    #[arg(
        long,
        default_value = "5",
        help = "Timeout for health check request in seconds"
    )]
    pub timeout: u64,
}

/// Execute health check and return appropriate exit code
///
/// This function is designed for container health checks:
/// - Minimal output to avoid log spam
/// - Fast execution with configurable timeout
/// - Clear exit codes (0 = healthy, 1 = unhealthy)
///
/// Uses std::process::exit() to ensure proper exit codes for container orchestrators.
pub async fn execute(cmd: HealthCommand) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cmd.timeout))
        .build()?;

    match client.get(&cmd.url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                // Minimal output for successful health check
                println!("ok");
                Ok(())
            } else {
                eprintln!("unhealthy: status {}", response.status());
                std::process::exit(1);
            }
        }
        Err(e) => {
            if e.is_timeout() {
                eprintln!("unhealthy: timeout");
            } else if e.is_connect() {
                eprintln!("unhealthy: connection refused");
            } else {
                eprintln!("unhealthy: {}", e);
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_command_defaults() {
        // Test that default values are set correctly
        let cmd = HealthCommand {
            url: "http://localhost:8080/health".to_string(),
            timeout: 5,
        };
        assert_eq!(cmd.url, "http://localhost:8080/health");
        assert_eq!(cmd.timeout, 5);
    }

    #[test]
    fn test_health_command_custom_values() {
        let cmd = HealthCommand {
            url: "http://example.com:9090/health".to_string(),
            timeout: 10,
        };
        assert_eq!(cmd.url, "http://example.com:9090/health");
        assert_eq!(cmd.timeout, 10);
    }
}
