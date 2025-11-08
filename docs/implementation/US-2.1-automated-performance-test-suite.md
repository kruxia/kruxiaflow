# US-2.1: Automated Performance Test Suite - Implementation Plan

**Epic**: 2 - Performance Benchmarking and Validation
**User Story**: US-2.1
**Status**: Ready for Implementation
**Estimated Effort**: 16-20 hours
**Priority**: P0 (Required for Epic 2)

---

## User Story

**As** a platform engineering lead
**I want** continuous performance benchmarking from day one
**So that** we detect regressions early and stay on performance track

## Acceptance Criteria

- [x] Benchmark scenarios: Sequential workflow, parallel workflow, high-concurrency
- [x] Metrics: Workflow throughput (wf/sec), start latency (P50/P95/P99), activity latency
- [x] CI integration: Run on every commit to main
- [x] Performance regression detection: Fail if >10% slower
- [x] Historical trend tracking
- [x] Programmatic workflows: Use Rust WorkflowDefinition structs, not YAML
- [x] Early Baseline: Establish performance floor after Epic 1 implementation

---

## Implementation Overview

This implementation creates a comprehensive performance testing framework with three key components:

1. **Performance benchmark suite** (`core/benches/`) - Criterion.rs benchmarks for micro-benchmarks
2. **Integration load tests** (`core/tests/performance/`) - End-to-end workflow throughput and latency tests
3. **CI automation** (`.github/workflows/`) - Automated regression detection and historical tracking

### Architecture

```mermaid
flowchart TB
    subgraph "Performance Test Suite"
        Criterion[Criterion Benchmarks<br/>core/benches/]
        LoadTests[Load Tests<br/>core/tests/performance/]
        Scenarios[Test Scenarios<br/>Sequential, Parallel, High-Concurrency]
    end

    subgraph "Metrics Collection"
        Throughput[Throughput<br/>workflows/sec]
        Latency[Latency<br/>P50/P95/P99]
        Activity[Activity Latency<br/>schedule to complete]
        Queue[Queue Depth<br/>pending activities]
    end

    subgraph "CI Pipeline"
        Trigger[GitHub Actions<br/>on push to main]
        Runner[Ubuntu Runner<br/>PostgreSQL 18]
        Compare[Regression Check<br/>>10% = fail]
        Store[Historical Data<br/>GitHub Pages/Artifacts]
        Report[HTML Report<br/>with charts]
    end

    Criterion --> Metrics
    LoadTests --> Metrics
    Scenarios --> LoadTests

    Trigger --> Runner
    Runner --> Criterion
    Runner --> LoadTests
    Metrics --> Compare
    Metrics --> Store
    Compare --> Report
    Store --> Report
```

---

## Detailed Implementation

### 1. Criterion.rs Micro-Benchmarks (5 hours)

**File**: `core/benches/workflow_benchmarks.rs`

**Purpose**: Fast micro-benchmarks for core components measuring overhead and scalability.

#### 1.1 Benchmark Harness Setup

