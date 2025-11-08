# StreamFlow Performance Optimization Plan

**Current Status**: Performance is 3.6-10× slower than target (comprehensive profiling complete)  
**Target**: >100 wf/sec with P99 latency <200ms  
**Best Performance**: 27.63 wf/sec (high concurrency), 9.91 wf/sec (sequential) with 100% success  
**Critical Issue**: 12% failure rate on parallel workflows

**Last Updated**: 2025-11-08 17:10 (after full 4-test benchmark suite with profiling)

---

## Current Performance Baseline

### ✅ Critical Bug Fixed (2025-11-08)

**Issue**: All 20 worker threads shared a single `worker_id`, preventing true parallelism due to database row-level lock contention.

**Fix**: Modified `worker/src/manager.rs:44-48` to assign unique `worker_id` per poller thread:
```rust
poller_config.worker_id = format!("{}_poller_{}", self.config.worker_id, i);
```

**Impact**:
- **+14% throughput improvement** (9.64 → 11.02 wf/sec)
- **-19% latency reduction** (P50: 1142ms → 925ms)
- **All 20 workers now active** (was: only 1 worker active)

### Benchmark Results (Comprehensive Profiling - 2025-11-08 17:08)

**Full Report**: `docs/performance/performance-2025-11-08-17-08.md`

| Scenario                                           | Throughput         | P50 Latency | P99 Latency | Success | Target       | Gap                         |
|----------------------------------------------------|--------------------:|------------:|------------:|--------:|--------------|----------------------------|
| **Sequential** (5 act, 100 wf)                     | 9.91 wf/sec         | 1,088 ms    | 1,359 ms    | 100%   | 100 wf/sec   | **10× slower**             |
| **Parallel** (10 act, 50 wf)                       | 1.42 wf/sec         | 887 ms      | 30,336 ms   | **88%**❌ | 50 wf/sec    | **35× slower** + failures  |
| **High Concurrency** (3 act, 300 wf, 100 conc)     | **27.63 wf/sec** ✅  | 3,318 ms    | 4,932 ms    | 100%   | 100 wf/sec   | **3.6× slower** (BEST)     |
| **Sustained** (60s, 20 conc)                       | 6.41 wf/sec         | 2,975 ms    | 5,126 ms    | 100%   | 100 wf/sec   | **16× slower**             |

**Profiling Data**: `var/benchmark-20251108-17{0819,0839,0924,0945}/` (flamegraphs, queries, logs)

### Key Observations from Comprehensive Profiling

1. **System thrives on high concurrency**: 27.63 wf/sec with 100 concurrent workflows (BEST performance)  
2. **Critical correctness issue**: Parallel workflows have 12% failure rate (6/50 timeout at 30s)  
3. **Event polling degrades over time**: Query time increases from 0.252ms → 2.370ms (9.4× slower in 60s test)  
4. **Database is NOT the bottleneck**: Only 2.5-8.9% of total execution time spent in DB  
5. **System is WAITING, not working**: High concurrency achieves 2.8× better throughput (suggests latency-based bottleneck)  
6. **Sustained performance is stable**: 422 workflows over 60s with 100% success (no crashes, leaks, or degradation)

---

## Profiling Infrastructure Status

### ✅ All Systems Working

**Comprehensive profiling complete** with all 4 benchmark scenarios:

1. **pg_stat_statements** - ✅ Enabled in docker-compose.yml, collecting query statistics  
2. **CPU Flamegraphs** - ✅ Generated using macOS `sample` command (no sudo required)  
3. **Query Analysis** - ✅ Top queries by total time identified for each scenario  
4. **Server Logs** - ✅ Debug logging captured for all tests  
5. **Benchmark Results Parsing** - ✅ Python scripts restored and working

**Schema Documentation**:
- Event polling: Uses `timestamp` column (NOT `created_at`)  
- Workflow queries: Uses `definition_name` column (NOT `workflow_type`)

**No Outstanding Infrastructure Issues**

---

## Phase 1: Measure & Profile

### 1.1 Add Distributed Tracing Instrumentation

Add timing spans at every major component to identify bottlenecks:

#### Orchestrator Event Loop

