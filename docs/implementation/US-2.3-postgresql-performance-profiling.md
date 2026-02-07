# US-2.3: PostgreSQL Performance Profiling - Implementation Plan

**Epic**: 2 - Performance Benchmarking and Validation
**User Story**: US-2.3
**Status**: ✅ IMPLEMENTED
**Estimated Effort**: 4-6 hours
**Priority**: P1 (Required before Epic 6 PostgreSQL optimization)
**Architecture**: Query profiling infrastructure, pg_stat_statements, EXPLAIN ANALYZE tooling

---

## User Story

**As** a platform engineering lead
**I want** detailed profiling of PostgreSQL query performance
**So that** we identify optimization opportunities before scaling complexity

## Acceptance Criteria

- [x] Query execution plans for all hot paths
- [x] Identify slow queries (>10ms)
- [x] Index usage analysis
- [x] Connection pool utilization metrics
- [x] Lock contention detection

---

## Current State Analysis

### Hot Path Queries Identified

Based on codebase analysis, the following queries are in the critical path:

#### 1. High-Frequency Queries (Orchestrator/Worker Poll Loops)

| Query                  | File                           | Line   | Frequency      | Description                                |
|------------------------|--------------------------------|--------|----------------|--------------------------------------------|
| Event poll             | `postgres_event_source.rs`     | 47-66  | 10ms-5s        | SELECT with LEFT JOIN on consumer position |
| Update consumer pos    | `postgres_event_source.rs`     | 72-90  | per event batch| UPSERT on workflow_event_consumers         |
| Activity claim_next    | `postgres_queue.rs`            | 119-166| 10ms-5s        | UPDATE with FOR UPDATE SKIP LOCKED         |
| Activity heartbeat     | `postgres_queue.rs`            | 323-401| ~10s per task  | UPDATE with conditional logic              |

#### 2. Medium-Frequency Queries (Workflow Execution)

| Query                  | File                           | Line   | Frequency       | Description                                |
|------------------------|--------------------------------|--------|-----------------|------------------------------------------- |
| Publish event          | `postgres_event_source.rs`     | 23-40  | per event       | INSERT with ON CONFLICT DO NOTHING         |
| Activity schedule      | `postgres_queue.rs`            | 38-116 | per activity    | INSERT with ON CONFLICT DO NOTHING         |
| Activity complete      | `postgres_queue.rs`            | 238-320| per completion  | UPDATE with WHERE conditions               |
| Load materialized state| `workflow_state.rs`            | 173-198| per event       | SELECT from workflows table                |
| Save materialized state| `workflow_state.rs`            | 202-224| per state change| UPDATE workflows table                     |
| Activity fail          | `postgres_queue.rs`            | 404-510| per failure     | Transaction with SELECT FOR UPDATE + UPDATE|

#### 3. API Entry Point Queries

| Query                  | File                           | Line   | Frequency       | Description                                |
|------------------------|--------------------------------|--------|-----------------|------------------------------------------- |
| Submit workflow        | `service.rs`                   | 74-202 | per submission  | Transaction: SELECT + 2x INSERT            |
| Get latest definition  | `repository.rs`                | 120-145| per submission  | SELECT ORDER BY created_at DESC LIMIT 1    |
| Check duplicate key    | `service.rs`                   | 110-124| per submission  | SELECT by unique_key                       |

### Existing Indexes

From migration files:

```sql
-- activity_queue (20251028000001)
CREATE INDEX idx_queue_claimable ON activity_queue(worker, name, status, scheduled_for)
    WHERE status IN ('pending', 'running');
CREATE INDEX idx_queue_timeout_check ON activity_queue(status, claimed_at)
    WHERE status = 'running';
CREATE INDEX idx_queue_workflow ON activity_queue(workflow_id, created_at DESC);

-- workflow_events (20251029000001)
CREATE INDEX idx_events_workflow_id ON workflow_events(workflow_id, id DESC);
CREATE INDEX idx_events_type ON workflow_events(event_type, id DESC);

-- workflow_definitions
CREATE INDEX idx_workflow_definitions_created_at ON workflow_definitions USING brin(created_at);

-- workflows
CREATE INDEX idx_workflows_definition_status ON workflows(definition_name, status, created_at DESC);
CREATE INDEX idx_workflows_status ON workflows(status, updated_at DESC);
CREATE INDEX idx_workflows_definition_id ON workflows(workflow_definition_id);
```

