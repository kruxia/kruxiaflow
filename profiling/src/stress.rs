//! Stress testing types and utilities for Kruxia Flow.
//!
//! This module provides types and functions for running ramping stress tests
//! that gradually increase load to identify system breaking points.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

use crate::client::StreamFlowClient;

/// Configuration for a ramping stress test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressTestConfig {
    /// Starting number of concurrent workflows
    pub initial_concurrent: usize,
    /// Maximum number of concurrent workflows to ramp up to
    pub peak_concurrent: usize,
    /// Number of concurrent workflows to add per step
    pub step_size: usize,
    /// Duration to maintain each concurrency level
    pub step_duration: Duration,
    /// Cooldown period between steps
    pub cooldown: Duration,
    /// Workflow definition to use for testing
    pub workflow_definition: String,
    /// Error rate threshold that triggers a breaking point (0.0 - 1.0)
    pub error_rate_threshold: f64,
    /// P99 latency threshold in ms that triggers a breaking point
    pub latency_threshold_ms: u64,
    /// Whether to stop immediately when breaking point is detected
    pub stop_on_failure: bool,
    /// Timeout for individual workflow completion
    pub workflow_timeout: Duration,
}

impl Default for StressTestConfig {
    fn default() -> Self {
        Self {
            initial_concurrent: 100,
            peak_concurrent: 10_000,
            step_size: 500,
            step_duration: Duration::from_secs(30),
            cooldown: Duration::from_secs(5),
            workflow_definition: "sequential_bench_5".to_string(),
            error_rate_threshold: 0.05,  // 5% error rate
            latency_threshold_ms: 15000, // 15 second P99
            stop_on_failure: true,
            workflow_timeout: Duration::from_secs(60),
        }
    }
}

impl StressTestConfig {
    /// Create a quick stress test config (100 -> 1,000)
    pub fn quick() -> Self {
        Self {
            initial_concurrent: 100,
            peak_concurrent: 1_000,
            step_size: 100,
            step_duration: Duration::from_secs(15),
            cooldown: Duration::from_secs(3),
            ..Default::default()
        }
    }

    /// Create a standard stress test config (100 -> 5,000)
    pub fn standard() -> Self {
        Self {
            initial_concurrent: 100,
            peak_concurrent: 5_000,
            step_size: 300,
            step_duration: Duration::from_secs(30),
            cooldown: Duration::from_secs(5),
            ..Default::default()
        }
    }

    /// Create a full stress test config (100 -> 10,000)
    pub fn full() -> Self {
        Self::default()
    }

    /// Calculate the number of steps in this test
    pub fn num_steps(&self) -> usize {
        if self.peak_concurrent <= self.initial_concurrent {
            return 1;
        }
        ((self.peak_concurrent - self.initial_concurrent) / self.step_size) + 1
    }

    /// Calculate the estimated duration of the test
    pub fn estimated_duration(&self) -> Duration {
        let steps = self.num_steps();
        Duration::from_secs(
            (steps as u64) * (self.step_duration.as_secs() + self.cooldown.as_secs()),
        )
    }
}

/// Metrics collected during a single step of the stress test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepMetrics {
    /// Step number (0-indexed)
    pub step_number: usize,
    /// Target concurrent workflows for this step
    pub target_concurrent: usize,
    /// Actual peak concurrent workflows achieved
    pub actual_concurrent: usize,
    /// Total workflows submitted during this step
    pub total_workflows: usize,
    /// Successfully completed workflows
    pub successful_workflows: usize,
    /// Failed workflows (errors or timeouts)
    pub failed_workflows: usize,
    /// Throughput in workflows per second
    pub throughput_wf_per_sec: f64,
    /// Success rate (0.0 - 1.0)
    pub success_rate: f64,
    /// P50 latency in milliseconds
    pub p50_latency_ms: u64,
    /// P95 latency in milliseconds
    pub p95_latency_ms: u64,
    /// P99 latency in milliseconds
    pub p99_latency_ms: u64,
    /// Average CPU utilization during step (0.0 - 100.0)
    pub cpu_percent: f64,
    /// Peak memory usage in MB
    pub memory_mb: f64,
    /// Database connection count
    pub db_connections: u32,
    /// Duration of this step
    pub duration: Duration,
    /// Error messages encountered
    pub errors: Vec<String>,
    /// Timestamp when step started
    pub started_at: DateTime<Utc>,
}

impl StepMetrics {
    /// Check if this step exceeded the error rate threshold
    pub fn exceeded_error_threshold(&self, threshold: f64) -> bool {
        (1.0 - self.success_rate) > threshold
    }

    /// Check if this step exceeded the latency threshold
    pub fn exceeded_latency_threshold(&self, threshold_ms: u64) -> bool {
        self.p99_latency_ms > threshold_ms
    }
}

