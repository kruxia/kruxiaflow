use serial_test::serial;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

use streamflow_benchmark::client::StreamFlowClient;

/// Create authenticated client from environment variables
fn create_client() -> StreamFlowClient {
    let base_url =
        env::var("STREAMFLOW_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let client_id = env::var("STREAMFLOW_CLIENT_ID")
        .expect("STREAMFLOW_CLIENT_ID environment variable must be set");

    let client_secret = env::var("STREAMFLOW_CLIENT_SECRET")
        .expect("STREAMFLOW_CLIENT_SECRET environment variable must be set");

    StreamFlowClient::new(base_url, client_id, client_secret)
}

/// Performance metrics collected during load tests
#[derive(Debug, Clone)]
struct PerformanceMetrics {
    total_workflows: usize,
    successful_workflows: usize,
    failed_workflows: usize,
    duration: Duration,
    throughput_wf_per_sec: f64,
    success_rate: f64,
    #[allow(dead_code)]
    latencies_ms: Vec<u64>,
    p50_latency_ms: u64,
    p95_latency_ms: u64,
    p99_latency_ms: u64,
}

impl PerformanceMetrics {
    fn from_measurements(
        total_workflows: usize,
        successful_workflows: usize,
        failed_workflows: usize,
        duration: Duration,
        latencies: Vec<Duration>,
    ) -> Self {
        let throughput_wf_per_sec = total_workflows as f64 / duration.as_secs_f64();
        let success_rate = if total_workflows > 0 {
            (successful_workflows as f64 / total_workflows as f64) * 100.0
        } else {
            0.0
        };

        let mut latencies_ms: Vec<u64> = latencies.iter().map(|d| d.as_millis() as u64).collect();
        latencies_ms.sort();

        let p50_latency_ms = percentile(&latencies_ms, 0.50);
        let p95_latency_ms = percentile(&latencies_ms, 0.95);
        let p99_latency_ms = percentile(&latencies_ms, 0.99);

        Self {
            total_workflows,
            successful_workflows,
            failed_workflows,
            duration,
            throughput_wf_per_sec,
            success_rate,
            latencies_ms,
            p50_latency_ms,
            p95_latency_ms,
            p99_latency_ms,
        }
    }

    fn print_report(&self, scenario_name: &str) {
        println!("\n{}", "=".repeat(60));
        println!("Performance Test: {}", scenario_name);
        println!("{}", "=".repeat(60));
        println!("Total Workflows:     {}", self.total_workflows);
        println!("Successful:          {}", self.successful_workflows);
        println!("Failed:              {}", self.failed_workflows);
        println!("Success Rate:        {:.1}%", self.success_rate);
        println!("Duration:            {:.2}s", self.duration.as_secs_f64());
        println!(
            "Throughput:          {:.2} workflows/sec",
            self.throughput_wf_per_sec
        );
        println!("\nEnd-to-End Latency:");
        println!("  P50:               {} ms", self.p50_latency_ms);
        println!("  P95:               {} ms", self.p95_latency_ms);
        println!("  P99:               {} ms", self.p99_latency_ms);
        println!("{}\n", "=".repeat(60));
    }

    fn to_scenario_json(&self, scenario_name: &str) -> serde_json::Value {
        serde_json::json!({
            "name": scenario_name,
            "total_workflows": self.total_workflows,
            "successful_workflows": self.successful_workflows,
            "failed_workflows": self.failed_workflows,
            "success_rate": self.success_rate,
            "duration_seconds": self.duration.as_secs_f64(),
            "throughput_wf_per_sec": self.throughput_wf_per_sec,
            "latency_p50_ms": self.p50_latency_ms,
            "latency_p95_ms": self.p95_latency_ms,
            "latency_p99_ms": self.p99_latency_ms,
        })
    }
}

fn percentile(sorted_values: &[u64], p: f64) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let index = ((sorted_values.len() as f64) * p) as usize;
    sorted_values[index.min(sorted_values.len() - 1)]
}

