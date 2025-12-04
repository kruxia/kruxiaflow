use anyhow::Result;
use clap::Args;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Health command - Check service health
///
/// Performs health checks via the API server's health endpoints.
/// Exit code: 0 (healthy), 1 (unhealthy).
#[derive(Args)]
pub struct HealthCommand {
    /// API server URL to check
    #[arg(
        long,
        env = "STREAMFLOW_API_URL",
        default_value = "http://127.0.0.1:8080",
        help = "StreamFlow API server URL"
    )]
    pub api_url: String,

    /// Health check timeout in seconds
    #[arg(
        short,
        long,
        env = "STREAMFLOW_HEALTH_TIMEOUT",
        default_value = "5",
        help = "Timeout for health checks in seconds"
    )]
    pub timeout: u64,

    /// Output format (text or json)
    #[arg(
        short,
        long,
        env = "STREAMFLOW_OUTPUT_FORMAT",
        default_value = "text",
        help = "Output format (text, json)"
    )]
    pub format: String,

    /// Check specific service only
    #[arg(long, help = "Check specific service (api, database, orchestrator)")]
    pub service: Option<String>,

    /// Verbose output (show response details)
    #[arg(short, long, help = "Show detailed health check results")]
    pub verbose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResult {
    pub service: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Degraded,
    Unknown,
}