```rust
// core/src/orchestrator/orchestrator.rs
pub async fn process_workflow_event(...) -> Result<()> {
    let span = tracing::info_span!(
        "process_workflow_event",
        workflow_id = %event.workflow_id,
        event_type = ?event.event_type
    );
    let _enter = span.enter();

    // Track DB transaction time
    let tx_span = tracing::info_span!("begin_transaction");
    let mut tx = {
        let _enter = tx_span.enter();
        config.pool.begin().await?
    };

    // Track advisory lock acquisition
    let lock_span = tracing::info_span!("acquire_advisory_lock");
    {
        let _enter = lock_span.enter();
        sqlx::query!("SELECT pg_advisory_xact_lock($1)", workflow_id)
            .execute(&mut *tx).await?;
    }

    // Track state loading
    let state_span = tracing::info_span!("load_workflow_state");
    let state = {
        let _enter = state_span.enter();
        load_materialized_state(&mut tx, event.workflow_id).await?
    };

    // Track event application
    let apply_span = tracing::info_span!("apply_event_to_state");
    {
        let _enter = apply_span.enter();
        apply_event_to_state(&mut state, event)?;
    }

    // Track dependency evaluation
    let eval_span = tracing::info_span!(
        "evaluate_dependencies",
        num_activities = state.activities.len()
    );
    let ready_activities = {
        let _enter = eval_span.enter();
        find_ready_activities(&state, &definition)?
    };

    // Track activity scheduling
    let schedule_span = tracing::info_span!(
        "schedule_activities",
        count = ready_activities.len()
    );
    {
        let _enter = schedule_span.enter();
        activity_queue.schedule(event.workflow_id, ready_activities).await?;
    }

    // Track state save
    let save_span = tracing::info_span!("save_workflow_state");
    {
        let _enter = save_span.enter();
        save_materialized_state(&mut tx, event.workflow_id, &state).await?;
    }

    // Track commit
    let commit_span = tracing::info_span!("commit_transaction");
    {
        let _enter = commit_span.enter();
        tx.commit().await?;
    }

    Ok(())
}
```

#### Activity Queue Operations

```rust
// core/src/queue/postgres_queue.rs
impl ActivityQueue for PostgresQueue {
    async fn schedule(...) -> Result<()> {
        let span = tracing::info_span!(
            "queue_schedule",
            workflow_id = %workflow_id,
            num_activities = activities.len()
        );
        let _enter = span.enter();

        // Track each insert
        for activity in activities {
            let insert_span = tracing::info_span!(
                "queue_insert",
                activity_key = %activity.key,
                namespace = %activity.namespace,
                name = %activity.name
            );
            let _enter = insert_span.enter();

            sqlx::query!(...)
                .execute(&self.pool).await?;
        }

        Ok(())
    }

    async fn claim_next(...) -> Result<Option<QueuedActivity>> {
        let span = tracing::info_span!(
            "queue_claim",
            namespace = %namespace,
            name = %name
        );
        let _enter = span.enter();

        // Measure the claim query time
        let result = sqlx::query_as!(...)
            .fetch_optional(&self.pool).await?;

        if result.is_some() {
            tracing::info!("Claimed activity");
        }

        Ok(result)
    }

    async fn complete(...) -> Result<()> {
        let span = tracing::info_span!(
            "queue_complete",
            activity_id = %activity_id
        );
        let _enter = span.enter();

        sqlx::query!(...)
            .execute(&self.pool).await?;

        Ok(())
    }
}
```

#### Event Source Operations

```rust
// core/src/events/postgres_event_source.rs
impl EventSource for PostgresPollingEventSource {
    async fn publish(&self, event: NewWorkflowEvent) -> Result<Uuid> {
        let span = tracing::info_span!(
            "event_publish",
            event_type = ?event.event_type,
            workflow_id = %event.workflow_id
        );
        let _enter = span.enter();

        // Measure the insert
        let result = sqlx::query!(...)
            .fetch_one(&self.pool).await?;

        Ok(result.id)
    }

    async fn poll(&self, consumer_id: &str) -> Result<Vec<WorkflowEvent>> {
        let span = tracing::info_span!(
            "event_poll",
            consumer_id = %consumer_id
        );
        let _enter = span.enter();

        let events = sqlx::query_as!(...)
            .fetch_all(&self.pool).await?;

        tracing::info!(
            event_count = events.len(),
            "Polled events"
        );

        Ok(events)
    }
}
```

#### Worker Activity Execution

```rust
// activity/src/worker_service.rs
async fn execute_activity(...) -> Result<()> {
    let span = tracing::info_span!(
        "worker_execute_activity",
        activity_id = %activity.id,
        namespace = %activity.namespace,
        name = %activity.name
    );
    let _enter = span.enter();

    // Measure the actual activity execution
    let exec_span = tracing::info_span!("activity_handler");
    let result = {
        let _enter = exec_span.enter();
        handler.execute(activity.parameters).await
    };

    // Measure the completion report
    let complete_span = tracing::info_span!("report_completion");
    {
        let _enter = complete_span.enter();
        queue.complete(activity.id, result).await?;
    }

    Ok(())
}
```

### 1.2 Configure Tracing Output

Add to `Cargo.toml`:
```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tracing-timing = "0.6"  # For timing analysis
```