/// Describes why the system hit its breaking point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureMode {
    /// Error rate exceeded the configured threshold
    ErrorRateExceeded { actual_rate: f64, threshold: f64 },
    /// P99 latency exceeded the configured threshold
    LatencyExceeded { actual_ms: u64, threshold_ms: u64 },
    /// Throughput degraded significantly from baseline
    ThroughputDegraded {
        current_wf_per_sec: f64,
        baseline_wf_per_sec: f64,
        degradation_percent: f64,
    },
    /// System resource exhausted
    ResourceExhausted {
        resource: String,
        current_value: f64,
        threshold: f64,
    },
}

impl std::fmt::Display for FailureMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailureMode::ErrorRateExceeded {
                actual_rate,
                threshold,
            } => {
                write!(
                    f,
                    "Error rate {:.1}% exceeded threshold {:.1}%",
                    actual_rate * 100.0,
                    threshold * 100.0
                )
            }
            FailureMode::LatencyExceeded {
                actual_ms,
                threshold_ms,
            } => {
                write!(
                    f,
                    "P99 latency {}ms exceeded threshold {}ms",
                    actual_ms, threshold_ms
                )
            }
            FailureMode::ThroughputDegraded {
                current_wf_per_sec,
                baseline_wf_per_sec,
                degradation_percent,
            } => {
                write!(
                    f,
                    "Throughput degraded {:.1}% ({:.1} -> {:.1} wf/sec)",
                    degradation_percent, baseline_wf_per_sec, current_wf_per_sec
                )
            }
            FailureMode::ResourceExhausted {
                resource,
                current_value,
                threshold,
            } => {
                write!(
                    f,
                    "{} exhausted: {:.1} exceeded threshold {:.1}",
                    resource, current_value, threshold
                )
            }
        }
    }
}

/// Information about when and why the system hit its breaking point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingPoint {
    /// Number of concurrent workflows at the breaking point
    pub concurrent_workflows: usize,
    /// The failure mode that triggered the breaking point
    pub failure_mode: FailureMode,
    /// Metrics at the time of failure
    pub metrics: StepMetrics,
}

/// Complete results from a stress test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressTestResults {
    /// Configuration used for the test
    pub config: StressTestConfig,
    /// Metrics for each step of the test
    pub steps: Vec<StepMetrics>,
    /// Breaking point information (if detected)
    pub breaking_point: Option<BreakingPoint>,
    /// Peak throughput achieved during the test
    pub peak_throughput_wf_per_sec: f64,
    /// Maximum concurrent workflows achieved
    pub max_concurrent_achieved: usize,
    /// Total duration of the test
    pub total_duration: Duration,
    /// Timestamp when test started
    pub started_at: DateTime<Utc>,
    /// Timestamp when test completed
    pub completed_at: DateTime<Utc>,
}

impl StressTestResults {
    /// Generate a summary string for display
    pub fn summary(&self) -> String {
        let mut summary = String::new();
        summary.push_str(&format!("\n{}\n", "=".repeat(70)));
        summary.push_str("STRESS TEST RESULTS\n");
        summary.push_str(&format!("{}\n\n", "=".repeat(70)));

        summary.push_str(&format!(
            "Duration:            {:.1}s\n",
            self.total_duration.as_secs_f64()
        ));
        summary.push_str(&format!(
            "Steps Completed:     {}/{}\n",
            self.steps.len(),
            self.config.num_steps()
        ));
        summary.push_str(&format!(
            "Peak Throughput:     {:.2} wf/sec\n",
            self.peak_throughput_wf_per_sec
        ));
        summary.push_str(&format!(
            "Max Concurrent:      {}\n",
            self.max_concurrent_achieved
        ));

        if let Some(ref bp) = self.breaking_point {
            summary.push_str(&format!(
                "\nBREAKING POINT DETECTED at {} concurrent workflows\n",
                bp.concurrent_workflows
            ));
            summary.push_str(&format!("  Reason: {}\n", bp.failure_mode));
        } else {
            summary.push_str(&format!(
                "\nNo breaking point detected (reached {} concurrent)\n",
                self.config.peak_concurrent
            ));
        }

        summary.push_str(&format!("\n{}\n", "=".repeat(70)));
        summary
    }

    /// Save results to a JSON file
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }
}