impl HealthStatus {
    pub fn symbol(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "✅",
            HealthStatus::Unhealthy => "❌",
            HealthStatus::Degraded => "⚠️",
            HealthStatus::Unknown => "❓",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub overall_status: HealthStatus,
    pub services: Vec<HealthResult>,
    pub timestamp: String,
}

/// Execute health command
///
/// All health checks go through the API server's health endpoints.
/// This ensures we use the service interfaces rather than direct database access.
pub async fn execute(cmd: HealthCommand) -> Result<()> {
    let timeout = Duration::from_secs(cmd.timeout);
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_default();

    // First, check if API is reachable and get readiness status
    let (api_result, readiness_response) = check_api_and_readiness(&client, &cmd.api_url).await;

    // Build results based on what was requested
    let mut results = Vec::new();
    let check_all = cmd.service.is_none();
    let service = cmd.service.as_deref();

    // API health
    if check_all || service == Some("api") {
        results.push(api_result.clone());
    }

    // Parse readiness response for other services
    if let Some(ref readiness) = readiness_response {
        // Database health (from readiness checks)
        if check_all || service == Some("database") {
            results.push(extract_service_health("database", readiness));
        }

        // Orchestrator health (from readiness checks)
        if check_all || service == Some("orchestrator") {
            results.push(extract_service_health("orchestrator", readiness));
        }
    } else if api_result.status == HealthStatus::Unhealthy {
        // API is down, so we can't check other services
        if check_all || service == Some("database") {
            results.push(HealthResult {
                service: "database".to_string(),
                status: HealthStatus::Unknown,
                message: Some("API server unreachable".to_string()),
                latency_ms: None,
                details: None,
            });
        }
        if check_all || service == Some("orchestrator") {
            results.push(HealthResult {
                service: "orchestrator".to_string(),
                status: HealthStatus::Unknown,
                message: Some("API server unreachable".to_string()),
                latency_ms: None,
                details: None,
            });
        }
    }

    // Determine overall status
    let overall_status = if results.iter().all(|r| r.status == HealthStatus::Healthy) {
        HealthStatus::Healthy
    } else if results.iter().any(|r| r.status == HealthStatus::Unhealthy) {
        HealthStatus::Unhealthy
    } else {
        HealthStatus::Degraded
    };

    let report = HealthReport {
        overall_status,
        services: results,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    // Output results
    match cmd.format.as_str() {
        "json" => print_json_report(&report),
        _ => print_text_report(&report, cmd.verbose),
    }

    // Exit with appropriate code
    if overall_status == HealthStatus::Healthy {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

/// Check API health and get readiness response
async fn check_api_and_readiness(
    client: &Client,
    api_url: &str,
) -> (HealthResult, Option<serde_json::Value>) {
    let start = std::time::Instant::now();
    let health_url = format!("{}/health", api_url.trim_end_matches('/'));

    // Check basic health endpoint
    let api_result = match client.get(&health_url).send().await {
        Ok(response) => {
            let status_code = response.status();
            let body = response.json::<serde_json::Value>().await.ok();

            if status_code.is_success() {
                HealthResult {
                    service: "api".to_string(),
                    status: HealthStatus::Healthy,
                    message: Some(format!("HTTP {}", status_code)),
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    details: body,
                }
            } else {
                HealthResult {
                    service: "api".to_string(),
                    status: HealthStatus::Unhealthy,
                    message: Some(format!("HTTP {}", status_code)),
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    details: body,
                }
            }
        }
        Err(e) => HealthResult {
            service: "api".to_string(),
            status: HealthStatus::Unhealthy,
            message: Some(format!("Request failed: {}", e)),
            latency_ms: Some(start.elapsed().as_millis() as u64),
            details: None,
        },
    };

    // If API is healthy, get readiness details
    let readiness_response = if api_result.status == HealthStatus::Healthy {
        let ready_url = format!("{}/health/ready", api_url.trim_end_matches('/'));
        match client.get(&ready_url).send().await {
            Ok(response) => response.json::<serde_json::Value>().await.ok(),
            Err(_) => None,
        }
    } else {
        None
    };

    (api_result, readiness_response)
}

/// Extract service health from readiness response
fn extract_service_health(service_name: &str, readiness: &serde_json::Value) -> HealthResult {
    let check = readiness.get("checks").and_then(|c| c.get(service_name));

    match check {
        Some(service_check) => {
            let status_str = service_check
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");

            let status = match status_str {
                "healthy" => HealthStatus::Healthy,
                "unhealthy" => HealthStatus::Unhealthy,
                "degraded" => HealthStatus::Degraded,
                _ => HealthStatus::Unknown,
            };

            let message = service_check
                .get("message")
                .and_then(|m| m.as_str())
                .map(String::from)
                .or_else(|| Some(status_str.to_string()));

            HealthResult {
                service: service_name.to_string(),
                status,
                message,
                latency_ms: None,
                details: Some(service_check.clone()),
            }
        }
        None => HealthResult {
            service: service_name.to_string(),
            status: HealthStatus::Unknown,
            message: Some("Not reported in readiness check".to_string()),
            latency_ms: None,
            details: None,
        },
    }
}

/// Print text report
fn print_text_report(report: &HealthReport, verbose: bool) {
    println!("StreamFlow Health Check");
    println!("{:-<50}", "");

    for result in &report.services {
        println!(
            "{} {:12} - {}",
            result.status.symbol(),
            result.service,
            result.message.as_deref().unwrap_or("No details")
        );

        if let Some(latency) = result.latency_ms {
            println!("   Latency: {}ms", latency);
        }

        if verbose {
            if let Some(ref details) = result.details {
                println!(
                    "   Details: {}",
                    serde_json::to_string_pretty(details).unwrap_or_default()
                );
            }
        }
    }

    println!("{:-<50}", "");
    println!(
        "Overall: {} {}",
        report.overall_status.symbol(),
        format!("{:?}", report.overall_status).to_uppercase()
    );
}

/// Print JSON report
fn print_json_report(report: &HealthReport) {
    println!("{}", serde_json::to_string_pretty(report).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_symbol() {
        assert_eq!(HealthStatus::Healthy.symbol(), "✅");
        assert_eq!(HealthStatus::Unhealthy.symbol(), "❌");
        assert_eq!(HealthStatus::Degraded.symbol(), "⚠️");
        assert_eq!(HealthStatus::Unknown.symbol(), "❓");
    }

    #[test]
    fn test_health_command_defaults() {
        let cmd = HealthCommand {
            api_url: "http://127.0.0.1:8080".to_string(),
            timeout: 5,
            format: "text".to_string(),
            service: None,
            verbose: false,
        };

        assert_eq!(cmd.timeout, 5);
        assert_eq!(cmd.format, "text");
        assert!(cmd.service.is_none());
    }

    #[test]
    fn test_health_result_serialization() {
        let result = HealthResult {
            service: "test".to_string(),
            status: HealthStatus::Healthy,
            message: Some("OK".to_string()),
            latency_ms: Some(10),
            details: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("healthy"));
    }

    #[test]
    fn test_extract_service_health_found() {
        let readiness = serde_json::json!({
            "checks": {
                "database": {
                    "status": "healthy",
                    "message": "Connected"
                }
            }
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.service, "database");
    }

    #[test]
    fn test_extract_service_health_not_found() {
        let readiness = serde_json::json!({
            "checks": {}
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Unknown);
    }
}
