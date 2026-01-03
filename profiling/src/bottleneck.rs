//! Bottleneck detection and analysis for stress testing.
//!
//! This module analyzes stress test results and resource metrics to identify
//! the primary bottlenecks limiting system performance.

use serde::{Deserialize, Serialize};

use crate::monitor::{DatabaseMetrics, ResourceAnalysis};
use crate::stress::{StepMetrics, StressTestResults};

/// Priority level for recommendations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Critical => write!(f, "CRITICAL"),
            Priority::High => write!(f, "HIGH"),
            Priority::Medium => write!(f, "MEDIUM"),
            Priority::Low => write!(f, "LOW"),
        }
    }
}

/// Categories of bottlenecks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckCategory {
    Database,
    Cpu,
    Memory,
    Network,
    Configuration,
}

impl std::fmt::Display for BottleneckCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BottleneckCategory::Database => write!(f, "Database"),
            BottleneckCategory::Cpu => write!(f, "CPU"),
            BottleneckCategory::Memory => write!(f, "Memory"),
            BottleneckCategory::Network => write!(f, "Network"),
            BottleneckCategory::Configuration => write!(f, "Configuration"),
        }
    }
}

/// A detected bottleneck with details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    /// Category of the bottleneck
    pub category: BottleneckCategory,
    /// Brief description of the bottleneck
    pub description: String,
    /// Current value that triggered the bottleneck
    pub current_value: f64,
    /// Threshold that was exceeded
    pub threshold: f64,
    /// Severity score (0.0 - 1.0, higher is more severe)
    pub severity: f64,
    /// Impact on system performance
    pub impact: String,
}

/// A recommendation for addressing a bottleneck.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    /// Priority of this recommendation
    pub priority: Priority,
    /// Category of the issue
    pub category: BottleneckCategory,
    /// Description of the issue
    pub issue: String,
    /// Recommended action to take
    pub action: String,
    /// Expected impact of implementing the recommendation
    pub expected_impact: String,
}

/// Estimated system capacity based on stress test results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityEstimate {
    /// Safe concurrent workflows (comfortable margin)
    pub safe_concurrent: usize,
    /// Maximum concurrent workflows (at capacity)
    pub max_concurrent: usize,
    /// Sustained throughput in workflows per second
    pub sustained_throughput_wf_per_sec: f64,
    /// Sustained throughput in workflows per minute
    pub sustained_throughput_wf_per_min: f64,
    /// Primary factor limiting capacity
    pub limiting_factor: String,
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
}

/// Complete bottleneck analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckReport {
    /// Primary bottleneck (most limiting factor)
    pub primary_bottleneck: Option<Bottleneck>,
    /// Secondary bottlenecks
    pub secondary_bottlenecks: Vec<Bottleneck>,
    /// Recommendations for improvement
    pub recommendations: Vec<Recommendation>,
    /// Capacity estimate
    pub capacity_estimate: CapacityEstimate,
}

impl BottleneckReport {
    /// Generate a markdown report.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Bottleneck Analysis Report\n\n");

        // Primary bottleneck
        md.push_str("## Primary Bottleneck\n\n");
        if let Some(ref bn) = self.primary_bottleneck {
            md.push_str(&format!("**Category**: {}\n\n", bn.category));
            md.push_str(&format!("**Description**: {}\n\n", bn.description));
            md.push_str(&format!(
                "**Current Value**: {:.2} (threshold: {:.2})\n\n",
                bn.current_value, bn.threshold
            ));
            md.push_str(&format!("**Impact**: {}\n\n", bn.impact));
        } else {
            md.push_str("No primary bottleneck detected.\n\n");
        }

        // Secondary bottlenecks
        if !self.secondary_bottlenecks.is_empty() {
            md.push_str("## Secondary Bottlenecks\n\n");
            for bn in &self.secondary_bottlenecks {
                md.push_str(&format!("### {}\n\n", bn.category));
                md.push_str(&format!("- **Description**: {}\n", bn.description));
                md.push_str(&format!(
                    "- **Value**: {:.2} (threshold: {:.2})\n",
                    bn.current_value, bn.threshold
                ));
                md.push_str(&format!("- **Impact**: {}\n\n", bn.impact));
            }
        }