/// Get current git SHA for versioning benchmark results
fn get_git_sha() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Save benchmark results to JSON file in the output directory
fn save_results(metrics: &PerformanceMetrics, scenario_name: &str) {
    // Check if BENCHMARK_OUTPUT_DIR environment variable is set
    let output_dir = match env::var("BENCHMARK_OUTPUT_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return, // Silently skip if not set (for normal test runs)
    };

    // Create output directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(&output_dir) {
        eprintln!("Failed to create output directory: {}", e);
        return;
    }

    let results_path = output_dir.join("results.json");

    // Read existing results or create new structure
    let mut results = if results_path.exists() {
        let contents = match fs::read_to_string(&results_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read existing results: {}", e);
                return;
            }
        };
        match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to parse existing results: {}", e);
                return;
            }
        }
    } else {
        // Create new results structure
        serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "git_sha": get_git_sha(),
            "scenarios": []
        })
    };

    // Add this scenario to the scenarios array
    if let Some(scenarios) = results.get_mut("scenarios").and_then(|s| s.as_array_mut()) {
        scenarios.push(metrics.to_scenario_json(scenario_name));
    }

    // Write results back to file
    let json_str = match serde_json::to_string_pretty(&results) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to serialize results: {}", e);
            return;
        }
    };

    if let Err(e) = fs::write(&results_path, json_str) {
        eprintln!("Failed to write results file: {}", e);
    }
}

#[tokio::test]
#[serial]
async fn test_sequential_workflow_load() {
    let client = create_client();
    let definition_name = "sequential_bench_5";
    let num_workflows = 100;

    let metrics = run_workflow_load_test(&client, definition_name, num_workflows, 10).await;

    let scenario_name = "Sequential Workflow (5 activities, 100 workflows)";
    metrics.print_report(scenario_name);
    save_results(&metrics, scenario_name);

    // Assert performance targets
    assert!(
        metrics.throughput_wf_per_sec >= 100.0,
        "Expected >= 100 wf/sec, got {:.2}",
        metrics.throughput_wf_per_sec
    );
    assert!(
        metrics.p99_latency_ms <= 100,
        "Expected P99 latency <= 100ms, got {}ms",
        metrics.p99_latency_ms
    );
}

#[tokio::test]
#[serial]
async fn test_parallel_workflow_load() {
    let client = create_client();

    let definition_name = "parallel_bench_10";
    let num_workflows = 50;

    let metrics = run_workflow_load_test(&client, definition_name, num_workflows, 10).await;

    let scenario_name = "Parallel Workflow (10 parallel activities, 50 workflows)";
    metrics.print_report(scenario_name);
    save_results(&metrics, scenario_name);

    // Assert performance targets
    assert!(
        metrics.throughput_wf_per_sec >= 50.0,
        "Expected >= 50 wf/sec, got {:.2}",
        metrics.throughput_wf_per_sec
    );
    assert!(
        metrics.p99_latency_ms <= 200,
        "Expected P99 latency <= 200ms, got {}ms",
        metrics.p99_latency_ms
    );
}

#[tokio::test]
#[serial]
async fn test_high_concurrency_load() {
    let client = create_client();

    let definition_name = "sequential_bench_3";
    let num_workflows = 300;

    let metrics = run_workflow_load_test(&client, definition_name, num_workflows, 100).await;

    let scenario_name = "High Concurrency (3 activities, 300 workflows, 100 concurrent)";
    metrics.print_report(scenario_name);
    save_results(&metrics, scenario_name);

    // Assert performance targets for high concurrency
    assert!(
        metrics.throughput_wf_per_sec >= 200.0,
        "Expected >= 200 wf/sec, got {:.2}",
        metrics.throughput_wf_per_sec
    );
    assert!(
        metrics.p99_latency_ms <= 150,
        "Expected P99 latency <= 150ms, got {}ms",
        metrics.p99_latency_ms
    );
}