Configure tracing on startup:
```rust
// main.rs or serve command
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "streamflow=info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_level(true)
            .with_thread_ids(true)
            .with_timer(tracing_subscriber::fmt::time::uptime())
        )
        .init();
}
```

Run benchmarks with timing enabled:
```bash
RUST_LOG=streamflow=info,sqlx=debug \
  cargo test --package streamflow-benchmark --release test_sequential_workflow_load -- --nocapture
```

Look for output like:
```
[INFO  streamflow::orchestrator] 0.234s process_workflow_event workflow_id=abc123
  [INFO  streamflow::orchestrator]   0.012s begin_transaction
  [INFO  streamflow::orchestrator]   0.045s acquire_advisory_lock  ⚠️ SLOW
  [INFO  streamflow::orchestrator]   0.003s load_workflow_state
  [INFO  streamflow::orchestrator]   0.001s apply_event_to_state
  [INFO  streamflow::orchestrator]   0.002s evaluate_dependencies
  [INFO  streamflow::orchestrator]   0.156s schedule_activities  ⚠️ VERY SLOW
  [INFO  streamflow::orchestrator]   0.008s save_workflow_state
  [INFO  streamflow::orchestrator]   0.007s commit_transaction
```

This will immediately show which component is the bottleneck.

### 1.3 Database Query Analysis

#### Enable PostgreSQL Slow Query Logging

Add to `postgresql.conf`:
```
log_min_duration_statement = 100  # Log queries taking >100ms
log_line_prefix = '%t [%p]: [%l-1] user=%u,db=%d,app=%a,client=%h '
log_statement = 'all'  # Temporarily log all statements
```

Or set via SQL:
```sql
-- Enable slow query logging for this session
SET log_min_duration_statement = 100;

-- Enable query plan logging
SET auto_explain.log_min_duration = 100;
```

#### Analyze Critical Queries

**1. Event Polling Query**:
```sql
EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
SELECT id, workflow_id, event_type, activity_key, payload, timestamp, created_at
FROM workflow_events
WHERE id > (
    SELECT last_event_id FROM consumer_positions WHERE consumer_id = 'orchestrator'
)
ORDER BY id ASC
LIMIT 100;
```

Expected issues:
- Sequential scan instead of index scan
- Missing index on `id` for ordering
- Consumer position lookup adding overhead

**2. Activity Queue Claim Query**:
```sql
EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
UPDATE activity_queue
SET status = 'running',
    claimed_at = NOW(),
    claimed_by = '12345'
WHERE id = (
    SELECT id FROM activity_queue
    WHERE status = 'pending'
      AND scheduled_for <= NOW()
      AND namespace = 'default'
      AND name = 'echo'
    ORDER BY scheduled_for ASC
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING *;
```

Expected issues:
- Index not covering all WHERE conditions
- Sequential scan on `activity_queue`
- Lock contention on queue table

**3. Activity Queue Insert (Scheduling)**:
```sql
EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
INSERT INTO activity_queue
(id, workflow_id, activity_key, namespace, name, parameters, settings, scheduled_for, status)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending')
ON CONFLICT (workflow_id, activity_key) DO NOTHING;
```

Expected issues:
- Conflict check requiring table scan
- Missing unique index on `(workflow_id, activity_key)`
- No partial index on `status = 'pending'`

**4. Workflow State Load/Save**:
```sql
EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
SELECT state_data FROM workflows WHERE id = $1;

EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
UPDATE workflows SET state_data = $1, updated_at = NOW() WHERE id = $2;
```

Expected issues:
- JSONB field causing large data transfer
- No compression on JSONB
- Full row lock instead of field-level lock

**5. Advisory Lock Acquisition**:
```sql
EXPLAIN (ANALYZE, BUFFERS, VERBOSE)
SELECT pg_advisory_xact_lock($1);
```

Look for:
- Time spent waiting for lock
- Number of concurrent lock holders
- Lock queue depth

#### Check Missing Indexes

```sql
-- Find tables with sequential scans
SELECT schemaname, tablename, seq_scan, seq_tup_read, idx_scan, idx_tup_fetch
FROM pg_stat_user_tables
WHERE seq_scan > 1000
ORDER BY seq_tup_read DESC;

-- Find missing indexes on frequently accessed columns
SELECT * FROM pg_stat_user_tables WHERE schemaname = 'public';

-- Check index usage
SELECT schemaname, tablename, indexname, idx_scan, idx_tup_read, idx_tup_fetch
FROM pg_stat_user_indexes
WHERE schemaname = 'public'
ORDER BY idx_scan DESC;
```

#### Analyze Connection Pool Settings