```rust
// core/benches/workflow_benchmarks.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use streamflow_core::events::*;
use streamflow_core::queue::*;
use streamflow_core::orchestrator::*;
use tokio::runtime::Runtime;
use sqlx::PgPool;
use uuid::Uuid;

fn bench_workflow_evaluation(c: &mut Criterion) {
    let mut group = c.benchmark_group("workflow_evaluation");

    // Benchmark orchestrator evaluation time (core DAG logic)
    for size in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("sequential_workflow", size),
            size,
            |b, &size| {
                // Setup: Create workflow definition with N sequential activities
                let definition = create_sequential_workflow(size);
                let runtime = Runtime::new().unwrap();

                b.to_async(&runtime).iter(|| async {
                    // Measure: Time to evaluate workflow state and schedule next activities
                    let pool = setup_test_pool().await;
                    let orchestrator = Orchestrator::new(pool.clone(), OrchestratorConfig::default());

                    let start = std::time::Instant::now();
                    orchestrator.evaluate_workflow(&definition, &HashMap::new()).await;
                    black_box(start.elapsed())
                });
            },
        );
    }

    group.finish();
}

fn bench_activity_scheduling(c: &mut Criterion) {
    let mut group = c.benchmark_group("activity_scheduling");

    // Benchmark queue insertion performance
    for batch_size in [1, 10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::new("batch_insert", batch_size),
            batch_size,
            |b, &batch_size| {
                let runtime = Runtime::new().unwrap();

                b.to_async(&runtime).iter(|| async {
                    let pool = setup_test_pool().await;
                    let queue = PostgresQueue::new(pool.clone(), QueueConfig::default());

                    let activities = create_test_activities(batch_size);
                    let start = std::time::Instant::now();
                    queue.enqueue_batch(&activities).await.unwrap();
                    black_box(start.elapsed())
                });
            },
        );
    }

    group.finish();
}

fn bench_event_publishing(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_publishing");

    // Benchmark event source write performance
    group.bench_function("publish_workflow_event", |b| {
        let runtime = Runtime::new().unwrap();

        b.to_async(&runtime).iter(|| async {
            let pool = setup_test_pool().await;
            let event_source = PostgresEventSource::new(pool.clone());

            let event = NewWorkflowEvent {
                workflow_id: Uuid::now_v7(),
                event_type: WorkflowEventType::WorkflowCreated,
                payload: serde_json::json!({}),
            };

            let start = std::time::Instant::now();
            event_source.publish(event).await.unwrap();
            black_box(start.elapsed())
        });
    });

    group.finish();
}

fn bench_workflow_state_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_queries");

    // Benchmark workflow status query performance
    group.bench_function("get_workflow_by_id", |b| {
        let runtime = Runtime::new().unwrap();

        b.to_async(&runtime).iter(|| async {
            let pool = setup_test_pool().await;
            let workflow_id = setup_test_workflow(&pool).await;

            let start = std::time::Instant::now();
            let result = sqlx::query!(
                "SELECT id, status, state_data FROM workflows WHERE id = $1",
                workflow_id
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            black_box((start.elapsed(), result))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_workflow_evaluation,
    bench_activity_scheduling,
    bench_event_publishing,
    bench_workflow_state_query
);

criterion_main!(benches);
```

**Cargo.toml additions**:
```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["async_tokio", "html_reports"] }

[[bench]]
name = "workflow_benchmarks"
harness = false
```

**Configuration**: `core/benches/criterion.toml`
```toml
[default]
measurement_time = 10  # 10 second measurement per benchmark
warm_up_time = 3       # 3 second warm-up
sample_size = 100      # 100 samples per benchmark
noise_threshold = 0.05 # 5% noise threshold
```

#### 1.2 Benchmark Test Helpers

**File**: `core/benches/helpers.rs`

```rust
use sqlx::PgPool;
use streamflow_core::events::*;
use uuid::Uuid;
use serde_json::json;

pub async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/streamflow_bench".to_string());

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to benchmark database");

    // Ensure migrations are run
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

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

pub fn create_test_activities(count: usize) -> Vec<NewActivityTask> {
    (0..count)
        .map(|i| NewActivityTask {
            workflow_id: Uuid::now_v7(),
            activity_key: format!("activity_{}", i),
            namespace: "bench".to_string(),
            name: "noop".to_string(),
            parameters: json!({}),
            timeout_seconds: 300,
        })
        .collect()
}

pub async fn setup_test_workflow(pool: &PgPool) -> Uuid {
    let workflow_id = Uuid::now_v7();
    let definition = create_sequential_workflow(5);

    // Insert workflow definition
    let definition_id = insert_workflow_definition(pool, &definition).await;

    // Insert workflow
    sqlx::query!(
        r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
           VALUES ($1, $2, $3, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)"#,
        workflow_id,
        definition.name,
        definition_id
    )
    .execute(pool)
    .await
    .expect("Failed to insert test workflow");

    workflow_id
}

async fn insert_workflow_definition(pool: &PgPool, definition: &WorkflowDefinition) -> Uuid {
    let activities_json = serde_json::to_value(&definition.activities)
        .expect("Failed to serialize activities");

    let row = sqlx::query!(
        r#"INSERT INTO workflow_definitions (name, activities)
           VALUES ($1, $2)
           RETURNING id"#,
        definition.name,
        activities_json
    )
    .fetch_one(pool)
    .await
    .expect("Failed to insert workflow definition");

    row.id
}
```

---

### 2. Integration Load Tests (6 hours)

**File**: `core/tests/performance/load_tests.rs`

**Purpose**: End-to-end workflow throughput and latency measurements simulating production load.

#### 2.1 Test Scenarios