#[tokio::test]
#[serial]
async fn test_sustained_throughput() {
    let client = create_client();

    let definition_name = "sequential_bench_5";

    // Run for 60 seconds to test sustained performance
    let duration = Duration::from_secs(60);
    let metrics = run_sustained_load_test(&client, definition_name, duration, 20).await;

    let scenario_name = "Sustained Throughput (60 seconds, 20 concurrent)";
    metrics.print_report(scenario_name);
    save_results(&metrics, scenario_name);

    // Target: >100 workflows/sec sustained
    assert!(
        metrics.throughput_wf_per_sec >= 100.0,
        "Expected >= 100 wf/sec sustained, got {:.2}",
        metrics.throughput_wf_per_sec
    );
}

/// Run workflow load test via HTTP API
async fn run_workflow_load_test(
    client: &StreamFlowClient,
    definition_name: &str,
    num_workflows: usize,
    max_concurrent: usize,
) -> PerformanceMetrics {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut tasks = Vec::new();

    let start_time = Instant::now();

    for _ in 0..num_workflows {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let definition_name = definition_name.to_string();

        let task = tokio::spawn(async move {
            let workflow_start = Instant::now();

            // Create workflow via HTTP API
            let response = client
                .create_workflow(&definition_name, serde_json::json!({}))
                .await;

            let workflow_id = match response {
                Ok(resp) => resp.workflow_id,
                Err(e) => {
                    eprintln!("Failed to create workflow: {}", e);
                    drop(permit);
                    return (workflow_start.elapsed(), false);
                }
            };

            // Wait for workflow completion via HTTP polling
            let completion_result = client
                .wait_for_completion(workflow_id, Duration::from_secs(30))
                .await;

            let success = match completion_result {
                Ok(status) => status.status == "completed",
                Err(e) => {
                    eprintln!("Workflow {} failed: {}", workflow_id, e);
                    false
                }
            };

            drop(permit);
            (workflow_start.elapsed(), success)
        });

        tasks.push(task);
    }

    // Collect all results
    let mut latencies = Vec::new();
    let mut success_count = 0;
    let mut failure_count = 0;

    for task in tasks {
        let (latency, success) = task.await.expect("Task failed");
        latencies.push(latency);
        if success {
            success_count += 1;
        } else {
            failure_count += 1;
        }
    }

    let total_duration = start_time.elapsed();

    PerformanceMetrics::from_measurements(
        num_workflows,
        success_count,
        failure_count,
        total_duration,
        latencies,
    )
}

/// Run sustained load test via HTTP API
async fn run_sustained_load_test(
    client: &StreamFlowClient,
    definition_name: &str,
    duration: Duration,
    max_concurrent: usize,
) -> PerformanceMetrics {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut tasks = Vec::new();
    let start_time = Instant::now();

    while start_time.elapsed() < duration {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let definition_name = definition_name.to_string();

        let task = tokio::spawn(async move {
            let workflow_start = Instant::now();

            // Create workflow via HTTP API
            let response = client
                .create_workflow(&definition_name, serde_json::json!({}))
                .await;

            let workflow_id = match response {
                Ok(resp) => resp.workflow_id,
                Err(e) => {
                    eprintln!("Failed to create workflow: {}", e);
                    drop(permit);
                    return (workflow_start.elapsed(), false);
                }
            };

            // Wait for workflow completion via HTTP polling
            let completion_result = client
                .wait_for_completion(workflow_id, Duration::from_secs(30))
                .await;

            let success = match completion_result {
                Ok(status) => status.status == "completed",
                Err(e) => {
                    eprintln!("Workflow {} failed: {}", workflow_id, e);
                    false
                }
            };

            drop(permit);
            (workflow_start.elapsed(), success)
        });

        tasks.push(task);
    }

    // Collect all results
    let mut latencies = Vec::new();
    let mut success_count = 0;
    let mut failure_count = 0;

    for task in tasks {
        let (latency, success) = task.await.expect("Task failed");
        latencies.push(latency);
        if success {
            success_count += 1;
        } else {
            failure_count += 1;
        }
    }

    let total_duration = start_time.elapsed();
    let workflow_count = latencies.len();

    PerformanceMetrics::from_measurements(
        workflow_count,
        success_count,
        failure_count,
        total_duration,
        latencies,
    )
}