```sql
-- Check current connection stats
SELECT COUNT(*) as total_connections,
       COUNT(*) FILTER (WHERE state = 'active') as active,
       COUNT(*) FILTER (WHERE state = 'idle') as idle,
       COUNT(*) FILTER (WHERE state = 'idle in transaction') as idle_in_transaction
FROM pg_stat_activity
WHERE datname = 'streamflow_benchmark';

-- Check for lock waits
SELECT locktype, relation::regclass, mode, granted, pid, wait_event_type, wait_event
FROM pg_locks
WHERE NOT granted;
```

Current pool config (likely in code):
```rust
PgPoolOptions::new()
    .min_connections(2)   // Too low?
    .max_connections(20)  // Too low for 100 concurrent workflows?
    .connect(&database_url)
    .await?
```

### 1.4 CPU Profiling with Flamegraphs

Install flamegraph tooling:
```bash
cargo install flamegraph
```

Profile the benchmark:
```bash
# Start the server with release build
cargo build --release
./target/release/streamflow serve --port 8080 &
SERVER_PID=$!

# Profile for 60 seconds while benchmark runs
sudo flamegraph -o flamegraph.svg -p $SERVER_PID -- sleep 60 &
FLAMEGRAPH_PID=$!

# Run benchmark
cargo test --package streamflow-benchmark --release test_sequential_workflow_load -- --nocapture

# Wait for flamegraph to complete
wait $FLAMEGRAPH_PID

# Kill server
kill $SERVER_PID

# View flamegraph
open flamegraph.svg
```

Flamegraph will show:
- CPU time spent in each function
- Hot paths (wide bars = expensive)
- Call stack relationships
- Synchronization overhead (mutex locks, async waits)

Look for:
- Database query execution time
- Serialization/deserialization (serde_json)
- Lock contention (tokio::sync, parking_lot)
- Event polling overhead
- HTTP client overhead (worker → API calls)

### 1.5 Run Comprehensive Profiling Session

```bash
#!/bin/bash
# scripts/profile_benchmarks.sh

set -e

echo "=== Starting StreamFlow Performance Profiling ==="

# 1. Start server with tracing enabled
export RUST_LOG=streamflow=debug,sqlx=debug
export DATABASE_URL=postgres://streamflow:streamflow_dev@127.0.0.1:5432/streamflow_benchmark

echo "Building release binary..."
cargo build --release

echo "Starting server..."
./target/release/streamflow serve --port 8080 > server.log 2>&1 &
SERVER_PID=$!

sleep 5  # Wait for server to start

# 2. Enable PostgreSQL query logging
psql $DATABASE_URL -c "SET log_min_duration_statement = 10;"  # Log queries >10ms

# 3. Start flamegraph profiling
echo "Starting CPU profiling..."
sudo flamegraph -o flamegraph-sequential.svg -p $SERVER_PID -- sleep 60 &
FLAMEGRAPH_PID=$!

# 4. Run benchmark with tracing
echo "Running sequential workflow benchmark..."
cargo test --package streamflow-benchmark --release test_sequential_workflow_load -- --nocapture \
  | tee benchmark-trace.log

# 5. Wait for profiling to complete
wait $FLAMEGRAPH_PID

# 6. Collect PostgreSQL stats
echo "Collecting database statistics..."
psql $DATABASE_URL -c "
SELECT schemaname, tablename, seq_scan, seq_tup_read, idx_scan
FROM pg_stat_user_tables
ORDER BY seq_tup_read DESC;
" > db-table-stats.txt

psql $DATABASE_URL -c "
SELECT query, calls, total_time, mean_time, max_time
FROM pg_stat_statements
ORDER BY total_time DESC
LIMIT 20;
" > db-query-stats.txt

# 7. Stop server
kill $SERVER_PID

echo "=== Profiling Complete ==="
echo "Results:"
echo "  - server.log: Server tracing output"
echo "  - benchmark-trace.log: Benchmark timing spans"
echo "  - flamegraph-sequential.svg: CPU profile"
echo "  - db-table-stats.txt: Database table access patterns"
echo "  - db-query-stats.txt: Slowest queries"
```

---

## Phase 2: Identified Bottlenecks & Quick Wins

### ✅ Database NOT the Primary Bottleneck

**Profiling Data** (var/benchmark-20251108-162432):
- Total DB time: ~791ms / 10,940ms = **7.2% of total time**  
- Activity queue claim query: **0.014ms** execution time (uses index scan on `idx_queue_claimable`)  
- System is **waiting**, not working (latency-based bottleneck, not throughput-based)

### 🔴 Critical Issues Identified (2025-11-08 17:08 Profiling)

#### 2.1 Parallel Workflow Failures (CRITICAL - CORRECTNESS ISSUE)

**Finding**: 12% failure rate (6/50 workflows timeout) with extreme P99 latency