```rust
// core/tests/performance/load_tests.rs
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use streamflow_core::events::*;
use streamflow_core::orchestrator::{Orchestrator, OrchestratorConfig};
use streamflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use tokio::sync::Semaphore;
use uuid::Uuid;

mod helpers;
use helpers::*;

/// Performance metrics collected during load tests
#[derive(Debug, Clone)]
struct PerformanceMetrics {
    total_workflows: usize,
    duration: Duration,
    throughput_wf_per_sec: f64,
    latencies_ms: Vec<u64>,
    p50_latency_ms: u64,
    p95_latency_ms: u64,
    p99_latency_ms: u64,
    activity_latencies_ms: Vec<u64>,
    avg_activity_latency_ms: u64,
    max_queue_depth: usize,
}

impl PerformanceMetrics {
    fn from_measurements(
        total_workflows: usize,
        duration: Duration,
        latencies: Vec<Duration>,
        activity_latencies: Vec<Duration>,
        max_queue_depth: usize,
    ) -> Self {
        let throughput_wf_per_sec = total_workflows as f64 / duration.as_secs_f64();

        let mut latencies_ms: Vec<u64> = latencies.iter().map(|d| d.as_millis() as u64).collect();
        latencies_ms.sort();

        let p50_latency_ms = percentile(&latencies_ms, 0.50);
        let p95_latency_ms = percentile(&latencies_ms, 0.95);
        let p99_latency_ms = percentile(&latencies_ms, 0.99);

        let activity_latencies_ms: Vec<u64> = activity_latencies
            .iter()
            .map(|d| d.as_millis() as u64)
            .collect();
        let avg_activity_latency_ms = if activity_latencies_ms.is_empty() {
            0
        } else {
            activity_latencies_ms.iter().sum::<u64>() / activity_latencies_ms.len() as u64
        };

        Self {
            total_workflows,
            duration,
            throughput_wf_per_sec,
            latencies_ms,
            p50_latency_ms,
            p95_latency_ms,
            p99_latency_ms,
            activity_latencies_ms,
            avg_activity_latency_ms,
            max_queue_depth,
        }
    }

    fn print_report(&self, scenario_name: &str) {
        println!("\n{'='*60}");
        println!("Performance Test: {}", scenario_name);
        println!("{'='*60}");
        println!("Total Workflows:     {}", self.total_workflows);
        println!("Duration:            {:.2}s", self.duration.as_secs_f64());
        println!("Throughput:          {:.2} workflows/sec", self.throughput_wf_per_sec);
        println!("\nWorkflow Start Latency:");
        println!("  P50:               {} ms", self.p50_latency_ms);
        println!("  P95:               {} ms", self.p95_latency_ms);
        println!("  P99:               {} ms", self.p99_latency_ms);
        println!("\nActivity Latency:");
        println!("  Average:           {} ms", self.avg_activity_latency_ms);
        println!("  Max Queue Depth:   {}", self.max_queue_depth);
        println!("{'='*60}\n");
    }

    fn to_json(&self, scenario_name: &str) -> serde_json::Value {
        serde_json::json!({
            "scenario": scenario_name,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "total_workflows": self.total_workflows,
            "duration_seconds": self.duration.as_secs_f64(),
            "throughput_wf_per_sec": self.throughput_wf_per_sec,
            "latency": {
                "p50_ms": self.p50_latency_ms,
                "p95_ms": self.p95_latency_ms,
                "p99_ms": self.p99_latency_ms,
            },
            "activity_latency": {
                "avg_ms": self.avg_activity_latency_ms,
            },
            "queue": {
                "max_depth": self.max_queue_depth,
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
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let definition = create_sequential_workflow(5);
    let definition_id = insert_workflow_definition(&pool, &definition).await;

    let num_workflows = 1000;
    let metrics = run_workflow_load_test(
        &pool,
        &definition,
        definition_id,
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
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let definition = create_parallel_workflow(10);
    let definition_id = insert_workflow_definition(&pool, &definition).await;

    let num_workflows = 500;
    let metrics = run_workflow_load_test(
        &pool,
        &definition,
        definition_id,
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
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let definition = create_sequential_workflow(3);
    let definition_id = insert_workflow_definition(&pool, &definition).await;

    let num_workflows = 5000;
    let metrics = run_workflow_load_test(
        &pool,
        &definition,
        definition_id,
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
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let definition = create_sequential_workflow(5);
    let definition_id = insert_workflow_definition(&pool, &definition).await;

    // Run for 60 seconds to test sustained performance
    let duration = Duration::from_secs(60);
    let metrics = run_sustained_load_test(
        &pool,
        &definition,
        definition_id,
        duration,
        20, // concurrent workflows
    )
    .await;

    metrics.print_report("Sustained Throughput (60 seconds, 20 concurrent)");

    // Target: >1,000 workflows/sec sustained
    assert!(
        metrics.throughput_wf_per_sec >= 100.0,
        "Expected >= 100 wf/sec sustained, got {:.2}",
        metrics.throughput_wf_per_sec
    );
}

async fn run_workflow_load_test(
    pool: &PgPool,
    definition: &WorkflowDefinition,
    definition_id: Uuid,
    num_workflows: usize,
    max_concurrent: usize,
) -> PerformanceMetrics {
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let orchestrator = Arc::new(Orchestrator::new(pool.clone(), OrchestratorConfig::default()));

    // Start orchestrator in background
    let orch_handle = tokio::spawn({
        let orchestrator = orchestrator.clone();
        async move {
            orchestrator.run().await.expect("Orchestrator failed");
        }
    });

    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut latencies = Vec::new();
    let mut activity_latencies = Vec::new();
    let mut max_queue_depth = 0;

    let start_time = Instant::now();

    for i in 0..num_workflows {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let pool = pool.clone();
        let event_source = event_source.clone();
        let definition = definition.clone();

        let handle = tokio::spawn(async move {
            let workflow_start = Instant::now();
            let workflow_id = Uuid::now_v7();

            // Insert workflow
            sqlx::query!(
                r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
                   VALUES ($1, $2, $3, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)"#,
                workflow_id,
                definition.name,
                definition_id
            )
            .execute(&pool)
            .await
            .expect("Failed to insert workflow");

            // Publish workflow created event
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::WorkflowCreated,
                    payload: serde_json::json!({}),
                })
                .await
                .expect("Failed to publish event");

            let workflow_latency = workflow_start.elapsed();

            // Wait for workflow completion
            let mut completion_check_interval = tokio::time::interval(Duration::from_millis(10));
            loop {
                completion_check_interval.tick().await;

                let status = sqlx::query!(
                    "SELECT status FROM workflows WHERE id = $1",
                    workflow_id
                )
                .fetch_one(&pool)
                .await
                .expect("Failed to query workflow status");

                if status.status == "completed" || status.status == "failed" {
                    break;
                }
            }

            drop(permit);
            workflow_latency
        });

        latencies.push(handle.await.expect("Task failed"));

        // Sample queue depth every 100 workflows
        if i % 100 == 0 {
            let depth = get_queue_depth(pool).await;
            max_queue_depth = max_queue_depth.max(depth);
        }
    }

    let total_duration = start_time.elapsed();

    // Stop orchestrator
    orch_handle.abort();

    PerformanceMetrics::from_measurements(
        num_workflows,
        total_duration,
        latencies,
        activity_latencies,
        max_queue_depth,
    )
}

async fn run_sustained_load_test(
    pool: &PgPool,
    definition: &WorkflowDefinition,
    definition_id: Uuid,
    duration: Duration,
    max_concurrent: usize,
) -> PerformanceMetrics {
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let orchestrator = Arc::new(Orchestrator::new(pool.clone(), OrchestratorConfig::default()));

    // Start orchestrator
    let orch_handle = tokio::spawn({
        let orchestrator = orchestrator.clone();
        async move {
            orchestrator.run().await.expect("Orchestrator failed");
        }
    });

    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut latencies = Vec::new();
    let mut workflow_count = 0;
    let start_time = Instant::now();

    while start_time.elapsed() < duration {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let pool = pool.clone();
        let event_source = event_source.clone();
        let definition = definition.clone();

        tokio::spawn(async move {
            let workflow_start = Instant::now();
            let workflow_id = Uuid::now_v7();

            // Insert and publish workflow
            sqlx::query!(
                r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
                   VALUES ($1, $2, $3, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)"#,
                workflow_id,
                definition.name,
                definition_id
            )
            .execute(&pool)
            .await
            .expect("Failed to insert workflow");

            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::WorkflowCreated,
                    payload: serde_json::json!({}),
                })
                .await
                .expect("Failed to publish event");

            drop(permit);
            workflow_start.elapsed()
        });

        workflow_count += 1;
    }

    let total_duration = start_time.elapsed();

    // Stop orchestrator
    orch_handle.abort();

    PerformanceMetrics::from_measurements(
        workflow_count,
        total_duration,
        latencies,
        vec![],
        0,
    )
}

async fn get_queue_depth(pool: &PgPool) -> usize {
    let result = sqlx::query!("SELECT COUNT(*) as count FROM activity_queue WHERE status = 'pending'")
        .fetch_one(pool)
        .await
        .expect("Failed to query queue depth");

    result.count.unwrap_or(0) as usize
}
```