---

## Implementation Plan

### Phase 1: Profiling Infrastructure Setup (1-2 hours)

#### 1.1 Enable pg_stat_statements Extension

Create migration to enable query statistics collection:

```sql
-- Migration: Enable pg_stat_statements for query profiling
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Configure settings (via PostgreSQL config or ALTER SYSTEM)
-- pg_stat_statements.track = 'all'
-- pg_stat_statements.max = 10000
```

**Configuration requirements** (postgresql.conf or docker-compose):
```
shared_preload_libraries = 'pg_stat_statements'
pg_stat_statements.track = all
pg_stat_statements.max = 10000
```

#### 1.2 Create Profiling Views

Create views for easy query analysis:

```sql
-- Top slow queries by total time
CREATE VIEW v_slow_queries AS
SELECT
    substring(query, 1, 100) as query_preview,
    calls,
    round(total_exec_time::numeric, 2) as total_time_ms,
    round(mean_exec_time::numeric, 2) as avg_time_ms,
    round(max_exec_time::numeric, 2) as max_time_ms,
    rows,
    round((100 * total_exec_time / sum(total_exec_time) OVER())::numeric, 2) as pct_total
FROM pg_stat_statements
ORDER BY total_exec_time DESC
LIMIT 20;

-- Index usage analysis
CREATE VIEW v_index_usage AS
SELECT
    schemaname,
    tablename,
    indexname,
    idx_scan as scans,
    idx_tup_read as tuples_read,
    idx_tup_fetch as tuples_fetched,
    pg_size_pretty(pg_relation_size(indexrelid)) as index_size
FROM pg_stat_user_indexes
ORDER BY idx_scan DESC;

-- Unused indexes (candidates for removal)
CREATE VIEW v_unused_indexes AS
SELECT
    schemaname,
    tablename,
    indexname,
    pg_size_pretty(pg_relation_size(indexrelid)) as index_size
FROM pg_stat_user_indexes
WHERE idx_scan = 0
AND indexname NOT LIKE '%_pkey'
ORDER BY pg_relation_size(indexrelid) DESC;

-- Table statistics
CREATE VIEW v_table_stats AS
SELECT
    schemaname,
    relname as tablename,
    seq_scan,
    seq_tup_read,
    idx_scan,
    idx_tup_fetch,
    n_tup_ins as inserts,
    n_tup_upd as updates,
    n_tup_del as deletes,
    n_live_tup as live_rows,
    n_dead_tup as dead_rows
FROM pg_stat_user_tables
ORDER BY seq_scan + idx_scan DESC;
```

#### 1.3 Add CLI Profiling Command

Add `kruxiaflow profile` command to gather and report metrics:

```rust
// kruxiaflow/src/commands/profile.rs
pub async fn run_profile(pool: &PgPool) -> Result<()> {
    // 1. Query pg_stat_statements for hot queries
    // 2. Analyze index usage
    // 3. Check for sequential scans
    // 4. Report lock contention
    // 5. Output formatted report
}
```

---

### Phase 2: Query Execution Plan Analysis (1-2 hours)

#### 2.1 EXPLAIN ANALYZE for Hot Paths

Create profiling scripts to capture execution plans under load:

```sql
-- Event poll query
EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
SELECT e.id, e.workflow_id, e.event_type, e.activity_key, e.payload, e.timestamp, e.iteration
FROM workflow_events e
LEFT JOIN workflow_event_consumers c ON c.consumer_id = 'orchestrator'
WHERE c.last_event_id IS NULL OR e.id > c.last_event_id
ORDER BY e.id ASC
LIMIT 100;

-- Activity claim_next query (the most complex hot path)
EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
UPDATE activity_queue
SET status = 'running'::activity_status,
    claimed_at = NOW(),
    claimed_by = 'worker-1'::TEXT,
    retry_count = CASE
        WHEN status = 'running'::activity_status THEN retry_count + 1
        ELSE retry_count
    END
WHERE id = (
    SELECT id FROM activity_queue
    WHERE worker = 'builtin'
      AND name = 'echo'
      AND (
          (status = 'pending'::activity_status AND scheduled_for <= NOW())
          OR
          (status = 'running'::activity_status
           AND NOW() > claimed_at + timeout_duration
           AND retry_count < max_retries)
      )
    ORDER BY scheduled_for ASC
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING id, workflow_id, activity_key, worker, name as activity_name,
          parameters, settings, retry_count, claimed_at, output_definitions, iteration;

-- Load materialized state query
EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
SELECT id, definition_name, status, activities, state_data, input
FROM workflows WHERE id = '<workflow_uuid>';
```