| Metric        | Value              | Expected     | Issue                              |
|---------------|--------------------|--------------|------------------------------------|
| Success Rate  | **88%** ❌          | 100%         | Workflows timing out               |
| P99 Latency   | **30,505 ms** ❌    | < 5,000 ms   | 6× timeout threshold               |
| Throughput    | **1.02 wf/sec** ❌  | ~10 wf/sec   | Activities not running in parallel |

**Hypothesis**:
- Deadlock in parallel activity scheduling  
- Workers claiming activities out of dependency order  
- Resource starvation with 10 parallel activities  
- Bug in dependency evaluation for parallel workflows

**Action**: Debug logs for failed workflows, verify all 10 activities scheduled simultaneously

#### 2.3 Event Polling Latency (MEDIUM-HIGH PROBABILITY)

**Current backoff config**: 10ms min, 5000ms max, 1.5× multiplier

**Impact calculation**:
- Under moderate load, backoff reaches 500ms-5s between polls  
- 100 workflows × 5 activities = 500 activity completions  
- Each completion publishes event → orchestrator polls to see it  
- Average polling delay
- This matches observed P50 latency of 3-5 seconds!

**Recommended fix**:
```rust
AdaptiveBackoff::new(
    Duration::from_millis(1),    // Min: 1ms (was 10ms)
    Duration::from_millis(100),  // Max: 100ms (was 5000ms)
    1.2,                         // Slower growth (was 1.5)
)
```

**Expected Impact**: 5-10× improvement in orchestration latency

#### 2.4 Connection Pool Size (MEDIUM PROBABILITY)

**Current config**: `max_connections = 20`
**Benchmark concurrency**: 100 workflows

**Resource calculation**:
- Each workflow needs: API (1) + Orchestrator (1) + Worker (1) + Polling (1-2) = **3-4 connections**
- 100 workflows × 3-4 = **300-400 connections needed**
- Only 20 available → severe queueing

**Recommended fix**:
```rust
PgPoolOptions::new()
    .min_connections(10)
    .max_connections(100)  // Match or exceed concurrency
    .acquire_timeout(Duration::from_secs(5))
    .connect(&database_url)
    .await?
```

**Expected Impact**: 2-5× improvement if pool exhaustion is occurring

### 🟡 Database Query Optimizations (LOW-MEDIUM IMPACT)

While database is only 7.2% of total time, there are still efficiency gains available:

#### Database Query Analysis

**Top bottlenecks** (from var/benchmark-20251108-162432):

| Query | Calls | Avg (ms) | Total (ms) | % of Test | Issue |
|-------|-------|----------|------------|-----------|-------|
| DELETE activity_queue | 500 | 0.336 | 167.93 | 1.5% | Immediate cleanup |
| UPDATE activity_queue (heartbeat) | 496 | 0.221 | 109.57 | 1.0% | Unnecessary for short activities |
| INSERT workflow_events | 4,950 | 0.033 | 162.56 | 1.5% | High volume (49.5 events/wf) |
| UPDATE workflows (state) | 5,050 | 0.025 | 127.38 | 1.2% | 10× per workflow |
| UPDATE activity_queue (claim) | 3,546 | 0.029 | 102.23 | 0.9% | 7.1× polling overhead |

**Polling efficiency**: 3,546 polls / 500 activities = **7.1× overhead** (improved from earlier, but still high)

#### Quick Database Wins (Est. 280ms / 2.5% improvement)

**1. Batch Activity Deletion** (saves ~168ms):
```rust
// Instead of: DELETE WHERE id = $1 (after each completion)
// Background task every 60s:
DELETE FROM activity_queue
WHERE status = 'completed'
  AND updated_at < NOW() - INTERVAL '1 hour';
```

**2. Remove Heartbeat Updates** (saves ~110ms):
```rust
// Remove UPDATE claimed_at for activities <5min
// Trust workers or use shorter timeouts
```

**3. Adaptive Poll Backoff** (reduces polling overhead):
```rust
if activities_claimed == 0 {
    poll_interval = min(poll_interval * 1.5, 5000ms);
} else {
    poll_interval = 100ms;  // Reset on success
}
```

#### Missing Indexes (Already Working)

**Activity queue claim query uses `idx_queue_claimable`** ✅:
- Execution time: **0.014ms** (excellent)
- Index covers: `(namespace, name, status, scheduled_for)`
- Using index scan (not sequential scan)

**No additional indexes needed** for current queries.

### 2.5 Missing Database Indexes (LEGACY SECTION - VERIFIED NOT NEEDED)

**Status**: ✅ Indexes already exist and working efficiently

**Test**: EXPLAIN ANALYZE shows index scans, not sequential scans