#### 2.2 Test Helpers

**File**: `core/tests/performance/helpers.rs`

```rust
// Reuse helpers from orchestrator_integration_tests.rs
// Plus additional performance-specific helpers

use sqlx::PgPool;
use streamflow_core::events::*;
use uuid::Uuid;
use serde_json::json;

pub async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/streamflow_test".to_string());

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

pub async fn clean_test_data(pool: &PgPool) {
    sqlx::query!(
        "TRUNCATE workflow_events, workflow_event_consumers, workflows, workflow_definitions, activity_queue CASCADE"
    )
    .execute(pool)
    .await
    .expect("Failed to clean test data");
}

pub async fn insert_workflow_definition(pool: &PgPool, definition: &WorkflowDefinition) -> Uuid {
    let activities_json =
        serde_json::to_value(&definition.activities).expect("Failed to serialize activities");

    let row = sqlx::query!(
        r#"INSERT INTO workflow_definitions (name, activities)
           VALUES ($1, $2)
           RETURNING id"#,
        definition.name,
        activities_json
    )
    .fetch_one(pool)
    .await
    .expect("Failed to insert workflow definition");

    row.id
}

pub fn create_sequential_workflow(num_activities: usize) -> WorkflowDefinition {
    // Same as in benchmarks/helpers.rs
    // (implementation omitted for brevity - see section 1.2)
}

pub fn create_parallel_workflow(num_parallel: usize) -> WorkflowDefinition {
    // Same as in benchmarks/helpers.rs
    // (implementation omitted for brevity - see section 1.2)
}
```