        // Capacity estimate
        md.push_str("## Capacity Estimate\n\n");
        md.push_str(&format!("| Metric | Value |\n|--------|-------|\n"));
        md.push_str(&format!(
            "| Safe Concurrent Workflows | {} |\n",
            self.capacity_estimate.safe_concurrent
        ));
        md.push_str(&format!(
            "| Max Concurrent Workflows | {} |\n",
            self.capacity_estimate.max_concurrent
        ));
        md.push_str(&format!(
            "| Sustained Throughput | {:.1} wf/sec ({:.0} wf/min) |\n",
            self.capacity_estimate.sustained_throughput_wf_per_sec,
            self.capacity_estimate.sustained_throughput_wf_per_min
        ));
        md.push_str(&format!(
            "| Limiting Factor | {} |\n",
            self.capacity_estimate.limiting_factor
        ));
        md.push_str(&format!(
            "| Confidence | {:.0}% |\n\n",
            self.capacity_estimate.confidence * 100.0
        ));

        // Recommendations
        md.push_str("## Recommendations\n\n");
        for rec in &self.recommendations {
            md.push_str(&format!("### [{}] {}\n\n", rec.priority, rec.category));
            md.push_str(&format!("**Issue**: {}\n\n", rec.issue));
            md.push_str(&format!("**Action**: {}\n\n", rec.action));
            md.push_str(&format!("**Expected Impact**: {}\n\n", rec.expected_impact));
        }

        md
    }
}

/// Analyzer for detecting bottlenecks in stress test results.
pub struct BottleneckAnalyzer {
    /// CPU threshold (0.0 - 1.0) for bottleneck detection
    pub cpu_threshold: f64,
    /// Memory threshold (0.0 - 1.0) for bottleneck detection
    pub memory_threshold: f64,
    /// Memory growth rate threshold (MB/sec) for leak detection
    pub memory_leak_threshold: f64,
    /// Connection pool utilization threshold (0.0 - 1.0)
    pub connection_pool_threshold: f64,
    /// Error rate threshold (0.0 - 1.0)
    pub error_rate_threshold: f64,
    /// Latency degradation threshold (ratio of current/baseline)
    pub latency_degradation_threshold: f64,
}

impl Default for BottleneckAnalyzer {
    fn default() -> Self {
        Self {
            cpu_threshold: 0.85,                // 85% CPU
            memory_threshold: 0.90,             // 90% memory
            memory_leak_threshold: 0.1,         // 0.1 MB/sec
            connection_pool_threshold: 0.9,     // 90% pool utilization
            error_rate_threshold: 0.05,         // 5% error rate
            latency_degradation_threshold: 3.0, // 3x latency increase
        }
    }
}

