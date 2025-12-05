//! Capacity planning and documentation generation.
//!
//! This module generates capacity planning documentation based on
//! stress test results and bottleneck analysis.

use serde::{Deserialize, Serialize};

use crate::bottleneck::BottleneckReport;
use crate::stress::StressTestResults;

/// System configuration for capacity testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfiguration {
    /// Number of orchestrator instances
    pub orchestrators: u32,
    /// Number of worker instances
    pub workers: u32,
    /// Database connections per instance
    pub db_connections_per_instance: u32,
    /// Worker threads per instance
    pub worker_threads: u32,
    /// PostgreSQL max_connections
    pub postgres_max_connections: u32,
    /// Memory allocated per instance (MB)
    pub memory_per_instance_mb: u32,
}

impl Default for SystemConfiguration {
    fn default() -> Self {
        Self {
            orchestrators: 1,
            workers: 1,
            db_connections_per_instance: 20,
            worker_threads: 4,
            postgres_max_connections: 100,
            memory_per_instance_mb: 2048,
        }
    }
}

impl SystemConfiguration {
    /// Generate a short description.
    pub fn short_description(&self) -> String {
        format!("{} orch + {} workers", self.orchestrators, self.workers)
    }

    /// Total connections used.
    pub fn total_connections(&self) -> u32 {
        (self.orchestrators + self.workers) * self.db_connections_per_instance
    }
}

/// A single row in the capacity matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityRow {
    /// System configuration
    pub configuration: SystemConfiguration,
    /// Concurrent workflows tested
    pub concurrent_workflows: usize,
    /// Throughput in workflows per minute
    pub throughput_wf_per_min: f64,
    /// Throughput in workflows per second
    pub throughput_wf_per_sec: f64,
    /// P99 latency in milliseconds
    pub latency_p99_ms: u64,
    /// CPU utilization (0.0 - 100.0)
    pub cpu_utilization: f64,
    /// Memory utilization (0.0 - 100.0)
    pub memory_utilization: f64,
    /// Capacity status
    pub status: CapacityStatus,
}

/// Status of system at a given capacity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapacityStatus {
    /// All metrics within comfortable limits
    Healthy,
    /// Near limits but stable
    AtCapacity,
    /// Exceeding some thresholds
    Degraded,
    /// Significant failures
    Overloaded,
}

impl std::fmt::Display for CapacityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapacityStatus::Healthy => write!(f, "Healthy"),
            CapacityStatus::AtCapacity => write!(f, "At Capacity"),
            CapacityStatus::Degraded => write!(f, "Degraded"),
            CapacityStatus::Overloaded => write!(f, "Overloaded"),
        }
    }
}

impl CapacityStatus {
    /// Get emoji representation.
    pub fn emoji(&self) -> &'static str {
        match self {
            CapacityStatus::Healthy => "✅",
            CapacityStatus::AtCapacity => "⚠️",
            CapacityStatus::Degraded => "🔴",
            CapacityStatus::Overloaded => "💀",
        }
    }
}

/// Complete capacity matrix with multiple configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityMatrix {
    /// Rows in the capacity matrix
    pub rows: Vec<CapacityRow>,
    /// Test metadata
    pub test_date: String,
    /// Git SHA of tested code
    pub git_sha: String,
}

impl CapacityMatrix {
    /// Generate from stress test results.
    pub fn from_stress_results(results: &StressTestResults, config: &SystemConfiguration) -> Self {
        let mut rows = Vec::new();

        for step in &results.steps {
            let status = determine_status(step.success_rate, step.p99_latency_ms, step.cpu_percent);

            rows.push(CapacityRow {
                configuration: config.clone(),
                concurrent_workflows: step.actual_concurrent,
                throughput_wf_per_min: step.throughput_wf_per_sec * 60.0,
                throughput_wf_per_sec: step.throughput_wf_per_sec,
                latency_p99_ms: step.p99_latency_ms,
                cpu_utilization: step.cpu_percent,
                memory_utilization: step.memory_mb / config.memory_per_instance_mb as f64 * 100.0,
                status,
            });
        }

        Self {
            rows,
            test_date: results.started_at.format("%Y-%m-%d").to_string(),
            git_sha: "unknown".to_string(),
        }
    }