#### 2.2 Profile Under Load

Integrate profiling with existing benchmark suite (`scripts/profiling.sh`):

```bash
# Extend scripts/profiling.sh to capture PostgreSQL statistics
# Results go to var/profiling-YYYYmmdd-HHMMSS/ (existing convention)

# Before benchmark run:
psql -c "SELECT pg_stat_statements_reset();"

# After benchmark run (in the same OUTPUT_DIR):
psql -c "SELECT * FROM v_slow_queries;" > "${OUTPUT_DIR}/slow_queries.txt"
psql -c "SELECT * FROM v_index_usage;" > "${OUTPUT_DIR}/index_usage.txt"
psql -c "SELECT * FROM v_table_stats;" > "${OUTPUT_DIR}/table_stats.txt"

# Capture execution plans for hot paths
psql -f scripts/explain_hot_paths.sql > "${OUTPUT_DIR}/execution_plans.txt"
```

---

### Phase 3: Lock Contention Detection (30 min)

#### 3.1 Lock Monitoring View

```sql
CREATE VIEW v_lock_contention AS
SELECT
    blocked_locks.pid AS blocked_pid,
    blocked_activity.usename AS blocked_user,
    blocking_locks.pid AS blocking_pid,
    blocking_activity.usename AS blocking_user,
    blocked_activity.query AS blocked_statement,
    blocking_activity.query AS current_statement_in_blocking_process
FROM pg_catalog.pg_locks blocked_locks
JOIN pg_catalog.pg_stat_activity blocked_activity ON blocked_activity.pid = blocked_locks.pid
JOIN pg_catalog.pg_locks blocking_locks
    ON blocking_locks.locktype = blocked_locks.locktype
    AND blocking_locks.database IS NOT DISTINCT FROM blocked_locks.database
    AND blocking_locks.relation IS NOT DISTINCT FROM blocked_locks.relation
    AND blocking_locks.page IS NOT DISTINCT FROM blocked_locks.page
    AND blocking_locks.tuple IS NOT DISTINCT FROM blocked_locks.tuple
    AND blocking_locks.virtualxid IS NOT DISTINCT FROM blocked_locks.virtualxid
    AND blocking_locks.transactionid IS NOT DISTINCT FROM blocked_locks.transactionid
    AND blocking_locks.classid IS NOT DISTINCT FROM blocked_locks.classid
    AND blocking_locks.objid IS NOT DISTINCT FROM blocked_locks.objid
    AND blocking_locks.objsubid IS NOT DISTINCT FROM blocked_locks.objsubid
    AND blocking_locks.pid != blocked_locks.pid
JOIN pg_catalog.pg_stat_activity blocking_activity ON blocking_activity.pid = blocking_locks.pid
WHERE NOT blocked_locks.granted;
```

#### 3.2 FOR UPDATE SKIP LOCKED Effectiveness

Monitor skip statistics for `claim_next` query:

```sql
-- Check if SKIP LOCKED is working effectively
-- High skip rate indicates good concurrency handling
SELECT
    relname,
    seq_scan,
    idx_scan,
    n_tup_hot_upd,
    n_tup_upd
FROM pg_stat_user_tables
WHERE relname = 'activity_queue';
```

---

### Phase 4: Connection Pool Analysis (30 min)

#### 4.1 sqlx Pool Metrics

Add connection pool metrics to health check endpoint:

```rust
// api/src/health/checks.rs - extend existing health check
pub struct PoolMetrics {
    pub size: u32,
    pub idle: u32,
    pub active: u32,
    pub max_connections: u32,
}

pub async fn get_pool_metrics(pool: &PgPool) -> PoolMetrics {
    PoolMetrics {
        size: pool.size(),
        idle: pool.num_idle(),
        active: pool.size() - pool.num_idle(),
        max_connections: pool.options().get_max_connections(),
    }
}
```