impl BottleneckAnalyzer {
    /// Analyze stress test results to detect bottlenecks.
    pub fn analyze(
        &self,
        results: &StressTestResults,
        resource_analysis: Option<&ResourceAnalysis>,
        db_metrics: Option<&DatabaseMetrics>,
    ) -> BottleneckReport {
        let mut bottlenecks = Vec::new();
        let mut recommendations = Vec::new();

        // Analyze resource usage
        if let Some(resources) = resource_analysis {
            // Check CPU
            if resources.cpu_max > self.cpu_threshold * 100.0 {
                bottlenecks.push(Bottleneck {
                    category: BottleneckCategory::Cpu,
                    description: "CPU saturation detected".to_string(),
                    current_value: resources.cpu_max,
                    threshold: self.cpu_threshold * 100.0,
                    severity: (resources.cpu_max / 100.0).min(1.0),
                    impact: "Workflow processing slowed due to CPU contention".to_string(),
                });

                recommendations.push(Recommendation {
                    priority: Priority::High,
                    category: BottleneckCategory::Cpu,
                    issue: format!(
                        "CPU usage peaked at {:.1}%, exceeding {:.0}% threshold",
                        resources.cpu_max,
                        self.cpu_threshold * 100.0
                    ),
                    action: "Scale horizontally by adding more orchestrator/worker instances, or scale vertically with more CPU cores".to_string(),
                    expected_impact: "Linear throughput increase with additional CPU capacity".to_string(),
                });
            }

            // Check memory leak
            if resources.memory_leak_detected {
                bottlenecks.push(Bottleneck {
                    category: BottleneckCategory::Memory,
                    description: "Memory leak detected".to_string(),
                    current_value: resources.memory_growth_rate,
                    threshold: self.memory_leak_threshold,
                    severity: 0.9, // High severity for memory leaks
                    impact: "System will eventually run out of memory".to_string(),
                });

                recommendations.push(Recommendation {
                    priority: Priority::Critical,
                    category: BottleneckCategory::Memory,
                    issue: format!(
                        "Memory growing at {:.3} MB/sec",
                        resources.memory_growth_rate
                    ),
                    action: "Profile memory usage with jemalloc or heaptrack to identify the source of the leak".to_string(),
                    expected_impact: "Stable memory usage, enabling long-running production workloads".to_string(),
                });
            }
        }

        // Analyze database metrics
        if let Some(db) = db_metrics {
            if db.is_pool_exhausted() {
                let utilization = db.active_connections as f64 / db.max_connections as f64;
                bottlenecks.push(Bottleneck {
                    category: BottleneckCategory::Database,
                    description: "Connection pool exhaustion".to_string(),
                    current_value: utilization * 100.0,
                    threshold: self.connection_pool_threshold * 100.0,
                    severity: utilization.min(1.0),
                    impact: "New requests blocked waiting for database connections".to_string(),
                });

                recommendations.push(Recommendation {
                    priority: Priority::High,
                    category: BottleneckCategory::Database,
                    issue: format!(
                        "Connection pool at {:.0}% ({}/{} connections)",
                        utilization * 100.0,
                        db.active_connections,
                        db.max_connections
                    ),
                    action: "Increase max_connections in PostgreSQL config and connection pool size in Kruxia Flow".to_string(),
                    expected_impact: "Higher concurrent workflow capacity".to_string(),
                });
            }
        }

        // Analyze step metrics for degradation patterns
        if let Some(last_step) = results.steps.last() {
            let error_rate = 1.0 - last_step.success_rate;
            if error_rate > self.error_rate_threshold {
                bottlenecks.push(Bottleneck {
                    category: BottleneckCategory::Configuration,
                    description: "High error rate".to_string(),
                    current_value: error_rate * 100.0,
                    threshold: self.error_rate_threshold * 100.0,
                    severity: (error_rate / 0.5).min(1.0), // Cap at 50% error rate
                    impact: "Significant workflow failures reducing effective throughput"
                        .to_string(),
                });

                // Analyze error messages for more specific recommendations
                if !last_step.errors.is_empty() {
                    let sample_error = last_step.errors.first().unwrap();
                    if sample_error.contains("timeout") {
                        recommendations.push(Recommendation {
                            priority: Priority::High,
                            category: BottleneckCategory::Configuration,
                            issue: "Workflow timeouts occurring".to_string(),
                            action: "Increase workflow timeout or optimize activity execution time"
                                .to_string(),
                            expected_impact: "Reduced timeout failures".to_string(),
                        });
                    } else if sample_error.contains("connection") {
                        recommendations.push(Recommendation {
                            priority: Priority::High,
                            category: BottleneckCategory::Database,
                            issue: "Connection errors occurring".to_string(),
                            action: "Check database connection limits and network stability"
                                .to_string(),
                            expected_impact: "Improved connection reliability".to_string(),
                        });
                    }
                }
            }
        }

        // Calculate capacity estimate
        let capacity_estimate = self.calculate_capacity(results);

        // Sort bottlenecks by severity
        bottlenecks.sort_by(|a, b| {
            b.severity
                .partial_cmp(&a.severity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Sort recommendations by priority
        recommendations.sort_by(|a, b| {
            let priority_order = |p: &Priority| match p {
                Priority::Critical => 0,
                Priority::High => 1,
                Priority::Medium => 2,
                Priority::Low => 3,
            };
            priority_order(&a.priority).cmp(&priority_order(&b.priority))
        });

        let primary_bottleneck = bottlenecks.first().cloned();
        let secondary_bottlenecks = bottlenecks.into_iter().skip(1).collect();

        BottleneckReport {
            primary_bottleneck,
            secondary_bottlenecks,
            recommendations,
            capacity_estimate,
        }
    }

    /// Calculate capacity estimate from stress test results.
    fn calculate_capacity(&self, results: &StressTestResults) -> CapacityEstimate {
        // Find the step with highest throughput without exceeding error threshold
        let healthy_steps: Vec<&StepMetrics> = results
            .steps
            .iter()
            .filter(|s| s.success_rate >= (1.0 - self.error_rate_threshold))
            .collect();

        let (safe_concurrent, max_concurrent, throughput, limiting_factor) =
            if let Some(bp) = &results.breaking_point {
                // We hit a breaking point
                let safe = if bp.concurrent_workflows > results.config.step_size {
                    bp.concurrent_workflows - results.config.step_size
                } else {
                    bp.concurrent_workflows / 2
                };
                let max = bp.concurrent_workflows;
                let best_step = healthy_steps
                    .iter()
                    .max_by(|a, b| {
                        a.throughput_wf_per_sec
                            .partial_cmp(&b.throughput_wf_per_sec)
                            .unwrap()
                    })
                    .map(|s| s.throughput_wf_per_sec)
                    .unwrap_or(0.0);
                let factor = format!("{}", bp.failure_mode);
                (safe, max, best_step, factor)
            } else if !healthy_steps.is_empty() {
                // No breaking point, use peak values
                let best_step = healthy_steps
                    .iter()
                    .max_by(|a, b| {
                        a.throughput_wf_per_sec
                            .partial_cmp(&b.throughput_wf_per_sec)
                            .unwrap()
                    })
                    .unwrap();
                let safe = (best_step.actual_concurrent as f64 * 0.8) as usize;
                let max = best_step.actual_concurrent;
                let throughput = best_step.throughput_wf_per_sec;
                (safe, max, throughput, "None detected".to_string())
            } else {
                // No successful steps
                (0, 0, 0.0, "All steps failed".to_string())
            };

        // Calculate confidence based on number of data points
        let confidence = if results.steps.len() >= 5 {
            0.9
        } else if results.steps.len() >= 3 {
            0.7
        } else {
            0.5
        };

        CapacityEstimate {
            safe_concurrent,
            max_concurrent,
            sustained_throughput_wf_per_sec: throughput,
            sustained_throughput_wf_per_min: throughput * 60.0,
            limiting_factor,
            confidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stress::{FailureMode, StressTestConfig};
    use chrono::Utc;
    use std::time::Duration;

    fn make_step(
        step_number: usize,
        concurrent: usize,
        throughput: f64,
        success_rate: f64,
        p99: u64,
    ) -> StepMetrics {
        StepMetrics {
            step_number,
            target_concurrent: concurrent,
            actual_concurrent: concurrent,
            total_workflows: 100,
            successful_workflows: (100.0 * success_rate) as usize,
            failed_workflows: (100.0 * (1.0 - success_rate)) as usize,
            throughput_wf_per_sec: throughput,
            success_rate,
            p50_latency_ms: 100,
            p95_latency_ms: 200,
            p99_latency_ms: p99,
            cpu_percent: 50.0,
            memory_mb: 500.0,
            db_connections: 10,
            duration: Duration::from_secs(30),
            errors: vec![],
            started_at: Utc::now(),
        }
    }

    #[test]
    fn test_capacity_estimate_with_breaking_point() {
        let analyzer = BottleneckAnalyzer::default();

        let results = StressTestResults {
            config: StressTestConfig::default(),
            steps: vec![
                make_step(0, 100, 50.0, 0.99, 100),
                make_step(1, 600, 80.0, 0.98, 200),
                make_step(2, 1100, 70.0, 0.90, 500), // Breaking point
            ],
            breaking_point: Some(crate::stress::BreakingPoint {
                concurrent_workflows: 1100,
                failure_mode: FailureMode::ErrorRateExceeded {
                    actual_rate: 0.10,
                    threshold: 0.05,
                },
                metrics: make_step(2, 1100, 70.0, 0.90, 500),
            }),
            peak_throughput_wf_per_sec: 80.0,
            max_concurrent_achieved: 1100,
            total_duration: Duration::from_secs(120),
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        let report = analyzer.analyze(&results, None, None);

        assert!(report.capacity_estimate.safe_concurrent < 1100);
        assert_eq!(report.capacity_estimate.max_concurrent, 1100);
        assert!(report.capacity_estimate.sustained_throughput_wf_per_sec > 0.0);
    }

    #[test]
    fn test_cpu_bottleneck_detection() {
        let analyzer = BottleneckAnalyzer::default();

        let resource_analysis = ResourceAnalysis {
            sample_count: 60,
            duration_secs: 60.0,
            cpu_min: 50.0,
            cpu_max: 95.0, // Over threshold
            cpu_avg: 80.0,
            memory_min_mb: 400.0,
            memory_max_mb: 500.0,
            memory_avg_mb: 450.0,
            memory_growth_rate: 0.01,
            memory_leak_detected: false,
        };

        let results = StressTestResults {
            config: StressTestConfig::default(),
            steps: vec![make_step(0, 100, 50.0, 0.99, 100)],
            breaking_point: None,
            peak_throughput_wf_per_sec: 50.0,
            max_concurrent_achieved: 100,
            total_duration: Duration::from_secs(60),
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        let report = analyzer.analyze(&results, Some(&resource_analysis), None);

        assert!(report.primary_bottleneck.is_some());
        assert_eq!(
            report.primary_bottleneck.as_ref().unwrap().category,
            BottleneckCategory::Cpu
        );
    }
}