**Existing indexes**:
```sql
-- Activity queue pending activities index
CREATE INDEX CONCURRENTLY idx_activity_queue_pending_scheduled
ON activity_queue(namespace, name, scheduled_for)
WHERE status = 'pending' AND scheduled_for <= NOW();

-- Event polling index
CREATE INDEX CONCURRENTLY idx_workflow_events_id
ON workflow_events(id);

-- Workflow state lookup (should already exist)
CREATE INDEX CONCURRENTLY idx_workflows_id
ON workflows(id);

-- Activity queue workflow lookup
CREATE INDEX CONCURRENTLY idx_activity_queue_workflow
ON activity_queue(workflow_id);
```

**Expected Impact**: 5-10x improvement on queue operations

### 2.3 Advisory Lock Contention

**Hypothesis**: Multiple orchestrators waiting for workflow locks

**Test**:
```sql
-- Check lock waits
SELECT COUNT(*) as waiting_locks
FROM pg_locks
WHERE NOT granted AND locktype = 'advisory';
```

**Fix**: This is expected behavior (prevents concurrent evaluation of same workflow). Not a bug, but indicates orchestrator parallelism is working. If many locks are waiting, it means we need more orchestrator instances processing different workflows.

**Expected Impact**: No change (working as designed)

### 2.4 HTTP Polling Overhead (Worker → API)

**Hypothesis**: Workers polling API with high latency

**Test**: Check worker poll timing in traces:
```
[INFO worker] poll_activity took 150ms  ⚠️ Too slow
```

**Fix**:
```rust
// Add long-polling support to reduce round-trips
// In API handler:
async fn poll_activity(...) -> Result<Response> {
    let mut attempts = 0;
    loop {
        if let Some(activity) = queue.claim_next(namespace, name).await? {
            return Ok(Json(activity));
        }

        attempts += 1;
        if attempts >= 20 {
            // No activity after 20 attempts (2 seconds)
            return Ok(Json(None));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

**Expected Impact**: 2-3x improvement in activity claim latency

### 2.5 Event Polling Backoff Too Aggressive

**Hypothesis**: Orchestrator sleeping too long between polls

**Test**: Check orchestrator poll timing in traces:
```
[INFO orchestrator] Backoff interval: 5000ms  ⚠️ Too long under load
```

**Current Config**:
```rust
AdaptiveBackoff::new(
    Duration::from_millis(10),   // Min
    Duration::from_secs(5),      // Max
    1.5,                         // Multiplier
)
```

**Fix**: More aggressive polling under load:
```rust
AdaptiveBackoff::new(
    Duration::from_millis(1),    // Min: 1ms (was 10ms)
    Duration::from_millis(500),  // Max: 500ms (was 5s)
    1.2,                         // Multiplier: slower growth
)
```

**Expected Impact**: 50-100ms latency reduction per orchestration cycle

### 2.6 Serialization Overhead

**Hypothesis**: Large JSONB state causing serialization bottlenecks

**Test**: Look for `serde_json` in flamegraph taking >10% CPU

**Fix**:
```rust
// Use more efficient serialization for hot paths
// Consider MessagePack or bincode instead of JSON
use rmp_serde as msgpack;