#### 4.2 Connection Pool Dashboard Query

```sql
-- Active connections by state
SELECT
    state,
    count(*) as connections,
    string_agg(DISTINCT application_name, ', ') as applications
FROM pg_stat_activity
WHERE datname = current_database()
GROUP BY state;

-- Long-running queries (potential pool exhaustion)
SELECT
    pid,
    now() - pg_stat_activity.query_start AS duration,
    query,
    state
FROM pg_stat_activity
WHERE (now() - pg_stat_activity.query_start) > interval '5 seconds'
AND state != 'idle';
```

---

### Phase 5: Profiling Report Generation (1 hour)

#### 5.1 Report Structure

Extend existing profiling output (`var/profiling-YYYYmmdd-HHMMSS/`):

```
var/profiling-YYYYmmdd-HHMMSS/
├── results.json              # Existing: benchmark results
├── summary.md                # New: human-readable summary
├── slow_queries.txt          # New: top queries by execution time
├── index_usage.txt           # New: index usage statistics
├── table_stats.txt           # New: table statistics
├── execution_plans.txt       # New: EXPLAIN ANALYZE outputs
├── lock_analysis.txt         # New: lock contention during benchmark
└── pool_metrics.json         # New: connection pool utilization
```

#### 5.2 Profiling CLI Output

```
$ kruxiaflow profile --benchmark

PostgreSQL Performance Profile
==============================

Hot Path Query Analysis
-----------------------
| Query           | Calls  | Avg (ms) | Max (ms) | % Total |
|-----------------|--------|----------|----------|---------|
| claim_next      | 10,000 | 0.8      | 12.3     | 45%     |
| event_poll      | 5,000  | 0.3      | 2.1      | 8%      |
| load_state      | 3,000  | 0.5      | 4.2      | 8%      |
| save_state      | 3,000  | 0.6      | 5.1      | 9%      |

Slow Queries (>10ms)
--------------------
⚠️  claim_next: 23 calls exceeded 10ms (max: 12.3ms)
    Recommendation: Check idx_queue_claimable index effectiveness

Index Usage
-----------
✅ idx_queue_claimable: 10,000 scans (high utilization)
✅ idx_events_workflow_id: 5,000 scans
⚠️  idx_queue_timeout_check: 0 scans (unused during test)

Connection Pool
---------------
Max: 20 | Peak Active: 15 | Avg Idle: 8
✅ Pool utilization healthy (75% peak)

Lock Contention
---------------
✅ No blocking locks detected during benchmark
FOR UPDATE SKIP LOCKED: 0 blocks, 234 skips (optimal)
```

---

## Performance Targets

Based on US-2.1/US-2.2 benchmark results (56 wf/sec baseline), targets for optimization:

| Metric                        | Current Baseline | Target (Epic 6) |
|-------------------------------|------------------|-----------------|
| claim_next avg latency        | TBD (profiling)  | <1ms            |
| event_poll avg latency        | TBD (profiling)  | <0.5ms          |
| Workflows/sec (throughput)    | 56 wf/sec        | >100 wf/sec   |
| P99 orchestration latency     | TBD              | <5ms            |
| Sequential scans              | TBD              | 0 for hot paths |

---

## Files to Create/Modify

### New Files
- `migrations/YYYYMMDD_profiling_extensions.up.sql` - pg_stat_statements + views
- `kruxiaflow/src/commands/profile.rs` - CLI profiling command
- `scripts/profile_queries.sh` - Profiling automation script

### Modified Files
- `kruxiaflow/src/commands/mod.rs` - Add profile command
- `kruxiaflow/src/main.rs` - Wire up profile command
- `api/src/health/checks.rs` - Add pool metrics

---

## Dependencies

- PostgreSQL 14+ (pg_stat_statements built-in)
- Existing benchmark crate (`profiling/`)
- Docker Compose environment for controlled profiling

---

## Success Criteria

1. **Query Baseline Established**: All hot path queries profiled with execution plans
2. **Slow Query Detection**: Any query >10ms identified with root cause
3. **Index Validation**: Confirm all indexes are being used effectively
4. **Connection Pool Tuning**: Optimal pool size determined for target throughput
5. **Lock-Free Hot Paths**: Confirm FOR UPDATE SKIP LOCKED prevents blocking
6. **Actionable Optimization Plan**: Clear recommendations for Epic 6 implementation