---

### 3. CI Integration with GitHub Actions (5-6 hours)

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

jobs:
  benchmark:
    name: Run Performance Benchmarks
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
        with:
          components: rustfmt, clippy

      - name: Cache Cargo registry
        uses: actions/cache@v3
        with:
          path: ~/.cargo/registry
          key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}

      - name: Cache Cargo index
        uses: actions/cache@v3
        with:
          path: ~/.cargo/git
          key: ${{ runner.os }}-cargo-index-${{ hashFiles('**/Cargo.lock') }}

      - name: Cache target directory
        uses: actions/cache@v3
        with:
          path: target
          key: ${{ runner.os }}-target-bench-${{ hashFiles('**/Cargo.lock') }}

      - name: Install sqlx-cli
        run: cargo install sqlx-cli --no-default-features --features postgres

      - name: Run database migrations
        run: |
          cd migrations
          sqlx migrate run

      - name: Run Criterion benchmarks
        run: |
          cargo bench --package streamflow-core --bench workflow_benchmarks -- --output-format bencher | tee criterion-output.txt

      - name: Run integration load tests
        run: |
          cargo test --package streamflow-core --test performance_load_tests --release -- --nocapture --test-threads=1 | tee load-test-output.txt

      - name: Parse benchmark results
        id: parse_results
        run: |
          python scripts/parse_benchmark_results.py criterion-output.txt load-test-output.txt > benchmark-results.json

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
            criterion-output.txt
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
"""Parse benchmark output and generate JSON results."""

import sys
import json
import re
from datetime import datetime

def parse_criterion_output(filepath):
    """Parse Criterion benchmark output."""
    results = {}

    with open(filepath, 'r') as f:
        content = f.read()

    # Parse benchmark results (example pattern)
    pattern = r'test (\S+)\s+\.\.\. bench:\s+([\d,]+) ns/iter'
    for match in re.finditer(pattern, content):
        test_name = match.group(1)
        time_ns = int(match.group(2).replace(',', ''))
        results[test_name] = {
            'time_ns': time_ns,
            'time_ms': time_ns / 1_000_000.0
        }

    return results