/// Run a ramping stress test.
pub async fn run_stress_test(
    client: &StreamFlowClient,
    config: StressTestConfig,
    resource_samples: Option<Arc<tokio::sync::RwLock<Vec<crate::monitor::ResourceSample>>>>,
) -> StressTestResults {
    let started_at = Utc::now();
    let start_time = Instant::now();
    let mut steps: Vec<StepMetrics> = Vec::new();
    let mut breaking_point: Option<BreakingPoint> = None;
    let mut peak_throughput = 0.0_f64;
    let mut max_concurrent = 0_usize;

    let mut current_concurrent = config.initial_concurrent;
    let mut step_number = 0;

    println!(
        "\nStarting stress test: {} -> {} concurrent workflows",
        config.initial_concurrent, config.peak_concurrent
    );
    println!(
        "Estimated duration: {:.0}s ({} steps)\n",
        config.estimated_duration().as_secs_f64(),
        config.num_steps()
    );

    // Pre-fetch OAuth token to prevent thundering herd at test start
    println!("Pre-fetching OAuth token...");
    if let Err(e) = client.prefetch_token().await {
        eprintln!("Warning: Failed to prefetch token: {}", e);
        eprintln!("Continuing anyway - token will be fetched on first request");
    } else {
        println!("Token cached successfully\n");
    }

    // Collect baseline metrics from first step
    let mut baseline_throughput: Option<f64> = None;

    while current_concurrent <= config.peak_concurrent {
        println!(
            "Step {}: Running with {} concurrent workflows...",
            step_number + 1,
            current_concurrent
        );

        let step_metrics = run_step(
            client,
            &config,
            step_number,
            current_concurrent,
            &resource_samples,
        )
        .await;

        // Update peak metrics
        if step_metrics.throughput_wf_per_sec > peak_throughput {
            peak_throughput = step_metrics.throughput_wf_per_sec;
        }
        if step_metrics.actual_concurrent > max_concurrent {
            max_concurrent = step_metrics.actual_concurrent;
        }

        // Set baseline from first step
        if baseline_throughput.is_none() {
            baseline_throughput = Some(step_metrics.throughput_wf_per_sec);
        }

        // Print step summary
        println!(
            "  Throughput: {:.2} wf/sec | Success: {:.1}% | P99: {}ms",
            step_metrics.throughput_wf_per_sec,
            step_metrics.success_rate * 100.0,
            step_metrics.p99_latency_ms
        );

        // Check for breaking point
        let failure_mode = detect_failure(&step_metrics, &config, baseline_throughput.unwrap());
        if let Some(mode) = failure_mode {
            println!("\n  BREAKING POINT DETECTED: {}\n", mode);
            breaking_point = Some(BreakingPoint {
                concurrent_workflows: current_concurrent,
                failure_mode: mode,
                metrics: step_metrics.clone(),
            });

            if config.stop_on_failure {
                steps.push(step_metrics);
                break;
            }
        }

        steps.push(step_metrics);

        // Cooldown before next step
        if current_concurrent < config.peak_concurrent {
            println!("  Cooldown for {}s...", config.cooldown.as_secs());
            tokio::time::sleep(config.cooldown).await;
        }

        current_concurrent += config.step_size;
        step_number += 1;
    }

    let completed_at = Utc::now();
    let total_duration = start_time.elapsed();

    let results = StressTestResults {
        config,
        steps,
        breaking_point,
        peak_throughput_wf_per_sec: peak_throughput,
        max_concurrent_achieved: max_concurrent,
        total_duration,
        started_at,
        completed_at,
    };

    println!("{}", results.summary());
    results
}