    /// Generate markdown table.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("| Configuration | Concurrent | wf/min | P99 | CPU | Memory | Status |\n");
        md.push_str("|---------------|------------|--------|-----|-----|--------|--------|\n");

        for row in &self.rows {
            md.push_str(&format!(
                "| {} | {} | {:.0} | {}ms | {:.0}% | {:.0}% | {} {} |\n",
                row.configuration.short_description(),
                row.concurrent_workflows,
                row.throughput_wf_per_min,
                row.latency_p99_ms,
                row.cpu_utilization,
                row.memory_utilization,
                row.status.emoji(),
                row.status
            ));
        }

        md
    }
}

fn determine_status(success_rate: f64, p99_ms: u64, cpu_percent: f64) -> CapacityStatus {
    if success_rate < 0.9 {
        CapacityStatus::Overloaded
    } else if success_rate < 0.95 || p99_ms > 3000 {
        CapacityStatus::Degraded
    } else if cpu_percent > 80.0 || p99_ms > 1000 {
        CapacityStatus::AtCapacity
    } else {
        CapacityStatus::Healthy
    }
}

/// Generate the complete capacity planning document.
pub fn generate_capacity_document(
    results: &StressTestResults,
    bottleneck_report: &BottleneckReport,
    config: &SystemConfiguration,
) -> String {
    let mut doc = String::new();

    doc.push_str("# StreamFlow Capacity Planning Guide\n\n");
    doc.push_str(&format!(
        "**Generated**: {}\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));

    // Executive Summary
    doc.push_str("## Executive Summary\n\n");
    doc.push_str(&format!(
        "Based on stress testing with {}, StreamFlow can sustain:\n\n",
        config.short_description()
    ));
    doc.push_str(&format!(
        "- **Safe Operating Capacity**: {} concurrent workflows\n",
        bottleneck_report.capacity_estimate.safe_concurrent
    ));
    doc.push_str(&format!(
        "- **Peak Capacity**: {} concurrent workflows (with degraded latency)\n",
        bottleneck_report.capacity_estimate.max_concurrent
    ));
    if let Some(ref bp) = results.breaking_point {
        doc.push_str(&format!(
            "- **Breaking Point**: {} concurrent workflows\n",
            bp.concurrent_workflows
        ));
    }
    doc.push_str("\n");

    // Capacity Matrix
    doc.push_str("## Capacity Matrix\n\n");
    let matrix = CapacityMatrix::from_stress_results(results, config);
    doc.push_str(&matrix.to_markdown());
    doc.push_str("\n");

    // Bottleneck Analysis
    doc.push_str("## Bottleneck Analysis\n\n");

    if let Some(ref bn) = bottleneck_report.primary_bottleneck {
        doc.push_str("### Primary Bottleneck\n\n");
        doc.push_str(&format!("**Category**: {}\n\n", bn.category));
        doc.push_str(&format!("**Description**: {}\n\n", bn.description));
        doc.push_str(&format!(
            "**Current Value**: {:.2} (threshold: {:.2})\n\n",
            bn.current_value, bn.threshold
        ));
        doc.push_str(&format!("**Impact**: {}\n\n", bn.impact));
    } else {
        doc.push_str("No primary bottleneck detected during testing.\n\n");
    }

    if !bottleneck_report.secondary_bottlenecks.is_empty() {
        doc.push_str("### Secondary Bottlenecks\n\n");
        for bn in &bottleneck_report.secondary_bottlenecks {
            doc.push_str(&format!("- **{}**: {}\n", bn.category, bn.description));
        }
        doc.push_str("\n");
    }

    // Scaling Recommendations
    doc.push_str("## Scaling Recommendations\n\n");

    doc.push_str("### Horizontal Scaling\n\n");
    doc.push_str("Add more instances when:\n");
    doc.push_str("- CPU utilization consistently above 80%\n");
    doc.push_str("- Single instance throughput plateaus\n");
    doc.push_str("- Need geographic distribution\n\n");

    doc.push_str("### Vertical Scaling\n\n");
    doc.push_str("Increase resources when:\n");
    doc.push_str("- Memory utilization above 85%\n");
    doc.push_str("- Connection pool exhaustion\n");
    doc.push_str("- Need higher per-instance throughput\n\n");

    doc.push_str("### Database Scaling\n\n");
    doc.push_str("Optimize PostgreSQL when:\n");
    doc.push_str("- Connection pool at capacity\n");
    doc.push_str("- Query latency increasing\n");
    doc.push_str("- Dead tuple accumulation\n\n");

    // Recommendations
    if !bottleneck_report.recommendations.is_empty() {
        doc.push_str("## Actionable Recommendations\n\n");
        for rec in &bottleneck_report.recommendations {
            doc.push_str(&format!("### [{}] {}\n\n", rec.priority, rec.category));
            doc.push_str(&format!("**Issue**: {}\n\n", rec.issue));
            doc.push_str(&format!("**Action**: {}\n\n", rec.action));
            doc.push_str(&format!("**Expected Impact**: {}\n\n", rec.expected_impact));
        }
    }

    // Hardware Requirements
    doc.push_str("## Hardware Requirements\n\n");
    doc.push_str(&format!(
        "For target capacity of {:.0} workflows/minute:\n\n",
        bottleneck_report
            .capacity_estimate
            .sustained_throughput_wf_per_min
    ));
    doc.push_str("| Component | Minimum | Recommended | Notes |\n");
    doc.push_str("|-----------|---------|-------------|-------|\n");
    doc.push_str(&format!(
        "| CPU Cores | {} | {} | Per instance |\n",
        config.worker_threads,
        config.worker_threads * 2
    ));
    doc.push_str(&format!(
        "| Memory | {} MB | {} MB | Per instance |\n",
        config.memory_per_instance_mb / 2,
        config.memory_per_instance_mb
    ));
    doc.push_str(&format!(
        "| PostgreSQL | {} conn | {} conn | Max connections |\n",
        config.total_connections(),
        config.total_connections() * 2
    ));
    doc.push_str("| Network | 100 Mbps | 1 Gbps | Between services |\n\n");

    // Failure Modes
    doc.push_str("## Failure Modes\n\n");

    doc.push_str("### Graceful Degradation\n\n");
    doc.push_str("When system is overloaded:\n");
    doc.push_str("- New workflow submissions may be rate-limited\n");
    doc.push_str("- Latency increases but workflows complete\n");
    doc.push_str("- Error responses with retry guidance\n\n");

    doc.push_str("### Recovery Behavior\n\n");
    doc.push_str("After load is reduced:\n");
    doc.push_str("- Queued work drains within minutes\n");
    doc.push_str("- Latency returns to baseline\n");
    doc.push_str("- No manual intervention required\n\n");

    // Test Configuration
    doc.push_str("## Test Configuration\n\n");
    doc.push_str("| Parameter | Value |\n");
    doc.push_str("|-----------|-------|\n");
    doc.push_str(&format!(
        "| Initial Concurrent | {} |\n",
        results.config.initial_concurrent
    ));
    doc.push_str(&format!(
        "| Peak Concurrent | {} |\n",
        results.config.peak_concurrent
    ));
    doc.push_str(&format!("| Step Size | {} |\n", results.config.step_size));
    doc.push_str(&format!(
        "| Step Duration | {}s |\n",
        results.config.step_duration.as_secs()
    ));
    doc.push_str(&format!(
        "| Workflow | {} |\n",
        results.config.workflow_definition
    ));
    doc.push_str(&format!(
        "| Total Duration | {:.0}s |\n",
        results.total_duration.as_secs_f64()
    ));

    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capacity_status_emoji() {
        assert_eq!(CapacityStatus::Healthy.emoji(), "✅");
        assert_eq!(CapacityStatus::AtCapacity.emoji(), "⚠️");
        assert_eq!(CapacityStatus::Degraded.emoji(), "🔴");
        assert_eq!(CapacityStatus::Overloaded.emoji(), "💀");
    }

    #[test]
    fn test_determine_status() {
        assert_eq!(determine_status(0.99, 100, 50.0), CapacityStatus::Healthy);
        assert_eq!(
            determine_status(0.99, 1500, 85.0),
            CapacityStatus::AtCapacity
        );
        assert_eq!(determine_status(0.93, 2000, 70.0), CapacityStatus::Degraded);
        assert_eq!(
            determine_status(0.85, 5000, 95.0),
            CapacityStatus::Overloaded
        );
    }

    #[test]
    fn test_system_configuration() {
        let config = SystemConfiguration::default();
        assert_eq!(config.short_description(), "1 orch + 1 workers");
        assert_eq!(config.total_connections(), 40); // 2 * 20
    }
}
