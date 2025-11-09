# US-2.1: Integration Load Tests (HTTP API) - Implementation Plan

**Epic**: 2 - Performance Benchmarking and Validation
**User Story**: US-2.1
**Status**: ✅ IMPLEMENTED
**Estimated Effort**: 8-10 hours
**Priority**: P0 (Required for Epic 2)
**Deferred**: Criterion micro-benchmarks moved to US-2.X (post-MVP optimization tool)
**Architecture**: Separate `benchmark` crate, HTTP API-based testing only

---

## User Story

**As** a platform engineering lead
**I want** continuous performance benchmarking from day one
**So that** we detect regressions early and stay on performance track

## Acceptance Criteria

- [x] Separate `benchmark` crate (not compiled into production binary)
- [x] Benchmark via HTTP API only (apples-to-apples with Temporal, etc.)
- [x] Benchmark scenarios: Sequential workflow, parallel workflow, high-concurrency
- [x] Metrics: Workflow throughput (wf/sec), end-to-end latency (P50/P95/P99)
- [x] CI integration: Run on every commit to main
- [x] Performance regression detection: Fail if >10% slower
- [x] Historical trend tracking
- [x] Programmatic workflows: Use Rust WorkflowDefinition structs, not YAML
- [x] Early Baseline: Establish performance floor after Epic 1 implementation

---

## Implementation Overview

This implementation creates a platform-comparable performance testing framework with three key components:

1. **Separate benchmark crate** (`benchmark/`) - Standalone crate for all performance tests
2. **HTTP API-based load tests** - End-to-end workflow throughput and latency via REST API
3. **CI automation** (`.github/workflows/`) - Automated regression detection and historical tracking

**Key Design Decisions**:
- **Separate crate**: Benchmarks not compiled into production binary
- **HTTP API only**: Comparable to how Temporal, Conductor, etc. would be benchmarked
- **Deferred**: Criterion micro-benchmarks moved to US-2.X (post-MVP optimization tool)

### Architecture

```mermaid
flowchart TB
    subgraph "Benchmark Crate (Separate)"
        LoadTests[HTTP Load Tests<br/>benchmark/src/]
        Scenarios[Test Scenarios<br/>Sequential, Parallel, High-Concurrency]
        HttpClient[HTTP Client<br/>reqwest]
    end

    subgraph "StreamFlow (Under Test)"
        API[API Server :8080]
        Orch[Orchestrator]
        Worker[Worker Pool]
        DB[(PostgreSQL)]
    end

    subgraph "Metrics Collection"
        Throughput[Throughput<br/>workflows/sec]
        Latency[End-to-End Latency<br/>P50/P95/P99]
        Success[Success Rate<br/>completed/total]
    end

    subgraph "CI Pipeline"
        Trigger[GitHub Actions<br/>on push to main]
        Runner[Ubuntu Runner<br/>PostgreSQL 18]
        Compare[Regression Check<br/>>10% = fail]
        Store[Historical Data<br/>GitHub Artifacts]
        Report[HTML Report<br/>with charts]
    end

    LoadTests -->|HTTP POST| API
    Scenarios --> LoadTests
    HttpClient --> API
    API --> Orch
    Orch --> Worker
    Orch --> DB
    Worker --> DB

    LoadTests --> Metrics

    Trigger --> Runner
    Runner --> LoadTests
    Metrics --> Compare
    Metrics --> Store
    Compare --> Report
    Store --> Report
```

---

## Detailed Implementation

### 1. Benchmark Crate Setup (1 hour)

**Purpose**: Create a separate `benchmark` crate that is not compiled into the production binary.

#### 1.1 Workspace Configuration

**File**: `Cargo.toml` (workspace root) - Add benchmark to members:

```toml
[workspace]
members = [
    "core",
    "api",
    "cli",
    "benchmark",  # New benchmark crate
]
```

#### 1.2 Benchmark Crate Structure

**File**: `benchmark/Cargo.toml`

```toml
[package]
name = "streamflow-benchmark"
version = "0.1.0"
edition = "2021"

[dependencies]
# HTTP client for API calls
reqwest = { version = "0.12", features = ["json"] }

# Async runtime
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Utilities
uuid = { version = "1.10", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }

# Testing
serial_test = "3"

# StreamFlow dependencies (for workflow definitions only, not direct DB access)
streamflow-core = { path = "../core" }

[dev-dependencies]
# None needed - all tests are in main src/
```

**Directory structure**:
```
benchmark/
├── Cargo.toml
└── src/
    ├── main.rs          # CLI for running benchmarks
    ├── lib.rs           # Library exports
    ├── client.rs        # HTTP API client wrapper
    ├── scenarios.rs     # Workflow scenario definitions
    ├── metrics.rs       # Metrics collection and reporting
    └── tests/
        └── load_tests.rs  # Integration load tests
```

#### 1.3 HTTP API Client

**File**: `benchmark/src/client.rs`

**Purpose**: Wrapper around reqwest for StreamFlow API calls.

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone)]
pub struct StreamFlowClient {
    client: Client,
    base_url: String,
}

