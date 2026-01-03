use anyhow::Result;
use clap::Args;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Status command - Show detailed service status
///
/// All status information is retrieved via the API server's endpoints.
/// This ensures we use the service interfaces rather than direct database access.
#[derive(Args)]
pub struct StatusCommand {
    /// API server URL
    #[arg(
        long,
        env = "KRUXIAFLOW_API_URL",
        default_value = "http://127.0.0.1:8080",
        help = "Kruxia Flow API server URL"
    )]
    pub api_url: String,

    /// Status check timeout in seconds
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_STATUS_TIMEOUT",
        default_value = "10",
        help = "Timeout for status queries in seconds"
    )]
    pub timeout: u64,

    /// Output format (text or json)
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_OUTPUT_FORMAT",
        default_value = "text",
        help = "Output format (text, json)"
    )]
    pub format: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub service: String,
    pub version: Option<String>,
    pub status: String,
    pub uptime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub services: Vec<ServiceStatus>,
    pub checks: Option<serde_json::Value>,
    pub timestamp: String,
}

/// Execute status command
pub async fn execute(cmd: StatusCommand) -> Result<()> {
    let timeout = Duration::from_secs(cmd.timeout);
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_default();

    let mut services = Vec::new();

    // Get API server info
    let (api_status, readiness) = get_api_status(&client, &cmd.api_url).await;
    services.push(api_status);

    let report = StatusReport {
        services,
        checks: readiness,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    // Output results
    match cmd.format.as_str() {
        "json" => print_json_status(&report),
        _ => print_text_status(&report),
    }

    Ok(())
}

/// Get API server status via /api/v1/info and /health/ready endpoints
async fn get_api_status(
    client: &Client,
    api_url: &str,
) -> (ServiceStatus, Option<serde_json::Value>) {
    let info_url = format!("{}/api/v1/info", api_url.trim_end_matches('/'));

    let api_status = match client.get(&info_url).send().await {
        Ok(response) => {
            if let Ok(info) = response.json::<serde_json::Value>().await {
                ServiceStatus {
                    service: "api".to_string(),
                    version: info
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    status: "running".to_string(),
                    uptime: info
                        .get("uptime")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    details: Some(info),
                }
            } else {
                ServiceStatus {
                    service: "api".to_string(),
                    version: None,
                    status: "running".to_string(),
                    uptime: None,
                    details: None,
                }
            }
        }
        Err(e) => ServiceStatus {
            service: "api".to_string(),
            version: None,
            status: format!("unreachable: {}", e),
            uptime: None,
            details: None,
        },
    };

    // Get readiness info which includes service checks
    let readiness = if api_status.status == "running" {
        let ready_url = format!("{}/health/ready", api_url.trim_end_matches('/'));
        match client.get(&ready_url).send().await {
            Ok(response) => response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("checks").cloned()),
            Err(_) => None,
        }
    } else {
        None
    };

    (api_status, readiness)
}

/// Print text status
fn print_text_status(report: &StatusReport) {
    println!("Kruxia Flow Status");
    println!("{:=<60}", "");

    // Services table
    println!("\n📊 Services:");
    println!("{:-<60}", "");
    println!(
        "{:<15} {:<15} {:<15} {:<15}",
        "SERVICE", "STATUS", "VERSION", "UPTIME"
    );
    println!("{:-<60}", "");

    for service in &report.services {
        let status_display = if service.status.len() > 13 {
            &service.status[..13]
        } else {
            &service.status
        };
        println!(
            "{:<15} {:<15} {:<15} {:<15}",
            service.service,
            status_display,
            service.version.as_deref().unwrap_or("-"),
            service.uptime.as_deref().unwrap_or("-")
        );
    }

    // Service checks from readiness endpoint
    if let Some(ref checks) = report.checks {
        println!("\n🔍 Service Checks:");
        println!("{:-<60}", "");

        if let Some(obj) = checks.as_object() {
            for (name, check) in obj {
                let status = check
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                let symbol = match status {
                    "healthy" => "✅",
                    "unhealthy" => "❌",
                    "degraded" => "⚠️",
                    _ => "❓",
                };
                println!("{} {:<15} {}", symbol, name, status);
            }
        }
    }

    println!("\n{:=<60}", "");
    println!("Timestamp: {}", report.timestamp);
}

/// Print JSON status
fn print_json_status(report: &StatusReport) {
    println!("{}", serde_json::to_string_pretty(report).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_command_defaults() {
        let cmd = StatusCommand {
            api_url: "http://127.0.0.1:8080".to_string(),
            timeout: 10,
            format: "text".to_string(),
        };

        assert_eq!(cmd.timeout, 10);
        assert_eq!(cmd.format, "text");
    }

    #[test]
    fn test_service_status_serialization() {
        let status = ServiceStatus {
            service: "api".to_string(),
            version: Some("0.3.0".to_string()),
            status: "running".to_string(),
            uptime: Some("2h 15m".to_string()),
            details: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("running"));
        assert!(json.contains("0.3.0"));
    }
}
