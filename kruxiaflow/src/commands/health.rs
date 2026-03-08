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
        env = "KRUXIAFLOW_API_URL",
        default_value = "http://127.0.0.1:8080",
        help = "Kruxia Flow API server URL"
    )]
    pub api_url: String,

    /// Health check timeout in seconds
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_HEALTH_TIMEOUT",
        default_value = "5",
        help = "Timeout for health checks in seconds"
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
            // Handle both flat strings ("ok") and structured objects ({"status": "healthy"})
            let status_str = service_check
                .as_str()
                .or_else(|| service_check.get("status").and_then(|s| s.as_str()))
                .unwrap_or("unknown");

            let status = match status_str {
                "healthy" | "ok" => HealthStatus::Healthy,
                // "error" is not emitted by this server but accepted defensively
                "unhealthy" | "error" => HealthStatus::Unhealthy,
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
    println!("Kruxia Flow Health Check");
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

        if verbose && let Some(ref details) = result.details {
            println!(
                "   Details: {}",
                serde_json::to_string_pretty(details).unwrap_or_default()
            );
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

    // =========================================================================
    // extract_service_health edge cases
    // =========================================================================

    #[test]
    fn test_extract_service_health_unhealthy() {
        let readiness = serde_json::json!({
            "checks": {
                "database": {
                    "status": "unhealthy",
                    "message": "Connection refused"
                }
            }
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Unhealthy);
        assert_eq!(result.message, Some("Connection refused".to_string()));
    }

    #[test]
    fn test_extract_service_health_degraded() {
        let readiness = serde_json::json!({
            "checks": {
                "orchestrator": {
                    "status": "degraded",
                    "message": "High latency"
                }
            }
        });

        let result = extract_service_health("orchestrator", &readiness);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_extract_service_health_flat_string_ok() {
        // Readiness endpoint returns flat "ok" strings, not structured objects
        let readiness = serde_json::json!({
            "status": "ready",
            "checks": {
                "database": "ok",
                "event_source": "ok",
                "queue": "ok"
            }
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.message, Some("ok".to_string()));
    }

    #[test]
    fn test_extract_service_health_flat_string_error() {
        let readiness = serde_json::json!({
            "checks": { "database": "error" }
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_extract_service_health_unknown_status_string() {
        let readiness = serde_json::json!({
            "checks": {
                "database": {
                    "status": "starting_up"
                }
            }
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Unknown);
        // Message falls back to status string when no message field
        assert_eq!(result.message, Some("starting_up".to_string()));
    }

    #[test]
    fn test_extract_service_health_missing_status_field() {
        let readiness = serde_json::json!({
            "checks": {
                "database": {
                    "message": "some message"
                }
            }
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Unknown);
    }

    #[test]
    fn test_extract_service_health_no_checks_key() {
        let readiness = serde_json::json!({
            "status": "ok"
        });

        let result = extract_service_health("database", &readiness);
        assert_eq!(result.status, HealthStatus::Unknown);
        assert_eq!(
            result.message,
            Some("Not reported in readiness check".to_string())
        );
    }

    #[test]
    fn test_extract_service_health_has_details() {
        let readiness = serde_json::json!({
            "checks": {
                "database": {
                    "status": "healthy",
                    "message": "Connected",
                    "latency_ms": 5
                }
            }
        });

        let result = extract_service_health("database", &readiness);
        assert!(result.details.is_some());
        let details = result.details.unwrap();
        assert_eq!(details.get("latency_ms").unwrap().as_i64(), Some(5));
    }

    // =========================================================================
    // HealthResult serde tests
    // =========================================================================

    #[test]
    fn test_health_result_deserialization() {
        let json = r#"{
            "service": "api",
            "status": "healthy",
            "message": "OK",
            "latency_ms": 42
        }"#;

        let result: HealthResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.service, "api");
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.latency_ms, Some(42));
    }

    #[test]
    fn test_health_result_serialization_skips_none_details() {
        let result = HealthResult {
            service: "api".to_string(),
            status: HealthStatus::Healthy,
            message: Some("OK".to_string()),
            latency_ms: Some(10),
            details: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("details"));
    }

    #[test]
    fn test_health_result_serialization_includes_details() {
        let result = HealthResult {
            service: "api".to_string(),
            status: HealthStatus::Healthy,
            message: None,
            latency_ms: None,
            details: Some(serde_json::json!({"key": "value"})),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("details"));
        assert!(json.contains("value"));
    }

    #[test]
    fn test_health_result_with_none_message() {
        let result = HealthResult {
            service: "test".to_string(),
            status: HealthStatus::Unknown,
            message: None,
            latency_ms: None,
            details: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"message\":null"));
    }

    // =========================================================================
    // HealthStatus serde tests
    // =========================================================================

    #[test]
    fn test_health_status_serialization_all_variants() {
        assert_eq!(
            serde_json::to_string(&HealthStatus::Healthy).unwrap(),
            "\"healthy\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Unhealthy).unwrap(),
            "\"unhealthy\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Degraded).unwrap(),
            "\"degraded\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Unknown).unwrap(),
            "\"unknown\""
        );
    }

    #[test]
    fn test_health_status_deserialization() {
        assert_eq!(
            serde_json::from_str::<HealthStatus>("\"healthy\"").unwrap(),
            HealthStatus::Healthy
        );
        assert_eq!(
            serde_json::from_str::<HealthStatus>("\"unhealthy\"").unwrap(),
            HealthStatus::Unhealthy
        );
        assert_eq!(
            serde_json::from_str::<HealthStatus>("\"degraded\"").unwrap(),
            HealthStatus::Degraded
        );
        assert_eq!(
            serde_json::from_str::<HealthStatus>("\"unknown\"").unwrap(),
            HealthStatus::Unknown
        );
    }

    #[test]
    fn test_health_status_copy_clone() {
        let status = HealthStatus::Healthy;
        let copied = status;
        let cloned = status;
        assert_eq!(copied, HealthStatus::Healthy);
        assert_eq!(cloned, HealthStatus::Healthy);
    }

    // =========================================================================
    // HealthReport tests
    // =========================================================================

    #[test]
    fn test_health_report_serialization() {
        let report = HealthReport {
            overall_status: HealthStatus::Healthy,
            services: vec![
                HealthResult {
                    service: "api".to_string(),
                    status: HealthStatus::Healthy,
                    message: Some("HTTP 200".to_string()),
                    latency_ms: Some(5),
                    details: None,
                },
                HealthResult {
                    service: "database".to_string(),
                    status: HealthStatus::Healthy,
                    message: Some("Connected".to_string()),
                    latency_ms: None,
                    details: None,
                },
            ],
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("api"));
        assert!(json.contains("database"));
        assert!(json.contains("2026-02-01"));
    }

    // =========================================================================
    // Overall status determination tests
    // =========================================================================

    #[test]
    fn test_overall_status_all_healthy() {
        let results = [
            HealthResult {
                service: "a".to_string(),
                status: HealthStatus::Healthy,
                message: None,
                latency_ms: None,
                details: None,
            },
            HealthResult {
                service: "b".to_string(),
                status: HealthStatus::Healthy,
                message: None,
                latency_ms: None,
                details: None,
            },
        ];

        let overall = if results.iter().all(|r| r.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if results.iter().any(|r| r.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };

        assert_eq!(overall, HealthStatus::Healthy);
    }

    #[test]
    fn test_overall_status_any_unhealthy() {
        let results = [
            HealthResult {
                service: "a".to_string(),
                status: HealthStatus::Healthy,
                message: None,
                latency_ms: None,
                details: None,
            },
            HealthResult {
                service: "b".to_string(),
                status: HealthStatus::Unhealthy,
                message: None,
                latency_ms: None,
                details: None,
            },
        ];

        let overall = if results.iter().all(|r| r.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if results.iter().any(|r| r.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };

        assert_eq!(overall, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_overall_status_degraded() {
        let results = [
            HealthResult {
                service: "a".to_string(),
                status: HealthStatus::Healthy,
                message: None,
                latency_ms: None,
                details: None,
            },
            HealthResult {
                service: "b".to_string(),
                status: HealthStatus::Degraded,
                message: None,
                latency_ms: None,
                details: None,
            },
        ];

        let overall = if results.iter().all(|r| r.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if results.iter().any(|r| r.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };

        assert_eq!(overall, HealthStatus::Degraded);
    }

    #[test]
    fn test_overall_status_unknown_is_degraded() {
        let results = [HealthResult {
            service: "a".to_string(),
            status: HealthStatus::Unknown,
            message: None,
            latency_ms: None,
            details: None,
        }];

        let overall = if results.iter().all(|r| r.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else if results.iter().any(|r| r.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };

        assert_eq!(overall, HealthStatus::Degraded);
    }

    // =========================================================================
    // Output formatting tests
    // =========================================================================

    #[test]
    fn test_print_text_report_no_panic() {
        let report = HealthReport {
            overall_status: HealthStatus::Healthy,
            services: vec![HealthResult {
                service: "api".to_string(),
                status: HealthStatus::Healthy,
                message: Some("HTTP 200".to_string()),
                latency_ms: Some(5),
                details: None,
            }],
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic
        print_text_report(&report, false);
    }

    #[test]
    fn test_print_text_report_verbose_no_panic() {
        let report = HealthReport {
            overall_status: HealthStatus::Degraded,
            services: vec![HealthResult {
                service: "api".to_string(),
                status: HealthStatus::Healthy,
                message: Some("OK".to_string()),
                latency_ms: Some(10),
                details: Some(serde_json::json!({"version": "0.3.0"})),
            }],
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic with verbose output and details
        print_text_report(&report, true);
    }

    #[test]
    fn test_print_text_report_no_message() {
        let report = HealthReport {
            overall_status: HealthStatus::Unknown,
            services: vec![HealthResult {
                service: "test".to_string(),
                status: HealthStatus::Unknown,
                message: None,
                latency_ms: None,
                details: None,
            }],
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic when message is None (shows "No details")
        print_text_report(&report, false);
    }

    #[test]
    fn test_print_json_report_no_panic() {
        let report = HealthReport {
            overall_status: HealthStatus::Unhealthy,
            services: vec![
                HealthResult {
                    service: "api".to_string(),
                    status: HealthStatus::Unhealthy,
                    message: Some("Connection refused".to_string()),
                    latency_ms: None,
                    details: None,
                },
                HealthResult {
                    service: "database".to_string(),
                    status: HealthStatus::Unknown,
                    message: Some("API unreachable".to_string()),
                    latency_ms: None,
                    details: None,
                },
            ],
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Should not panic
        print_json_report(&report);
    }

    #[test]
    fn test_print_text_report_verbose_no_details() {
        let report = HealthReport {
            overall_status: HealthStatus::Healthy,
            services: vec![HealthResult {
                service: "api".to_string(),
                status: HealthStatus::Healthy,
                message: Some("OK".to_string()),
                latency_ms: None,
                details: None,
            }],
            timestamp: "2026-02-01T00:00:00Z".to_string(),
        };

        // Verbose mode with no details should not panic
        print_text_report(&report, true);
    }

    // =========================================================================
    // HealthCommand construction tests
    // =========================================================================

    #[test]
    fn test_health_command_with_service_filter() {
        let cmd = HealthCommand {
            api_url: "http://localhost:8080".to_string(),
            timeout: 10,
            format: "json".to_string(),
            service: Some("database".to_string()),
            verbose: true,
        };

        assert_eq!(cmd.service, Some("database".to_string()));
        assert!(cmd.verbose);
        assert_eq!(cmd.format, "json");
    }

    #[test]
    fn test_health_command_verbose_flag() {
        let cmd = HealthCommand {
            api_url: "http://127.0.0.1:8080".to_string(),
            timeout: 5,
            format: "text".to_string(),
            service: None,
            verbose: true,
        };

        assert!(cmd.verbose);
    }

    #[test]
    fn test_health_result_clone() {
        let result = HealthResult {
            service: "api".to_string(),
            status: HealthStatus::Healthy,
            message: Some("OK".to_string()),
            latency_ms: Some(10),
            details: Some(serde_json::json!({"key": "value"})),
        };

        let cloned = result.clone();
        assert_eq!(cloned.service, "api");
        assert_eq!(cloned.status, HealthStatus::Healthy);
        assert_eq!(cloned.latency_ms, Some(10));
    }

    #[test]
    fn test_health_result_debug() {
        let result = HealthResult {
            service: "api".to_string(),
            status: HealthStatus::Healthy,
            message: None,
            latency_ms: None,
            details: None,
        };

        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("HealthResult"));
        assert!(debug_str.contains("api"));
    }

    #[test]
    fn test_health_status_debug() {
        let debug_str = format!("{:?}", HealthStatus::Healthy);
        assert_eq!(debug_str, "Healthy");

        let debug_str = format!("{:?}", HealthStatus::Unhealthy);
        assert_eq!(debug_str, "Unhealthy");
    }

    // =========================================================================
    // check_api_and_readiness integration tests with wiremock
    // =========================================================================

    #[tokio::test]
    async fn test_check_api_and_readiness_healthy() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "healthy"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/health/ready"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ready",
                "checks": {
                    "database": {"status": "healthy", "message": "Connected"},
                    "orchestrator": {"status": "healthy"}
                }
            })))
            .mount(&mock_server)
            .await;

        let client = Client::new();
        let (api_result, readiness) = check_api_and_readiness(&client, &mock_server.uri()).await;

        assert_eq!(api_result.status, HealthStatus::Healthy);
        assert!(api_result.latency_ms.is_some());
        assert!(readiness.is_some());
    }

    #[tokio::test]
    async fn test_check_api_and_readiness_unhealthy_api() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;

        let client = Client::new();
        let (api_result, readiness) = check_api_and_readiness(&client, &mock_server.uri()).await;

        assert_eq!(api_result.status, HealthStatus::Unhealthy);
        assert!(readiness.is_none());
    }

    #[tokio::test]
    async fn test_check_api_and_readiness_connection_refused() {
        let client = Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let (api_result, readiness) = check_api_and_readiness(&client, "http://127.0.0.1:1").await;

        assert_eq!(api_result.status, HealthStatus::Unhealthy);
        assert!(api_result.message.unwrap().contains("Request failed"));
        assert!(readiness.is_none());
    }

    #[tokio::test]
    async fn test_check_api_and_readiness_healthy_but_readiness_fails() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "healthy"})),
            )
            .mount(&mock_server)
            .await;

        // readiness endpoint returns 500
        Mock::given(method("GET"))
            .and(path("/health/ready"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = Client::new();
        let (api_result, _readiness) = check_api_and_readiness(&client, &mock_server.uri()).await;

        assert_eq!(api_result.status, HealthStatus::Healthy);
        // readiness might be None since the response is not valid JSON
        // (or Some if wiremock returns empty JSON body for 500)
    }

    #[tokio::test]
    async fn test_execute_healthy_api() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "healthy"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/health/ready"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ready",
                "checks": {
                    "database": {"status": "healthy"},
                    "orchestrator": {"status": "healthy"}
                }
            })))
            .mount(&mock_server)
            .await;

        let cmd = HealthCommand {
            api_url: mock_server.uri(),
            service: None,
            format: "text".to_string(),
            timeout: 5,
            verbose: false,
        };

        let result = execute(cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_json_format() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "healthy"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/health/ready"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ready",
                "checks": {
                    "database": {"status": "healthy"},
                    "orchestrator": {"status": "healthy"}
                }
            })))
            .mount(&mock_server)
            .await;

        let cmd = HealthCommand {
            api_url: mock_server.uri(),
            service: None,
            format: "json".to_string(),
            timeout: 5,
            verbose: false,
        };

        let result = execute(cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_specific_service() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "healthy"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/health/ready"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ready",
                "checks": {
                    "database": {"status": "healthy"},
                    "orchestrator": {"status": "healthy"}
                }
            })))
            .mount(&mock_server)
            .await;

        let cmd = HealthCommand {
            api_url: mock_server.uri(),
            service: Some("database".to_string()),
            format: "text".to_string(),
            timeout: 5,
            verbose: true,
        };

        let result = execute(cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_api_only() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "healthy"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/health/ready"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ready",
                "checks": {}
            })))
            .mount(&mock_server)
            .await;

        let cmd = HealthCommand {
            api_url: mock_server.uri(),
            service: Some("api".to_string()),
            format: "text".to_string(),
            timeout: 5,
            verbose: false,
        };

        let result = execute(cmd).await;
        assert!(result.is_ok());
    }
}