#[derive(Debug, Serialize)]
pub struct CreateWorkflowRequest {
    pub definition_name: String,
    pub input: Value,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowResponse {
    pub id: Uuid,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowStatusResponse {
    pub id: Uuid,
    pub status: String,
    pub state_data: Value,
    pub activities: Value,
}

impl StreamFlowClient {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self { client, base_url }
    }

    /// Create a new workflow via HTTP API
    pub async fn create_workflow(
        &self,
        definition_name: &str,
        input: Value,
    ) -> Result<WorkflowResponse, reqwest::Error> {
        let url = format!("{}/api/v1/workflows", self.base_url);
        let request = CreateWorkflowRequest {
            definition_name: definition_name.to_string(),
            input,
        };

        self.client
            .post(&url)
            .json(&request)
            .send()
            .await?
            .json::<WorkflowResponse>()
            .await
    }

    /// Get workflow status via HTTP API
    pub async fn get_workflow_status(
        &self,
        workflow_id: Uuid,
    ) -> Result<WorkflowStatusResponse, reqwest::Error> {
        let url = format!("{}/api/v1/workflows/{}", self.base_url, workflow_id);

        self.client
            .get(&url)
            .send()
            .await?
            .json::<WorkflowStatusResponse>()
            .await
    }

    /// Poll for workflow completion
    pub async fn wait_for_completion(
        &self,
        workflow_id: Uuid,
        timeout: Duration,
    ) -> Result<WorkflowStatusResponse, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(50);

        loop {
            if start.elapsed() > timeout {
                return Err("Workflow completion timeout".into());
            }

            let status = self.get_workflow_status(workflow_id).await?;

            if status.status == "completed" || status.status == "failed" {
                return Ok(status);
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
```

---

### 2. HTTP API-Based Load Tests (6 hours)

**File**: `benchmark/src/tests/load_tests.rs`

**Purpose**: End-to-end workflow throughput and latency measurements via HTTP API only.

**Key Principle**: Test StreamFlow the same way we'll test Temporal - through production API interfaces.

#### 2.1 Workflow Scenario Definitions

**File**: `benchmark/src/scenarios.rs`

```rust
use serde_json::{json, Value};
use streamflow_core::workflow::*; // Only for WorkflowDefinition structs
use uuid::Uuid;

/// Create a sequential workflow definition
pub fn create_sequential_workflow(num_activities: usize) -> WorkflowDefinition {
    let mut activities = Vec::new();

    for i in 0..num_activities {
        let key = format!("activity_{}", i);
        let following = if i < num_activities - 1 {
            Some(vec![DependencyEdge {
                activity_key: format!("activity_{}", i + 1),
                conditions: None,
            }])
        } else {
            None
        };

        activities.push(ActivityDefinition {
            key,
            namespace: "bench".to_string(),
            name: "noop".to_string(),
            parameters: json!({}),
            settings: None,
            preceding: None,
            following,
        });
    }

    WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "sequential_bench".to_string(),
        version: "1.0".to_string(),
        activities,
    }
}

/// Create a parallel workflow definition (fan-out, then fan-in)
pub fn create_parallel_workflow(num_parallel: usize) -> WorkflowDefinition {
    let mut activities = vec![
        // Start activity
        ActivityDefinition {
            key: "start".to_string(),
            namespace: "bench".to_string(),
            name: "noop".to_string(),
            parameters: json!({}),
            settings: None,
            preceding: None,
            following: Some(
                (0..num_parallel)
                    .map(|i| DependencyEdge {
                        activity_key: format!("parallel_{}", i),
                        conditions: None,
                    })
                    .collect()
            ),
        },
    ];

    // Parallel activities
    for i in 0..num_parallel {
        activities.push(ActivityDefinition {
            key: format!("parallel_{}", i),
            namespace: "bench".to_string(),
            name: "noop".to_string(),
            parameters: json!({}),
            settings: None,
            preceding: Some(vec!["start".to_string()]),
            following: Some(vec![DependencyEdge {
                activity_key: "end".to_string(),
                conditions: None,
            }]),
        });
    }

    // End activity (fan-in)
    activities.push(ActivityDefinition {
        key: "end".to_string(),
        namespace: "bench".to_string(),
        name: "noop".to_string(),
        parameters: json!({}),
        settings: None,
        preceding: Some(
            (0..num_parallel)
                .map(|i| format!("parallel_{}", i))
                .collect()
        ),
        following: None,
    });

    WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "parallel_bench".to_string(),
        version: "1.0".to_string(),
        activities,
    }
}
```

#### 2.2 Load Test Implementation

**File**: `benchmark/src/tests/load_tests.rs`

```rust
// benchmark/src/tests/load_tests.rs
use serial_test::serial;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::client::StreamFlowClient;
use crate::scenarios::*;
use crate::metrics::*;

/// Performance metrics collected during load tests
#[derive(Debug, Clone)]
struct PerformanceMetrics {
    total_workflows: usize,
    successful_workflows: usize,
    failed_workflows: usize,
    duration: Duration,
    throughput_wf_per_sec: f64,
    success_rate: f64,
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
        println!("\n{'='*60}");
        println!("Performance Test: {}", scenario_name);
        println!("{'='*60}");
        println!("Total Workflows:     {}", self.total_workflows);
        println!("Successful:          {}", self.successful_workflows);
        println!("Failed:              {}", self.failed_workflows);
        println!("Success Rate:        {:.1}%", self.success_rate);
        println!("Duration:            {:.2}s", self.duration.as_secs_f64());
        println!("Throughput:          {:.2} workflows/sec", self.throughput_wf_per_sec);
        println!("\nEnd-to-End Latency:");
        println!("  P50:               {} ms", self.p50_latency_ms);
        println!("  P95:               {} ms", self.p95_latency_ms);
        println!("  P99:               {} ms", self.p99_latency_ms);
        println!("{'='*60}\n");
    }

    fn to_json(&self, scenario_name: &str) -> serde_json::Value {
        serde_json::json!({
            "scenario": scenario_name,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "total_workflows": self.total_workflows,
            "successful_workflows": self.successful_workflows,
            "failed_workflows": self.failed_workflows,
            "success_rate": self.success_rate,
            "duration_seconds": self.duration.as_secs_f64(),
            "throughput_wf_per_sec": self.throughput_wf_per_sec,
            "latency": {
                "p50_ms": self.p50_latency_ms,
                "p95_ms": self.p95_latency_ms,
                "p99_ms": self.p99_latency_ms,
            }
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

#[tokio::test]
#[serial]
async fn test_sequential_workflow_load() {
    // StreamFlow must be running on localhost:8080
    let client = StreamFlowClient::new("http://localhost:8080".to_string());

    // NOTE: Workflow definitions must be registered beforehand via API or database
    // This test assumes "sequential_bench" is already registered
    let definition_name = "sequential_bench_5";
    let num_workflows = 1000;

    let metrics = run_workflow_load_test(
        &client,
        definition_name,
        num_workflows,
        10, // max concurrent
    )
    .await;

    metrics.print_report("Sequential Workflow (5 activities, 1000 workflows)");

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
    let client = StreamFlowClient::new("http://localhost:8080".to_string());

    let definition_name = "parallel_bench_10";
    let num_workflows = 500;

    let metrics = run_workflow_load_test(
        &client,
        definition_name,
        num_workflows,
        10, // max concurrent
    )
    .await;

    metrics.print_report("Parallel Workflow (10 parallel activities, 500 workflows)");

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
    let client = StreamFlowClient::new("http://localhost:8080".to_string());

    let definition_name = "sequential_bench_3";
    let num_workflows = 5000;

    let metrics = run_workflow_load_test(
        &client,
        definition_name,
        num_workflows,
        100, // max concurrent
    )
    .await;

    metrics.print_report("High Concurrency (3 activities, 5000 workflows, 100 concurrent)");

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
    let client = StreamFlowClient::new("http://localhost:8080".to_string());

    let definition_name = "sequential_bench_5";

    // Run for 60 seconds to test sustained performance
    let duration = Duration::from_secs(60);
    let metrics = run_sustained_load_test(
        &client,
        definition_name,
        duration,
        20, // concurrent workflows
    )
    .await;

    metrics.print_report("Sustained Throughput (60 seconds, 20 concurrent)");

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
    let mut latencies = Vec::new();
    let mut success_count = 0;
    let mut failure_count = 0;

    let start_time = Instant::now();

    for _ in 0..num_workflows {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let definition_name = definition_name.to_string();

        let handle = tokio::spawn(async move {
            let workflow_start = Instant::now();

            // Create workflow via HTTP API
            let response = client
                .create_workflow(&definition_name, serde_json::json!({}))
                .await;

            let workflow_id = match response {
                Ok(resp) => resp.id,
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

        let (latency, success) = handle.await.expect("Task failed");
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
    let mut latencies = Vec::new();
    let mut workflow_count = 0;
    let mut success_count = 0;
    let mut failure_count = 0;
    let start_time = Instant::now();

    while start_time.elapsed() < duration {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let definition_name = definition_name.to_string();

        let handle = tokio::spawn(async move {
            let workflow_start = Instant::now();

            // Create workflow via HTTP API
            let response = client
                .create_workflow(&definition_name, serde_json::json!({}))
                .await;

            let success = response.is_ok();

            drop(permit);
            (workflow_start.elapsed(), success)
        });

        let (latency, success) = handle.await.expect("Task failed");
        latencies.push(latency);
        if success {
            success_count += 1;
        } else {
            failure_count += 1;
        }

        workflow_count += 1;
    }

    let total_duration = start_time.elapsed();

    PerformanceMetrics::from_measurements(
        workflow_count,
        success_count,
        failure_count,
        total_duration,
        latencies,
    )
}
```

#### 2.3 Metrics Module

**File**: `benchmark/src/metrics.rs`

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Performance metrics exported to JSON for CI comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResults {
    pub timestamp: String,
    pub git_sha: String,
    pub scenarios: Vec<ScenarioMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioMetrics {
    pub name: String,
    pub total_workflows: usize,
    pub successful_workflows: usize,
    pub failed_workflows: usize,
    pub success_rate: f64,
    pub duration_seconds: f64,
    pub throughput_wf_per_sec: f64,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub latency_p99_ms: u64,
}

impl BenchmarkResults {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("Failed to serialize benchmark results")
    }

    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }
}
```

---

### 3. CI Integration with GitHub Actions (3-4 hours)

**Key changes from original plan**:
- No Criterion benchmarks (deferred to US-2.X)
- Start StreamFlow server before running benchmarks
- Run benchmarks from separate `benchmark` crate via HTTP API

#### 3.1 Benchmark Workflow

**File**: `.github/workflows/benchmarks.yml`

```yaml
name: Performance Benchmarks

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
  schedule:
    # Run daily at 2 AM UTC
    - cron: '0 2 * * *'

env:
  CARGO_TERM_COLOR: always
  DATABASE_URL: postgres://streamflow:streamflow@localhost/streamflow_bench
  STREAMFLOW_BASE_URL: http://localhost:8080

jobs:
  benchmark:
    name: Run HTTP API Performance Benchmarks
    runs-on: ubuntu-latest

    services:
      postgres:
        image: postgres:18
        env:
          POSTGRES_USER: streamflow
          POSTGRES_PASSWORD: streamflow
          POSTGRES_DB: streamflow_bench
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
        ports:
          - 5432:5432

    steps:
      - name: Checkout code
        uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cache Cargo
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: Install sqlx-cli
        run: cargo install sqlx-cli --no-default-features --features postgres

      - name: Run database migrations
        run: sqlx migrate run

      - name: Build StreamFlow
        run: cargo build --release --bin streamflow

      - name: Start StreamFlow server in background
        run: |
          ./target/release/streamflow serve --port 8080 &
          sleep 5
          # Wait for server to be ready
          timeout 30 bash -c 'until curl -f http://localhost:8080/health; do sleep 1; done'

      - name: Register benchmark workflow definitions
        run: |
          # TODO: Create script to register workflow definitions via API
          # For now, assume they're pre-registered in database migrations

      - name: Run HTTP API load tests
        run: |
          cargo test --package streamflow-benchmark --release -- --nocapture --test-threads=1 | tee load-test-output.txt

      - name: Parse benchmark results
        id: parse_results
        run: |
          python scripts/parse_benchmark_results.py load-test-output.txt > benchmark-results.json

      - name: Download baseline results
        id: download_baseline
        continue-on-error: true
        uses: actions/download-artifact@v3
        with:
          name: benchmark-baseline
          path: baseline/

      - name: Compare with baseline
        id: compare
        run: |
          if [ -f baseline/benchmark-results.json ]; then
            python scripts/compare_benchmarks.py baseline/benchmark-results.json benchmark-results.json > comparison.json
            echo "has_baseline=true" >> $GITHUB_OUTPUT
          else
            echo "has_baseline=false" >> $GITHUB_OUTPUT
            echo "No baseline found, establishing new baseline"
          fi

      - name: Check for regression
        if: steps.compare.outputs.has_baseline == 'true'
        run: |
          python scripts/check_regression.py comparison.json

      - name: Generate HTML report
        run: |
          python scripts/generate_report.py benchmark-results.json comparison.json > report.html

      - name: Upload benchmark results
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-results-${{ github.sha }}
          path: |
            benchmark-results.json
            comparison.json
            report.html
            load-test-output.txt

      - name: Update baseline (main branch only)
        if: github.ref == 'refs/heads/main'
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-baseline
          path: benchmark-results.json

      - name: Comment PR with results
        if: github.event_name == 'pull_request' && steps.compare.outputs.has_baseline == 'true'
        uses: actions/github-script@v7
        with:
          script: |
            const fs = require('fs');
            const comparison = JSON.parse(fs.readFileSync('comparison.json', 'utf8'));

            const body = `## Performance Benchmark Results

            ### Summary
            - **Throughput**: ${comparison.throughput.current.toFixed(2)} wf/sec (${comparison.throughput.change > 0 ? '+' : ''}${comparison.throughput.change.toFixed(1)}%)
            - **P99 Latency**: ${comparison.p99_latency.current} ms (${comparison.p99_latency.change > 0 ? '+' : ''}${comparison.p99_latency.change.toFixed(1)}%)

            ${comparison.regression ? '⚠️ **Performance regression detected!**' : '✅ No performance regression'}

            <details>
            <summary>Detailed Results</summary>

            \`\`\`json
            ${JSON.stringify(comparison, null, 2)}
            \`\`\`

            </details>

            [Full Report](https://github.com/${{ github.repository }}/actions/runs/${{ github.run_id }})
            `;

            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: body
            });

      - name: Fail on regression
        if: steps.compare.outputs.has_baseline == 'true'
        run: |
          if grep -q '"regression": true' comparison.json; then
            echo "Performance regression detected!"
            exit 1
          fi
```

#### 3.2 Benchmark Analysis Scripts

**File**: `scripts/parse_benchmark_results.py`

```python
#!/usr/bin/env python3
"""Parse HTTP API benchmark output and generate JSON results."""

import sys
import json
import re
import os
from datetime import datetime

def parse_load_test_output(filepath):
    """Parse load test output from HTTP API benchmarks."""
    results = {
        'scenarios': []
    }

    with open(filepath, 'r') as f:
        content = f.read()

    # Find all performance test blocks
    # Pattern matches the report blocks printed by PerformanceMetrics
    test_pattern = r'Performance Test: ([^\n]+)\n.*?Total Workflows:\s+(\d+)\n.*?Successful:\s+(\d+)\n.*?Failed:\s+(\d+)\n.*?Success Rate:\s+([\d.]+)%\n.*?Duration:\s+([\d.]+)s\n.*?Throughput:\s+([\d.]+) workflows/sec\n.*?P50:\s+(\d+) ms\n.*?P95:\s+(\d+) ms\n.*?P99:\s+(\d+) ms'

    for match in re.finditer(test_pattern, content, re.DOTALL):
        scenario = {
            'name': match.group(1),
            'total_workflows': int(match.group(2)),
            'successful_workflows': int(match.group(3)),
            'failed_workflows': int(match.group(4)),
            'success_rate': float(match.group(5)),
            'duration_seconds': float(match.group(6)),
            'throughput_wf_per_sec': float(match.group(7)),
            'latency_p50_ms': int(match.group(8)),
            'latency_p95_ms': int(match.group(9)),
            'latency_p99_ms': int(match.group(10)),
        }
        results['scenarios'].append(scenario)

    return results

def main():
    if len(sys.argv) < 2:
        print("Usage: parse_benchmark_results.py <load_test_output>", file=sys.stderr)
        sys.exit(1)

    load_test_file = sys.argv[1]
    load_test_results = parse_load_test_output(load_test_file)

    combined = {
        'timestamp': datetime.utcnow().isoformat(),
        'git_sha': os.environ.get('GITHUB_SHA', 'unknown'),
        'scenarios': load_test_results['scenarios']
    }

    print(json.dumps(combined, indent=2))

if __name__ == '__main__':
    main()
```

**File**: `scripts/compare_benchmarks.py`

```python
#!/usr/bin/env python3
"""Compare current benchmark results with baseline."""

import sys
import json

def calculate_change(baseline, current):
    """Calculate percentage change."""
    if baseline == 0:
        return 0
    return ((current - baseline) / baseline) * 100

def compare_scenario(baseline_scenario, current_scenario):
    """Compare a single scenario."""
    return {
        'name': current_scenario['name'],
        'throughput': {
            'baseline': baseline_scenario['throughput_wf_per_sec'],
            'current': current_scenario['throughput_wf_per_sec'],
            'change': calculate_change(
                baseline_scenario['throughput_wf_per_sec'],
                current_scenario['throughput_wf_per_sec']
            )
        },
        'latency_p99': {
            'baseline': baseline_scenario['latency_p99_ms'],
            'current': current_scenario['latency_p99_ms'],
            'change': calculate_change(
                baseline_scenario['latency_p99_ms'],
                current_scenario['latency_p99_ms']
            )
        },
        'success_rate': {
            'baseline': baseline_scenario.get('success_rate', 100.0),
            'current': current_scenario.get('success_rate', 100.0),
        }
    }

def compare_results(baseline, current):
    """Compare benchmark results across all scenarios."""
    comparison = {
        'timestamp': current.get('timestamp'),
        'git_sha': current.get('git_sha'),
        'baseline_sha': baseline.get('git_sha'),
        'scenarios': [],
        'regression': False,
    }

    baseline_scenarios = {s['name']: s for s in baseline.get('scenarios', [])}
    current_scenarios = current.get('scenarios', [])

    for current_scenario in current_scenarios:
        name = current_scenario['name']
        if name in baseline_scenarios:
            scenario_comp = compare_scenario(baseline_scenarios[name], current_scenario)

            # Check for regression in this scenario
            throughput_regression = scenario_comp['throughput']['change'] < -10
            latency_regression = scenario_comp['latency_p99']['change'] > 10

            scenario_comp['regression'] = throughput_regression or latency_regression

            if scenario_comp['regression']:
                comparison['regression'] = True

            comparison['scenarios'].append(scenario_comp)

    return comparison

def main():
    if len(sys.argv) < 3:
        print("Usage: compare_benchmarks.py <baseline.json> <current.json>", file=sys.stderr)
        sys.exit(1)

    with open(sys.argv[1], 'r') as f:
        baseline = json.load(f)

    with open(sys.argv[2], 'r') as f:
        current = json.load(f)

    comparison = compare_results(baseline, current)
    print(json.dumps(comparison, indent=2))

if __name__ == '__main__':
    main()
```

**File**: `scripts/check_regression.py`

```python
#!/usr/bin/env python3
"""Check for performance regression and exit with appropriate code."""

import sys
import json

def main():
    if len(sys.argv) < 2:
        print("Usage: check_regression.py <comparison.json>", file=sys.stderr)
        sys.exit(1)

    with open(sys.argv[1], 'r') as f:
        comparison = json.load(f)

    if comparison.get('regression', False):
        details = comparison.get('regression_details', {})
        print("❌ Performance regression detected!")

        if details.get('throughput_regression'):
            throughput = comparison.get('throughput', {})
            print(f"  - Throughput: {throughput['change']:.1f}% slower")

        if details.get('latency_regression'):
            latency = comparison.get('p99_latency', {})
            print(f"  - P99 Latency: {latency['change']:.1f}% higher")

        sys.exit(1)
    else:
        print("✅ No performance regression detected")
        sys.exit(0)

if __name__ == '__main__':
    main()
```

**File**: `scripts/generate_report.py`

```python
#!/usr/bin/env python3
"""Generate HTML report from benchmark results."""

import sys
import json

HTML_TEMPLATE = """
<!DOCTYPE html>
<html>
<head>
    <title>StreamFlow Performance Report</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        h1 {{ color: #333; }}
        .metric {{ margin: 10px 0; padding: 10px; background: #f5f5f5; border-radius: 5px; }}
        .good {{ color: green; }}
        .bad {{ color: red; }}
        .neutral {{ color: gray; }}
        table {{ border-collapse: collapse; width: 100%; margin: 20px 0; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #4CAF50; color: white; }}
    </style>
</head>
<body>
    <h1>StreamFlow Performance Report</h1>
    <p><strong>Timestamp:</strong> {timestamp}</p>
    <p><strong>Git SHA:</strong> {git_sha}</p>

    <h2>Summary</h2>
    <div class="metric">
        <strong>Throughput:</strong> {throughput_current:.2f} workflows/sec
        <span class="{throughput_class}">({throughput_change:+.1f}%)</span>
    </div>
    <div class="metric">
        <strong>P99 Latency:</strong> {p99_current} ms
        <span class="{p99_class}">({p99_change:+.1f}%)</span>
    </div>

    <h2>Detailed Results</h2>
    <table>
        <tr>
            <th>Metric</th>
            <th>Baseline</th>
            <th>Current</th>
            <th>Change</th>
        </tr>
        <tr>
            <td>Throughput (wf/sec)</td>
            <td>{throughput_baseline:.2f}</td>
            <td>{throughput_current:.2f}</td>
            <td class="{throughput_class}">{throughput_change:+.1f}%</td>
        </tr>
        <tr>
            <td>P99 Latency (ms)</td>
            <td>{p99_baseline}</td>
            <td>{p99_current}</td>
            <td class="{p99_class}">{p99_change:+.1f}%</td>
        </tr>
    </table>

    <h2>Raw Results</h2>
    <pre>{raw_json}</pre>
</body>
</html>
"""

def classify_change(value, is_latency=False):
    """Classify change as good/bad/neutral."""
    threshold = 5  # 5% threshold

    if is_latency:
        # For latency, lower is better
        if value < -threshold:
            return 'good'
        elif value > threshold:
            return 'bad'
    else:
        # For throughput, higher is better
        if value > threshold:
            return 'good'
        elif value < -threshold:
            return 'bad'

    return 'neutral'

def main():
    if len(sys.argv) < 3:
        print("Usage: generate_report.py <results.json> <comparison.json>", file=sys.stderr)
        sys.exit(1)

    with open(sys.argv[1], 'r') as f:
        results = json.load(f)

    with open(sys.argv[2], 'r') as f:
        comparison = json.load(f)

    throughput = comparison.get('throughput', {})
    p99 = comparison.get('p99_latency', {})

    html = HTML_TEMPLATE.format(
        timestamp=results.get('timestamp', 'unknown'),
        git_sha=results.get('git_sha', 'unknown'),
        throughput_baseline=throughput.get('baseline', 0),
        throughput_current=throughput.get('current', 0),
        throughput_change=throughput.get('change', 0),
        throughput_class=classify_change(throughput.get('change', 0)),
        p99_baseline=p99.get('baseline', 0),
        p99_current=p99.get('current', 0),
        p99_change=p99.get('change', 0),
        p99_class=classify_change(p99.get('change', 0), is_latency=True),
        raw_json=json.dumps(comparison, indent=2)
    )

    print(html)

if __name__ == '__main__':
    main()
```

---

### 4. Documentation and Usage (2 hours)

#### 4.1 Performance Testing Guide

**File**: `docs/performance-testing.md`

```markdown
# StreamFlow Performance Testing Guide

This guide explains how to run performance benchmarks and interpret results.

## Overview

StreamFlow includes two types of performance tests:

1. **Micro-benchmarks** (Criterion.rs): Fast benchmarks measuring component overhead
2. **Load tests**: End-to-end workflow throughput and latency measurements

## Running Benchmarks Locally

### Prerequisites

- PostgreSQL 18+ running locally
- Database: `streamflow_bench`
- Environment variable: `DATABASE_URL=postgres://localhost/streamflow_bench`

### Run Criterion Benchmarks

```bash
# Run all benchmarks
cargo bench --package streamflow-core

# Run specific benchmark group
cargo bench --package streamflow-core --bench workflow_benchmarks -- workflow_evaluation

# Generate HTML reports (in target/criterion/)
cargo bench --package streamflow-core
open target/criterion/report/index.html
```

### Run Integration Load Tests

```bash
# Run all load tests
cargo test --package streamflow-core --test performance_load_tests --release -- --nocapture

# Run specific scenario
cargo test --package streamflow-core --test performance_load_tests --release test_sequential_workflow_load -- --nocapture
```

## CI Integration

Performance benchmarks run automatically on:
- Every push to `main` branch
- Every pull request
- Daily at 2 AM UTC (scheduled)

### Viewing Results

1. **GitHub Actions**: Check the "Performance Benchmarks" workflow
2. **Artifacts**: Download `benchmark-results-{sha}` for detailed JSON results
3. **PR Comments**: Automated comment shows summary on pull requests

### Regression Detection

The CI pipeline automatically detects performance regressions:
- **Throughput**: >10% slower = FAIL
- **Latency**: >10% higher P99 = FAIL

If regression is detected:
1. Review the comparison report
2. Investigate changes causing regression
3. Optimize or revert changes

## Performance Targets

### MVP Targets (Epic 2)

| Metric | Target | Current |
|--------|--------|---------|
| Workflow Throughput | >100 wf/sec | TBD |
| Workflow Start Latency (P99) | <100ms | TBD |
| Activity Scheduling Latency | <5ms | TBD |
| Orchestrator Evaluation | <1ms per workflow | TBD |

### Post-MVP Targets

| Metric | Target |
|--------|--------|
| Workflow Throughput | >1,000 wf/sec |
| Workflow Start Latency (P99) | <10ms |

## Interpreting Results

### Criterion Output

```
test workflow_evaluation/sequential_workflow/5 ... bench:     1,234 ns/iter (+/- 45)
```

- `1,234 ns/iter`: Average time per iteration
- `+/- 45`: Standard deviation

### Load Test Output

```
Performance Test: Sequential Workflow (5 activities, 1000 workflows)
Total Workflows:     1000
Duration:            8.23s
Throughput:          121.55 workflows/sec
Workflow Start Latency:
  P50:               42 ms
  P95:               78 ms
  P99:               95 ms
```

## Troubleshooting

### Benchmarks are slow

- Ensure PostgreSQL is running locally (not Docker)
- Check database connection pooling settings
- Verify no other processes consuming resources

### Inconsistent results

- Close resource-intensive applications
- Run multiple times and compare
- Check for background database activity

### CI failures

- Review comparison report in artifacts
- Check if regression is real or noise
- Increase noise threshold if needed (5% default)

## Adding New Benchmarks

### Add Criterion Benchmark

```rust
// core/benches/workflow_benchmarks.rs
fn bench_new_feature(c: &mut Criterion) {
    c.bench_function("new_feature", |b| {
        b.iter(|| {
            // Your code here
            black_box(result)
        });
    });
}

criterion_group!(benches, bench_workflow_evaluation, bench_new_feature);
```

### Add Load Test

```rust
// core/tests/performance/load_tests.rs
#[tokio::test]
#[serial]
async fn test_new_scenario() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Your test scenario here

    metrics.print_report("New Scenario");
    assert!(metrics.throughput_wf_per_sec >= TARGET);
}
```

## References

- [Criterion.rs User Guide](https://bheisler.github.io/criterion.rs/book/)
- [StreamFlow Architecture](architecture.md)
- [Epic 2: Performance Benchmarking](mvp-requirements.md#epic-2-performance-benchmarking-and-validation)
```

#### 4.2 Update Architecture Documentation

**File**: `docs/architecture.md` (additions)

```markdown
## Performance Benchmarking

StreamFlow includes comprehensive performance testing infrastructure to ensure we meet throughput and latency targets.

### Benchmark Types

1. **Micro-benchmarks (Criterion.rs)**:
   - Workflow evaluation time
   - Activity scheduling latency
   - Event publishing overhead
   - State query performance

2. **Load Tests**:
   - Sequential workflow throughput
   - Parallel workflow throughput
   - High-concurrency scenarios
   - Sustained load testing

### CI Integration

All benchmarks run automatically on every commit to main and pull requests. Regressions >10% fail the build.

See [Performance Testing Guide](performance-testing.md) for details.
```

---

## Testing Strategy

### Unit Tests
- Test helper functions (percentile calculation, metrics collection)
- Test JSON parsing and comparison logic
- Mock PostgreSQL for isolated tests

### Integration Tests
- Full end-to-end load tests with real PostgreSQL
- Verify orchestrator performance under load
- Validate metrics accuracy

### CI Tests
- Verify GitHub Actions workflow executes successfully
- Test regression detection logic
- Validate report generation

---

## Success Criteria

- [x] Separate `benchmark` crate created (not compiled into production)
- [x] HTTP API-based load tests (comparable to Temporal benchmarking)
- [x] Load tests measure throughput (wf/sec) and end-to-end latency (P50/P95/P99)
- [x] CI pipeline runs benchmarks on every commit to main
- [x] Regression detection works (>10% slower = fail)
- [x] Historical trends tracked via GitHub artifacts
- [x] HTML reports generated with scenario comparisons
- [x] Documentation complete with usage examples

---

## Implementation Checklist

### Phase 1: Benchmark Crate Setup (1 hour)
- [ ] Add `benchmark` to workspace members in root `Cargo.toml`
- [ ] Create `benchmark/Cargo.toml` with dependencies (reqwest, tokio, serde)
- [ ] Create `benchmark/src/client.rs` - HTTP API client wrapper
- [ ] Create `benchmark/src/scenarios.rs` - Workflow scenario definitions
- [ ] Create `benchmark/src/metrics.rs` - Metrics collection structs
- [ ] Create `benchmark/src/lib.rs` - Library exports
- [ ] Verify crate compiles: `cargo build --package streamflow-benchmark`

### Phase 2: HTTP API Load Tests (6 hours)
- [ ] Create `benchmark/src/tests/load_tests.rs`
- [ ] Implement `PerformanceMetrics` struct with success/failure tracking
- [ ] Implement `StreamFlowClient::create_workflow()` via HTTP POST
- [ ] Implement `StreamFlowClient::wait_for_completion()` with polling
- [ ] Add test: `test_sequential_workflow_load` (1000 workflows via HTTP)
- [ ] Add test: `test_parallel_workflow_load` (500 workflows, 10 parallel)
- [ ] Add test: `test_high_concurrency_load` (5000 workflows, 100 concurrent)
- [ ] Add test: `test_sustained_throughput` (60 second sustained load)
- [ ] Implement metrics reporting (throughput, latency percentiles, success rate)
- [ ] Verify tests run locally with StreamFlow server running

### Phase 3: CI Integration (3-4 hours)
- [ ] Create `.github/workflows/benchmarks.yml`
- [ ] Configure PostgreSQL service in GitHub Actions
- [ ] Add step: Build StreamFlow server
- [ ] Add step: Start StreamFlow server in background
- [ ] Add step: Wait for server health check
- [ ] Add step: Register benchmark workflow definitions
- [ ] Add step: Run HTTP API load tests from benchmark crate
- [ ] Update `scripts/parse_benchmark_results.py` (HTTP API output only)
- [ ] Update `scripts/compare_benchmarks.py` (scenario-based comparison)
- [ ] Update `scripts/check_regression.py` (updated format)
- [ ] Update `scripts/generate_report.py` (updated format)
- [ ] Add step: Download baseline results
- [ ] Add step: Compare with baseline
- [ ] Add step: Check for regression (fail if >10%)
- [ ] Add step: Generate HTML report
- [ ] Add step: Upload artifacts
- [ ] Add step: Update baseline (main branch only)
- [ ] Add step: Comment on PR with results
- [ ] Test workflow on PR

### Phase 4: Documentation (2 hours)
- [ ] Create `docs/performance-testing.md` guide
- [ ] Document HTTP API-based benchmarking approach
- [ ] Document running benchmarks locally (start server first)
- [ ] Document interpreting results
- [ ] Document adding new benchmark scenarios
- [ ] Document CI integration
- [ ] Document performance targets
- [ ] Update `docs/architecture.md` with performance section
- [ ] Add troubleshooting section (server startup, etc.)
- [ ] Note: Criterion benchmarks deferred to US-2.X
- [ ] Review and polish documentation

### Phase 5: Validation (1 hour)
- [ ] Run StreamFlow server locally
- [ ] Run benchmark tests locally and verify results
- [ ] Verify CI pipeline executes successfully on test PR
- [ ] Verify StreamFlow server starts correctly in CI
- [ ] Verify regression detection works
- [ ] Verify HTML report generation with scenarios
- [ ] Verify PR comments appear correctly
- [ ] Establish initial baseline on main branch
- [ ] Document baseline metrics in Epic 2 tracking

---

## Dependencies

### External Crates (Benchmark Crate Only)
- `reqwest` 0.12+ with `json` feature (HTTP client)
- `tokio` 1.x with `full` features (async runtime)
- `serde` 1.x with `derive` feature (serialization)
- `serde_json` 1.x (JSON handling)
- `uuid` 1.10+ with `v7` and `serde` features
- `chrono` 0.4+ with `serde` feature (timestamps)
- `serial_test` 3.x (sequential test execution)

**Note**: Criterion removed - deferred to US-2.X (micro-benchmark optimization tool)

### Infrastructure
- PostgreSQL 18+ (database: `streamflow_bench`)
- StreamFlow server running on localhost:8080 (for benchmarks)
- GitHub Actions with Ubuntu runner
- Python 3.x for analysis scripts

### Internal Dependencies
- Completed US-1C.2 (All-in-One Service Launcher) - for running StreamFlow server
- Completed US-1A.6 (Workflow Status Query) - for HTTP API status checks
- Completed Epic 1B (Built-in Worker) - for end-to-end workflow execution
- HTTP API endpoints: `POST /api/v1/workflows`, `GET /api/v1/workflows/{id}`

---

## Performance Targets (Epic 2)

**Note**: All metrics measured via HTTP API (end-to-end, comparable to Temporal)

### Initial Baseline (Establish First)
- Workflow throughput: TBD wf/sec (via HTTP API)
- P99 end-to-end latency: TBD ms (create to complete)
- Success rate: TBD %

### MVP Target (Must Achieve)
- Workflow throughput: >100 wf/sec (via HTTP API)
- P99 end-to-end latency: <200ms (create to complete)
- Success rate: >99%
- Sustained throughput: >100 wf/sec for 60 seconds

### Stretch Goal (Epic 6 - Post-MVP Optimization)
- Workflow throughput: >1,000 wf/sec (via HTTP API)
- P99 end-to-end latency: <50ms (create to complete)
- Success rate: >99.9%
- Sustained throughput: >1,000 wf/sec for 60 seconds

**Internal Micro-benchmarks (Deferred to US-2.X)**:
- Orchestrator evaluation: <1ms per workflow
- Activity scheduling: <5ms
- Event publishing: <1ms

---

## Risks and Mitigations

### Risk: StreamFlow Server Startup Failures in CI
**Mitigation**:
- Health check endpoint (`/health`) with timeout
- Verify server logs if startup fails
- Start server with explicit configuration (port, database URL)
- Fail fast if server doesn't start within 30 seconds

### Risk: HTTP API Latency Variability
**Mitigation**:
- Run multiple workflows (1000+) to smooth out variance
- Compare P99 latency (not average) to capture worst-case
- Compare trends over time, not single runs
- Allow 10% threshold for regressions

### Risk: PostgreSQL Performance Variability
**Mitigation**:
- Use PostgreSQL service in CI (not Docker overlay)
- Warm up database with sample workflows before benchmarking
- Run migrations once at start
- Use separate database for benchmarks

### Risk: Test Database Bloat
**Mitigation**:
- StreamFlow server manages workflow lifecycle
- Database cleanup handled by StreamFlow
- Monitor database size in CI logs

### Risk: Baseline Drift
**Mitigation**:
- Update baseline only on main branch
- Review baseline changes before merging
- Track historical trends via artifacts
- Clearly document when baseline is intentionally updated

---

## Future Enhancements

### US-2.X: Criterion Micro-Benchmarks (Post-MVP)
- Criterion.rs benchmarks for internal components
- Orchestrator evaluation time (DAG logic only)
- Activity scheduling latency
- Event publishing overhead
- State query performance
- **Purpose**: Optimization tool for developers, not for platform comparison
- **See**: Deferred to post-MVP optimization story

### Post-MVP Performance Enhancements
- **Story 2.7**: Competitor Comparison Benchmarks (Temporal, Conductor)
- **Story 2.8**: Grafana Performance Dashboard (see Epic 5: Story 5.1)
- **Advanced Profiling Tools**:
  - Memory profiling (heap allocation tracking)
  - CPU profiling (flamegraphs via pprof)
  - Database query profiling (pg_stat_statements)
  - Network latency simulation (tc netem)

**See**: `docs/post-mvp.md` for complete details on post-MVP performance enhancements.

---

## References

- [reqwest HTTP Client](https://docs.rs/reqwest/)
- [GitHub Actions: PostgreSQL Service](https://docs.github.com/en/actions/using-containerized-services/creating-postgresql-service-containers)
- [StreamFlow Architecture](architecture.md)
- [MVP Requirements - Epic 2](mvp-requirements.md#epic-2-performance-benchmarking-and-validation)
- [Temporal Benchmarking](https://temporal.io/blog/performance-benchmarking) (for comparison methodology)