// Or compress large JSONB
use flate2::write::GzEncoder;
```

**Expected Impact**: 10-20% CPU reduction if serialization is hot path

### 2.7 Worker Activity Execution Overhead

**Hypothesis**: "echo" activity implementation has unexpected overhead

**Test**: Check activity handler timing:
```
[INFO worker] activity_handler took 2500ms  ⚠️ Echo should be <1ms
```

**Fix**: Ensure echo activity is truly no-op:
```rust
// activity/src/handlers/echo.rs
pub async fn echo(params: Value) -> Result<Value> {
    // Should be instant
    Ok(params)
}
```

**Expected Impact**: If echo is slow, this indicates worker infrastructure overhead, not activity logic

---

## Phase 3: Optimization Roadmap (Updated Based on Findings)

Prioritized by impact based on profiling data:

### ✅ Completed
1. **Fix shared worker ID bug** - Unique worker IDs per poller thread (+14% throughput, -19% latency)
2. **Optimize batch size** - Set `max_activities_per_poll=1` for best load distribution
3. **Profile infrastructure** - Automated profiling script, query analysis, server logs

### Tier 1: Critical Investigations (Target: 5-10× improvement)

**Priority: Understand why system is waiting, not working**

1. **Investigate multi-test performance degradation** ⏱️ 2-4 hours
   - Why does sequential test degrade 5.4× when run after other tests?
   - Profile memory usage, connection pool, thread count across full suite
   - Check for resource leaks or accumulated state
   - **Expected Impact**: 5× improvement (restore 11.02 wf/sec baseline)

2. **Debug parallel workflow failures** ⏱️ 2-4 hours
   - Why do 12% of parallel workflows timeout?
   - Verify all 10 activities scheduled simultaneously
   - Check for dependency evaluation bugs
   - Analyze worker activity claiming patterns
   - **Expected Impact**: Fix correctness issue + 2-3× throughput

3. **Reduce event polling backoff** ⏱️ 15 minutes (QUICK WIN)
   ```rust
   // core/src/orchestrator/orchestrator.rs
   AdaptiveBackoff::new(
       Duration::from_millis(1),    // Was: 10ms
       Duration::from_millis(100),  // Was: 5000ms
       1.2,                         // Was: 1.5
   )
   ```
   - **Expected Impact**: 5-10× improvement (2.5s avg delay → 50ms)

4. **Increase connection pool size** ⏱️ 5 minutes (QUICK WIN)
   ```rust
   // All PgPoolOptions::new() calls
   .min_connections(10)
   .max_connections(100)  // Was: 20
   ```
   - **Expected Impact**: 2-5× improvement if pool exhaustion occurring

### Tier 2: Architectural Improvements (Target: 2-3× improvement)

5. **Add worker long-polling support** ⏱️ 1-2 hours
   - Reduce HTTP round-trips from 10/sec to 0.03/sec per worker
   - Decrease API server load and connection pool contention
   - **Expected Impact**: 1.5-2× improvement

6. **Batch activity deletion** ⏱️ 30 minutes
   - Background cleanup instead of immediate DELETE
   - **Expected Impact**: +2.5% (saves 168ms per 100 workflows)

7. **Remove heartbeat updates for short activities** ⏱️ 30 minutes
   - Skip UPDATE claimed_at for activities <5min
   - **Expected Impact**: +1% (saves 110ms per 100 workflows)

### Tier 3: Fine-Tuning (Target: 10-20% improvement)

8. **Fix profiling infrastructure** ⏱️ 30 minutes
   - Update schema mismatch queries (created_at, workflow_type)
   - Enable pg_stat_statements in docker-compose.yml
   - Document manual flamegraph process
   - **Expected Impact**: Better visibility for future optimizations

9. **Adaptive worker polling backoff** ⏱️ 1 hour
   - Reduce polling overhead (7.1× → ~2×)
   - **Expected Impact**: 5-10% reduction in overhead

10. **Database query batching** ⏱️ 2-4 hours
    - Batch event reads in orchestrator
    - Batch activity scheduling
    - **Expected Impact**: 10-15% improvement

### ❌ NOT Needed (Verified by Profiling)

- ~~Add database indexes~~ - Already exist and working (0.014ms query time)
- ~~Optimize slow queries~~ - Queries already fast (7.2% of total time) exist and working (0.014ms query time)  
- ~~Optimize slow queries~~ - Queries already fast (7.2% of total time)  
- ~~Serialization optimization~~ - Not showing in profiles as bottleneck  
- ~~JSONB compression~~ - Not needed yet (state updates only 1.2% of time)

---

## Phase 4: Continuous Monitoring

### Add Prometheus Metrics
```rust
use prometheus::{IntCounter, Histogram};