/// Run a single step of the stress test.
async fn run_step(
    client: &StreamFlowClient,
    config: &StressTestConfig,
    step_number: usize,
    target_concurrent: usize,
    resource_samples: &Option<Arc<tokio::sync::RwLock<Vec<crate::monitor::ResourceSample>>>>,
) -> StepMetrics {
    let started_at = Utc::now();
    let step_start = Instant::now();
    let semaphore = Arc::new(Semaphore::new(target_concurrent));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let active_count = Arc::new(AtomicUsize::new(0));

    let mut tasks = Vec::new();
    let mut latencies: Vec<Duration> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    let mut peak_concurrent = 0_usize;
    let step_duration = config.step_duration;
    let workflow_timeout = config.workflow_timeout;

    // Spawn workflow submitter tasks
    while step_start.elapsed() < step_duration && !stop_flag.load(Ordering::Relaxed) {
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                // All permits in use, wait a bit
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            }
        };

        let client = client.clone();
        let definition_name = config.workflow_definition.clone();
        let active = active_count.clone();

        active.fetch_add(1, Ordering::SeqCst);
        let current_active = active.load(Ordering::SeqCst);
        if current_active > peak_concurrent {
            peak_concurrent = current_active;
        }

        let task = tokio::spawn(async move {
            let workflow_start = Instant::now();

            let result = client
                .create_workflow(&definition_name, serde_json::json!({}))
                .await;

            let workflow_id = match result {
                Ok(resp) => resp.workflow_id,
                Err(e) => {
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                    return (workflow_start.elapsed(), Err(e.to_string()));
                }
            };

            let completion = client
                .wait_for_completion(workflow_id, workflow_timeout)
                .await;

            active.fetch_sub(1, Ordering::SeqCst);
            drop(permit);

            match completion {
                Ok(status) if status.status == "completed" => (workflow_start.elapsed(), Ok(())),
                Ok(status) => (
                    workflow_start.elapsed(),
                    Err(format!(
                        "Workflow {} failed with status: {}",
                        workflow_id, status.status
                    )),
                ),
                Err(e) => (workflow_start.elapsed(), Err(e.to_string())),
            }
        });

        tasks.push(task);
    }

    // Signal stop and wait for remaining tasks
    stop_flag.store(true, Ordering::SeqCst);

    let mut successful = 0_usize;
    let mut failed = 0_usize;

    for task in tasks {
        match task.await {
            Ok((latency, Ok(()))) => {
                latencies.push(latency);
                successful += 1;
            }
            Ok((latency, Err(e))) => {
                latencies.push(latency);
                failed += 1;
                if errors.len() < 10 {
                    errors.push(e);
                }
            }
            Err(e) => {
                failed += 1;
                if errors.len() < 10 {
                    errors.push(format!("Task panic: {}", e));
                }
            }
        }
    }

    let duration = step_start.elapsed();
    let total = successful + failed;
    let success_rate = if total > 0 {
        successful as f64 / total as f64
    } else {
        0.0
    };
    let throughput = if duration.as_secs_f64() > 0.0 {
        total as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    // Calculate latency percentiles
    let mut latencies_ms: Vec<u64> = latencies.iter().map(|d| d.as_millis() as u64).collect();
    latencies_ms.sort();

    let p50 = percentile(&latencies_ms, 0.50);
    let p95 = percentile(&latencies_ms, 0.95);
    let p99 = percentile(&latencies_ms, 0.99);

    // Get resource metrics if available
    let (cpu_percent, memory_mb) = if let Some(samples) = resource_samples {
        let samples = samples.read().await;
        if samples.is_empty() {
            (0.0, 0.0)
        } else {
            let recent: Vec<_> = samples
                .iter()
                .filter(|s| s.timestamp >= started_at)
                .collect();
            if recent.is_empty() {
                (0.0, 0.0)
            } else {
                let avg_cpu =
                    recent.iter().map(|s| s.cpu_percent).sum::<f64>() / recent.len() as f64;
                let max_mem = recent
                    .iter()
                    .map(|s| s.memory_rss_mb)
                    .fold(0.0_f64, f64::max);
                (avg_cpu, max_mem)
            }
        }
    } else {
        (0.0, 0.0)
    };

    StepMetrics {
        step_number,
        target_concurrent,
        actual_concurrent: peak_concurrent,
        total_workflows: total,
        successful_workflows: successful,
        failed_workflows: failed,
        throughput_wf_per_sec: throughput,
        success_rate,
        p50_latency_ms: p50,
        p95_latency_ms: p95,
        p99_latency_ms: p99,
        cpu_percent,
        memory_mb,
        db_connections: 0, // TODO: Add database connection monitoring
        duration,
        errors,
        started_at,
    }
}

/// Detect if a step represents a breaking point.
fn detect_failure(
    metrics: &StepMetrics,
    config: &StressTestConfig,
    baseline_throughput: f64,
) -> Option<FailureMode> {
    // Check error rate
    let error_rate = 1.0 - metrics.success_rate;
    if error_rate > config.error_rate_threshold {
        return Some(FailureMode::ErrorRateExceeded {
            actual_rate: error_rate,
            threshold: config.error_rate_threshold,
        });
    }

    // Check latency
    if metrics.p99_latency_ms > config.latency_threshold_ms {
        return Some(FailureMode::LatencyExceeded {
            actual_ms: metrics.p99_latency_ms,
            threshold_ms: config.latency_threshold_ms,
        });
    }

    // Check throughput degradation (if we're past the first step)
    if metrics.step_number > 0 && baseline_throughput > 0.0 {
        let degradation =
            (baseline_throughput - metrics.throughput_wf_per_sec) / baseline_throughput;
        if degradation > 0.5 {
            // 50% degradation
            return Some(FailureMode::ThroughputDegraded {
                current_wf_per_sec: metrics.throughput_wf_per_sec,
                baseline_wf_per_sec: baseline_throughput,
                degradation_percent: degradation * 100.0,
            });
        }
    }

    None
}

fn percentile(sorted_values: &[u64], p: f64) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let index = ((sorted_values.len() as f64) * p) as usize;
    sorted_values[index.min(sorted_values.len() - 1)]
}