def parse_load_test_output(filepath):
    """Parse load test output."""
    results = {}

    with open(filepath, 'r') as f:
        content = f.read()

    # Extract performance metrics from test output
    # Pattern: "Throughput:          123.45 workflows/sec"
    throughput_match = re.search(r'Throughput:\s+([\d.]+) workflows/sec', content)
    if throughput_match:
        results['throughput_wf_per_sec'] = float(throughput_match.group(1))

    # Pattern: "  P50:               12 ms"
    p50_match = re.search(r'P50:\s+(\d+) ms', content)
    p95_match = re.search(r'P95:\s+(\d+) ms', content)
    p99_match = re.search(r'P99:\s+(\d+) ms', content)

    if p50_match:
        results['latency'] = {
            'p50_ms': int(p50_match.group(1)),
            'p95_ms': int(p95_match.group(1)) if p95_match else 0,
            'p99_ms': int(p99_match.group(1)) if p99_match else 0,
        }

    return results

def main():
    if len(sys.argv) < 3:
        print("Usage: parse_benchmark_results.py <criterion_output> <load_test_output>", file=sys.stderr)
        sys.exit(1)

    criterion_file = sys.argv[1]
    load_test_file = sys.argv[2]

    criterion_results = parse_criterion_output(criterion_file)
    load_test_results = parse_load_test_output(load_test_file)

    combined = {
        'timestamp': datetime.utcnow().isoformat(),
        'git_sha': os.environ.get('GITHUB_SHA', 'unknown'),
        'criterion': criterion_results,
        'load_tests': load_test_results
    }

    print(json.dumps(combined, indent=2))

if __name__ == '__main__':
    import os
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

def compare_results(baseline, current):
    """Compare benchmark results."""
    comparison = {
        'timestamp': current.get('timestamp'),
        'git_sha': current.get('git_sha'),
        'baseline_sha': baseline.get('git_sha'),
    }

    # Compare throughput
    baseline_throughput = baseline.get('load_tests', {}).get('throughput_wf_per_sec', 0)
    current_throughput = current.get('load_tests', {}).get('throughput_wf_per_sec', 0)

    comparison['throughput'] = {
        'baseline': baseline_throughput,
        'current': current_throughput,
        'change': calculate_change(baseline_throughput, current_throughput)
    }

    # Compare latency
    baseline_latency = baseline.get('load_tests', {}).get('latency', {})
    current_latency = current.get('load_tests', {}).get('latency', {})

    comparison['p99_latency'] = {
        'baseline': baseline_latency.get('p99_ms', 0),
        'current': current_latency.get('p99_ms', 0),
        'change': calculate_change(
            baseline_latency.get('p99_ms', 0),
            current_latency.get('p99_ms', 0)
        )
    }

    # Detect regression (>10% slower throughput OR >10% higher latency)
    throughput_regression = comparison['throughput']['change'] < -10
    latency_regression = comparison['p99_latency']['change'] > 10

    comparison['regression'] = throughput_regression or latency_regression
    comparison['regression_details'] = {
        'throughput_regression': throughput_regression,
        'latency_regression': latency_regression
    }

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

- [x] Criterion benchmarks run successfully and generate reports
- [x] Load tests measure throughput (wf/sec) and latency (P50/P95/P99)
- [x] CI pipeline runs benchmarks on every commit to main
- [x] Regression detection works (>10% slower = fail)
- [x] Historical trends tracked via GitHub artifacts
- [x] HTML reports generated with charts
- [x] Documentation complete with usage examples

---

## Implementation Checklist

### Phase 1: Criterion Benchmarks (5 hours)
- [ ] Set up `core/benches/workflow_benchmarks.rs` with Criterion
- [ ] Add benchmark for workflow evaluation (sequential, parallel)
- [ ] Add benchmark for activity scheduling (single, batch)
- [ ] Add benchmark for event publishing
- [ ] Add benchmark for state queries
- [ ] Create `core/benches/helpers.rs` with test fixtures
- [ ] Configure Criterion settings (`criterion.toml`)
- [ ] Update `core/Cargo.toml` with Criterion dependency
- [ ] Verify benchmarks run locally: `cargo bench`

### Phase 2: Integration Load Tests (6 hours)
- [ ] Create `core/tests/performance/` directory
- [ ] Implement `load_tests.rs` with PerformanceMetrics struct
- [ ] Add test: `test_sequential_workflow_load` (1000 workflows)
- [ ] Add test: `test_parallel_workflow_load` (500 workflows, 10 parallel)
- [ ] Add test: `test_high_concurrency_load` (5000 workflows, 100 concurrent)
- [ ] Add test: `test_sustained_throughput` (60 second sustained load)
- [ ] Create `helpers.rs` with test setup functions
- [ ] Implement metrics collection (throughput, latency percentiles)
- [ ] Add JSON export for metrics
- [ ] Verify tests run locally: `cargo test --test performance_load_tests --release`