---

## Related Documentation

- [US-2.1: Automated Performance Test Suite](./US-2.1-automated-performance-test-suite.md)
- [US-2.2: Competitor Comparison Benchmarks](./US-2.2-competitor-comparison-benchmarks.md)
- [Architecture: Service Interfaces](../architecture.md#service-interface-pattern)

---

## Implementation Notes

### Why pg_stat_statements Over Other Tools

1. **Built-in**: No external dependencies, works with any PostgreSQL 14+
2. **Low overhead**: <5% performance impact when enabled
3. **Query normalization**: Automatically groups similar queries with different parameters
4. **Cumulative tracking**: Tracks total time across all executions

### FOR UPDATE SKIP LOCKED Analysis

The `claim_next` query uses `FOR UPDATE SKIP LOCKED` which is optimal for high-concurrency work queues. Profiling should confirm:
- No lock waits (rows are skipped, not blocked)
- Even distribution of claimed activities across workers
- Sub-millisecond claim latency under load

### Index Strategy Validation

Current partial indexes (`WHERE status IN (...)`) should be validated:
- `idx_queue_claimable` should be used for all claim_next queries
- Partial index should significantly reduce index size vs full index
- Check if index-only scans are possible (avoid heap fetches)

---

## Implementation Completed

### Files Created

| File                                                 | Description                                           |
|------------------------------------------------------|-------------------------------------------------------|
| `migrations/20251204000001_profiling_views.up.sql`   | Profiling views (v_slow_queries, v_index_usage, etc.) |
| `migrations/20251204000001_profiling_views.down.sql` | Rollback for profiling views                          |
| `kruxiaflow/src/commands/profile.rs`                 | CLI profiling command with full report generation     |

### Files Modified

| File                            | Changes                                        |
|---------------------------------|------------------------------------------------|
| `kruxiaflow/src/commands/mod.rs`| Added `pub mod profile;` export                |
| `kruxiaflow/src/main.rs`        | Added Profile command enum and wiring          |
| `api/src/health/responses.rs`   | Added PoolMetricsResponse struct               |
| `api/src/health/checks.rs`      | Added get_pool_metrics() function              |
| `api/src/health/mod.rs`         | Exported new pool metrics items                |
| `api/src/handlers/health.rs`    | Added pool_metrics_handler endpoint            |
| `api/src/handlers/mod.rs`       | Exported pool_metrics_handler                  |
| `api/src/routes.rs`             | Added GET /health/pool route                   |

### CLI Usage

```bash
# Full profiling report
kruxiaflow profile

# With EXPLAIN ANALYZE for hot paths
kruxiaflow profile --explain

# Verbose output with table statistics
kruxiaflow profile -v

# JSON output for automation
kruxiaflow profile --format json

# Reset pg_stat_statements
kruxiaflow profile --reset

# Filter by minimum query time
kruxiaflow profile --min-time-ms 1.0
```

### API Endpoints Added

- **GET /health/pool** - Returns connection pool metrics:
  ```json
  {
    "size": 10,
    "idle": 5,
    "active": 5,
    "max_connections": 20,
    "utilization_percent": 50.0,
    "status": "healthy"
  }
  ```

### Profiling Views Created (in PostgreSQL)

| View                     | Purpose                                      |
|--------------------------|----------------------------------------------|
| `v_slow_queries`         | Top queries by total execution time          |
| `v_index_usage`          | Index scan statistics                        |
| `v_unused_indexes`       | Indexes with zero scans (removal candidates) |
| `v_table_stats`          | Table-level statistics (scans, updates, etc.)|
| `v_lock_contention`      | Currently blocked locks                      |
| `v_connection_stats`     | Connections grouped by state                 |
| `v_long_running_queries` | Queries running > 5 seconds                  |

### Integration with Existing Profiling Script

The `scripts/profiling.sh` has been updated to automatically run `kruxiaflow profile`:
- Resets pg_stat_statements before benchmarks
- Collects query statistics after runs
- **NEW**: Runs `kruxiaflow profile --explain --format json` → `db_profile.json`
- **NEW**: Runs `kruxiaflow profile --explain -v` → `db_profile.txt`
- Saves results to `var/profiling-YYYYMMDD-HHMMSS/`

Output files now include:
```
var/profiling-YYYYmmdd-HHMMSS/
├── results.json              # Benchmark results
├── queries.txt               # pg_stat_statements raw output
├── db_profile.json           # Comprehensive DB profile (JSON)
├── db_profile.txt            # Comprehensive DB profile with EXPLAIN plans
├── memory_usage.csv          # Memory tracking
├── memory_analysis.txt       # Memory analysis summary
└── server-logs.txt           # Server logs during benchmark
```

---

## Performance Optimization Applied

### Event Poll Query Optimization

**Problem Identified**: The original event poll query used a LEFT JOIN pattern:

```sql
SELECT e.id, e.workflow_id, e.event_type, e.activity_key, e.payload, e.timestamp, e.iteration
FROM workflow_events e
LEFT JOIN workflow_event_consumers c ON c.consumer_id = $1
WHERE c.last_event_id IS NULL OR e.id > c.last_event_id
ORDER BY e.id ASC
LIMIT 100
```

This caused PostgreSQL to apply `id > c.last_event_id` as a **Filter** (post-join), not an **Index Condition** (pre-scan), resulting in full index scans reading millions of tuples (127M+ tuples read per profiling run).

**Solution**: Changed to scalar subquery pattern:

```sql
SELECT id, workflow_id, event_type, activity_key, payload, timestamp, iteration
FROM workflow_events
WHERE id > COALESCE(
    (SELECT last_event_id FROM workflow_event_consumers WHERE consumer_id = $1),
    '00000000-0000-0000-0000-000000000000'::uuid
)
ORDER BY id ASC
LIMIT 100
```

**Why it works**:
1. PostgreSQL evaluates scalar subqueries **ONCE** and treats the result as a constant
2. The `id > <constant>` can be used as an **Index Condition** for efficient range scans
3. The nil UUID fallback handles first-poll case (no checkpoint) - all UUIDv7s are greater than nil UUID
4. Uses the covering index `idx_events_consumer_poll` for index-only scans

**Query plan after optimization**:
```
Limit  (cost=8.32..32.87 rows=100 width=112)
  InitPlan 1
    ->  Index Scan using workflow_event_consumers_pkey  (cost=0.15..8.17 rows=1 width=16)
          Index Cond: (consumer_id = 'test'::text)
  ->  Index Scan using idx_events_consumer_poll on workflow_events  (cost=0.15..47.53 rows=193 width=112)
        Index Cond: (id > COALESCE((InitPlan 1).col1, '00000000-0000-0000-0000-000000000000'::uuid))
```

**Files Modified**:
- `core/src/events/postgres_event_source.rs` - Updated poll() query pattern

---

## Measured Results (2025-12-04)

### Before Optimization (var/profiling-20251204-140722)

| Metric                              | Value                |
|-------------------------------------|----------------------|
| Event Poll Query % of DB Time       | **73.7%**            |
| Event Poll Total Time               | 19,196.98 ms         |
| Event Poll Avg Time                 | 3.49 ms              |
| `workflow_events_pkey` Tuples Read  | **127,227,733**      |
| `idx_events_consumer_poll` Scans    | 72                   |

### After Optimization (var/profiling-20251204-144401)

| Metric                              | Value                |
|-------------------------------------|----------------------|
| Event Poll Query % of DB Time       | **1.8%**             |
| Event Poll Total Time               | 166.69 ms            |
| Event Poll Avg Time                 | 0.03 ms              |
| `workflow_events_pkey` Tuples Read  | **62,050**           |
| `idx_events_consumer_poll` Scans    | 175                  |

### Improvement Summary

| Metric                    | Before        | After     | Improvement        |
|---------------------------|---------------|-----------|-------------------|
| % of DB Time              | 73.7%         | 1.8%      | **97.6% reduction** |
| Total Query Time          | 19,196 ms     | 167 ms    | **115x faster**    |
| Tuples Read               | 127,227,733   | 62,050    | **2,050x reduction** |
| Query Plan                | Filter (post-join) | Index Cond | Proper pushdown |

The optimization successfully converted the event poll query from a bottleneck consuming 3/4 of all database time to a minor contributor at under 2%. The workload is now evenly distributed across INSERT and UPDATE operations as expected for a healthy workflow engine.