lazy_static! {
    static ref WORKFLOW_LATENCY: Histogram = register_histogram!(
        "streamflow_workflow_latency_seconds",
        "End-to-end workflow latency"
    ).unwrap();

    static ref ORCHESTRATOR_EVAL_TIME: Histogram = register_histogram!(
        "streamflow_orchestrator_eval_seconds",
        "Orchestrator evaluation time"
    ).unwrap();

    static ref QUEUE_CLAIM_TIME: Histogram = register_histogram!(
        "streamflow_queue_claim_seconds",
        "Activity queue claim time"
    ).unwrap();
}
```

Expose metrics endpoint:
```rust
// api/src/handlers/metrics.rs
pub async fn metrics() -> String {
    use prometheus::{Encoder, TextEncoder};> String {
    use prometheus::{Encoder, TextEncoder};
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
```

### Add Grafana Dashboards

Create dashboards tracking:
- Workflow throughput (wf/sec)  
- P50/P95/P99 latency  
- Database connection pool usage  
- Queue depth  
- Orchestrator event processing rate  
- Worker claim rate

---

## Success Criteria

### Phase 1 Complete ✅
- [x] Profiling infrastructure set up (automated script)  
- [x] Database query analysis completed  
- [x] Full profiling session run with results collected  
- [x] Bottlenecks identified with data  
- [ ] Tracing instrumentation added (optional, may not be needed)  
- [ ] Flamegraph profiling (blocked on sudo access, manual workaround documented)

### Phase 2 In Progress 🟡
- [x] Critical bug identified and fixed (shared worker ID)  
- [x] Batch size optimized (batch=1 with unique IDs)  
- [x] Database confirmed NOT primary bottleneck (7.2% of time)  
- [ ] Multi-test degradation investigated and fixed  
- [ ] Parallel workflow failures debugged and fixed  
- [ ] Event polling backoff reduced (QUICK WIN)  
- [ ] Connection pool increased (QUICK WIN)

### Phase 3 Goals
- [ ] All Tier 1 optimizations deployed  
- [ ] Multi-test performance degradation resolved (2.03 → 11.02 wf/sec)  
- [ ] Parallel workflow 100% success rate (currently 88%)  
- [ ] Throughput >50 wf/sec achieved (intermediate goal)  
- [ ] P99 latency <1000ms achieved (intermediate goal)

### Phase 4 Goals (Final Targets)
- [ ] Throughput >100 wf/sec achieved  
- [ ] P99 latency <200ms achieved  
- [ ] All benchmark scenarios passing consistently  
- [ ] Performance regression detection in CI

### Post-MVP (If Needed)
- [ ] Prometheus metrics exposed  
- [ ] Grafana dashboards created  
- [ ] Production monitoring operational

---

## Immediate Next Steps (Prioritized)

### 🔥 Quick Wins (Can do NOW - 20 minutes total)

1. **Reduce event polling backoff** ⏱️ 5 minutes
   ```bash
   # Edit core/src/orchestrator/orchestrator.rs
   # Change AdaptiveBackoff max from 5000ms to 100ms
   ```

2. **Increase connection pool size** ⏱️ 5 minutes
   ```bash
   # Find all PgPoolOptions::new() calls
   # Change max_connections from 20 to 100
   grep -r "PgPoolOptions::new" --include="*.rs"
   ```

3. **Re-run benchmark with quick wins** ⏱️ 10 minutes
   ```bash
   STREAMFLOW_MAX_ACTIVITIES_PER_POLL=1 bash scripts/benchmark.sh --test test_sequential_workflow_load
   ```
   - **Expected**: 50-100 wf/sec (5-10× improvement)

### 🔍 Critical Investigations (Next 1-2 days)

4. **Debug multi-test performance degradation** ⏱️ 2-4 hours  
   - Add logging to track connection pool usage across tests  
   - Monitor memory usage during full test suite  
   - Check for event consumer checkpoint accumulation  
   - Profile with `--nocapture` to see timing between tests

5. **Fix parallel workflow failures** ⏱️ 2-4 hours  
   - Enable debug logging for failed workflows  
   - Verify dependency evaluation logic  
   - Check worker activity claiming order  
   - Analyze server logs for parallel_* activities

### 🛠️ Follow-up Optimizations (Week 2)

6. **Add worker long-polling** ⏱️ 1-2 hours  
7. **Batch activity deletion** ⏱️ 30 minutes  
8. **Fix profiling infrastructure** ⏱️ 30 minutes  
9. **Remove heartbeat updates** ⏱️ 30 minutes  
10. **Adaptive worker polling** ⏱️ 1 hour

---
## Performance Evolution Tracking

| Date                | Test            | Change                          | Throughput         | P50 Latency | P99 Latency | Success | Gap                         |
|---------------------|-----------------|----------------------------------|--------------------:|------------:|------------:|--------:|----------------------------|
| 2025-11-07          | Sequential      | Baseline (shared ID)             | 9.64 wf/sec         | 1,142 ms    | 1,461 ms    | 100%   | 10.4×                      |
| 2025-11-08 09:00    | Sequential      | Fix shared ID + batch=1          | 11.02 wf/sec        | 925 ms      | 1,253 ms    | 100%   | 9×                         |
| **2025-11-08 17:08**| **Sequential**  | **Full profiling suite**         | **9.91 wf/sec**     | **1,088 ms**| **1,359 ms**| **100%**| **10×**                    |
| **2025-11-08 17:08**| **High Concurrency** | **Full profiling suite**   | **27.63 wf/sec** ✅  | **3,318 ms**| **4,932 ms**| **100%**| **3.6×** (BEST)            |
| **2025-11-08 17:08**| **Parallel**    | **Full profiling suite**         | **1.42 wf/sec** ❌   | **887 ms**  | **30,336 ms**| **88%** ❌ | **Failures**               |
| TBD                 | All             | Quick wins (polling + cleanup)   | Target: 50-100      | Target: 200-500 | Target: <1000 | 100% | 1-2×                       |
| TBD                 | All             | Fix parallel failures            | Target: 100+        | Target: <200 | Target: <500 | 100%   | ✅ At target               |

---

**Ready to implement quick wins? Start with items #1 and #2 above!**
