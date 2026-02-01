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

    // =========================================================================
    // ServiceStatus serde tests
    // =========================================================================

    #[test]
    fn test_service_status_deserialization() {
        let json = r#"{
            "service": "api",
            "version": "0.3.0",
            "status": "running",
            "uptime": "1h 30m"
        }"#;

        let status: ServiceStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.service, "api");
        assert_eq!(status.version, Some("0.3.0".to_string()));
        assert_eq!(status.status, "running");
        assert_eq!(status.uptime, Some("1h 30m".to_string()));
    }

    #[test]
    fn test_service_status_deserialization_with_nulls() {
        let json = r#"{
            "service": "api",
            "version": null,
            "status": "unreachable",
            "uptime": null
        }"#;

        let status: ServiceStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.service, "api");
        assert!(status.version.is_none());
        assert_eq!(status.status, "unreachable");
        assert!(status.uptime.is_none());
    }

    #[test]
    fn test_service_status_serialization_skips_none_details() {
        let status = ServiceStatus {
            service: "api".to_string(),
            version: None,
            status: "running".to_string(),
            uptime: None,
            details: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("details"));
    }

    #[test]
    fn test_service_status_serialization_includes_details() {
        let status = ServiceStatus {
            service: "api".to_string(),
            version: Some("0.3.0".to_string()),
            status: "running".to_string(),
            uptime: None,
            details: Some(serde_json::json!({"features": ["llm", "email"]})),
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("details"));
        assert!(json.contains("features"));
    }

    #[test]
    fn test_service_status_debug() {
        let status = ServiceStatus {
            service: "api".to_string(),
            version: Some("0.3.0".to_string()),
            status: "running".to_string(),
            uptime: None,
            details: None,
        };

        let debug_str = format!("{:?}", status);
        assert!(debug_str.contains("ServiceStatus"));
        assert!(debug_str.contains("api"));
    }

    // =========================================================================
    // StatusReport tests
    // =========================================================================

    #[test]
    fn test_status_report_serialization() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: Some("0.3.0".to_string()),
                status: "running".to_string(),
                uptime: Some("2h".to_string()),
                details: None,
            }],
            checks: Some(serde_json::json!({
                "database": {"status": "healthy"},
                "orchestrator": {"status": "healthy"}
            })),
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("api"));
        assert!(json.contains("database"));
        assert!(json.contains("orchestrator"));
        assert!(json.contains("2026-02-01"));
    }

    #[test]
    fn test_status_report_serialization_no_checks() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: None,
                status: "unreachable: connection refused".to_string(),
                uptime: None,
                details: None,
            }],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("unreachable"));
    }

    #[test]
    fn test_status_report_debug() {
        let report = StatusReport {
            services: vec![],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        let debug_str = format!("{:?}", report);
        assert!(debug_str.contains("StatusReport"));
    }

    // =========================================================================
    // Output formatting tests
    // =========================================================================

    #[test]
    fn test_print_text_status_no_panic() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: Some("0.3.0".to_string()),
                status: "running".to_string(),
                uptime: Some("2h 15m".to_string()),
                details: None,
            }],
            checks: Some(serde_json::json!({
                "database": {"status": "healthy"},
                "orchestrator": {"status": "unhealthy"}
            })),
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic
        print_text_status(&report);
    }

    #[test]
    fn test_print_text_status_no_checks() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: None,
                status: "unreachable: timeout".to_string(),
                uptime: None,
                details: None,
            }],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic when checks is None
        print_text_status(&report);
    }

    #[test]
    fn test_print_text_status_truncates_long_status() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: None,
                status: "unreachable: connection refused by remote host".to_string(),
                uptime: None,
                details: None,
            }],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Status longer than 13 chars should be truncated; should not panic
        print_text_status(&report);
    }

    #[test]
    fn test_print_text_status_short_status() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: None,
                status: "running".to_string(), // Exactly 7 chars, under 13
                uptime: None,
                details: None,
            }],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Short status should not be truncated
        print_text_status(&report);
    }

    #[test]
    fn test_print_text_status_with_degraded_check() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: Some("0.3.0".to_string()),
                status: "running".to_string(),
                uptime: None,
                details: None,
            }],
            checks: Some(serde_json::json!({
                "database": {"status": "degraded"},
                "cache": {"status": "unknown_status"}
            })),
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should handle degraded and unknown statuses without panic
        print_text_status(&report);
    }

    #[test]
    fn test_print_json_status_no_panic() {
        let report = StatusReport {
            services: vec![ServiceStatus {
                service: "api".to_string(),
                version: Some("0.3.0".to_string()),
                status: "running".to_string(),
                uptime: Some("30m".to_string()),
                details: Some(serde_json::json!({"build": "abc123"})),
            }],
            checks: Some(serde_json::json!({"database": {"status": "healthy"}})),
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic
        print_json_status(&report);
    }

    #[test]
    fn test_print_text_status_empty_services() {
        let report = StatusReport {
            services: vec![],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic with empty services
        print_text_status(&report);
    }

    #[test]
    fn test_print_text_status_checks_non_object() {
        let report = StatusReport {
            services: vec![],
            checks: Some(serde_json::json!("not_an_object")),
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Non-object checks value should not panic
        print_text_status(&report);
    }

    // =========================================================================
    // StatusCommand construction tests
    // =========================================================================

    #[test]
    fn test_status_command_json_format() {
        let cmd = StatusCommand {
            api_url: "http://127.0.0.1:9090".to_string(),
            timeout: 30,
            format: "json".to_string(),
        };

        assert_eq!(cmd.format, "json");
        assert_eq!(cmd.timeout, 30);
    }

    #[test]
    fn test_status_command_custom_api_url() {
        let cmd = StatusCommand {
            api_url: "https://kruxiaflow.example.com".to_string(),
            timeout: 10,
            format: "text".to_string(),
        };

        assert_eq!(cmd.api_url, "https://kruxiaflow.example.com");
    }

    // =========================================================================
    // Multiple services tests
    // =========================================================================

    #[test]
    fn test_print_text_status_multiple_services() {
        let report = StatusReport {
            services: vec![
                ServiceStatus {
                    service: "api".to_string(),
                    version: Some("0.3.0".to_string()),
                    status: "running".to_string(),
                    uptime: Some("2h".to_string()),
                    details: None,
                },
                ServiceStatus {
                    service: "worker".to_string(),
                    version: Some("0.3.0".to_string()),
                    status: "running".to_string(),
                    uptime: Some("1h 55m".to_string()),
                    details: None,
                },
            ],
            checks: None,
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        print_text_status(&report);
    }

    #[test]
    fn test_service_status_with_no_version_no_uptime() {
        let status = ServiceStatus {
            service: "api".to_string(),
            version: None,
            status: "running".to_string(),
            uptime: None,
            details: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        let deser: ServiceStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.service, "api");
        assert!(deser.version.is_none());
        assert!(deser.uptime.is_none());
    }
}