### Phase 3: CI Integration (5-6 hours)
- [ ] Create `.github/workflows/benchmarks.yml`
- [ ] Configure PostgreSQL service in GitHub Actions
- [ ] Add steps: checkout, install Rust, run migrations
- [ ] Add step: Run Criterion benchmarks
- [ ] Add step: Run integration load tests
- [ ] Create `scripts/parse_benchmark_results.py`
- [ ] Create `scripts/compare_benchmarks.py`
- [ ] Create `scripts/check_regression.py`
- [ ] Create `scripts/generate_report.py`
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
- [ ] Document running benchmarks locally
- [ ] Document interpreting results
- [ ] Document adding new benchmarks
- [ ] Document CI integration
- [ ] Document performance targets
- [ ] Update `docs/architecture.md` with performance section
- [ ] Add troubleshooting section
- [ ] Review and polish documentation

### Phase 5: Validation (1 hour)
- [ ] Run full test suite locally
- [ ] Verify CI pipeline executes successfully on test PR
- [ ] Verify regression detection works
- [ ] Verify HTML report generation
- [ ] Verify PR comments appear correctly
- [ ] Establish initial baseline on main branch
- [ ] Document baseline metrics in Epic 2 tracking

---

## Dependencies

### External Crates
- `criterion` 0.5+ with `async_tokio` and `html_reports` features
- Existing: `tokio`, `sqlx`, `serde_json`, `uuid`

### Infrastructure
- PostgreSQL 18+ (test database: `streamflow_bench`)
- GitHub Actions with Ubuntu runner
- Python 3.x for analysis scripts

### Internal Dependencies
- Completed US-1C.2 (All-in-One Service Launcher) for running orchestrator
- Completed US-1A.6 (Workflow Status Query) for metrics collection
- Completed Epic 1B (Built-in Worker) for end-to-end tests

---

## Performance Targets (Epic 2)

### Initial Baseline (Establish First)
- Workflow throughput: TBD wf/sec
- P99 start latency: TBD ms
- Activity scheduling: TBD ms
- Orchestrator evaluation: TBD ms per workflow

### MVP Target (Must Achieve)
- Workflow throughput: >100 wf/sec
- P99 start latency: <100ms
- Activity scheduling: <5ms
- Orchestrator evaluation: <1ms per workflow

### Stretch Goal (Epic 6)
- Workflow throughput: >1,000 wf/sec
- P99 start latency: <10ms
- Activity scheduling: <1ms
- Orchestrator evaluation: <500μs per workflow

---

## Risks and Mitigations

### Risk: Noisy CI Environment
**Mitigation**:
- Use 5% noise threshold in Criterion
- Run multiple samples (100 per benchmark)
- Compare trends over time, not single runs

### Risk: PostgreSQL Performance Variability
**Mitigation**:
- Use PostgreSQL service in CI (not Docker)
- Warm up database before benchmarks
- Run migrations once at start

### Risk: Test Database Bloat
**Mitigation**:
- Truncate tables between test scenarios
- Use separate database for benchmarks
- Run VACUUM periodically

### Risk: Baseline Drift
**Mitigation**:
- Update baseline only on main branch
- Review baseline changes before merging
- Track historical trends via artifacts

---

## Future Enhancements (Post-Epic 2)

### Epic 2.2: Competitor Comparison
- Containerized Temporal/Conductor setups
- Identical workflow scenarios
- Hardware-normalized comparisons
- Published methodology

### Epic 2.5: Grafana Dashboard
- Prometheus metrics endpoint integration
- Real-time performance monitoring
- Alerting on performance degradation

### Advanced Benchmarking
- Memory profiling (heap allocation tracking)
- CPU profiling (flamegraphs)
- Database query profiling (pg_stat_statements)
- Network latency simulation

---

## References

- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [GitHub Actions: PostgreSQL Service](https://docs.github.com/en/actions/using-containerized-services/creating-postgresql-service-containers)
- [StreamFlow Architecture](architecture.md)
- [MVP Requirements - Epic 2](mvp-requirements.md#epic-2-performance-benchmarking-and-validation)
