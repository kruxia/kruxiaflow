# StreamFlow Profiling Quick Start Guide

This guide shows you how to run comprehensive performance profiling to identify bottlenecks.

## Prerequisites

1. **Environment variables set** (source .envrc or set manually):
   ```bash
   export STREAMFLOW_CLIENT_ID="streamflow-dev-client"
   export STREAMFLOW_CLIENT_SECRET="dev-secret-key"
   export STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM="$(cat dev-keys/public.pem)"
   export STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM="$(cat dev-keys/private.pem)"
   ```

2. **Install cargo-flamegraph** (if not already installed):
   ```bash
   cargo install flamegraph
   ```

3. **PostgreSQL with pg_stat_statements** (already configured in docker-compose.yml):
   - The Docker PostgreSQL container has pg_stat_statements enabled
   - No manual setup required if using `docker-compose up -d postgres`

## Step 1: Run Profiling

Run the benchmark script with profiling enabled:

```bash
# Profile a specific benchmark
./scripts/profiling.sh --test test_sequential_workflow_load

# Or profile other benchmarks:
./scripts/profiling.sh --test test_parallel_workflow_load
./scripts/profiling.sh --test test_high_concurrency_load
./scripts/profiling.sh --test test_sustained_throughput
```

**Note**:
- You must specify `--test TEST_NAME` when using `--profile`
- Profiling the entire suite produces unclear results

The script will:
- ✅ Set up database (drop/recreate/migrate)
- ✅ Seed OAuth client
- ✅ Build release binary
- ✅ Start StreamFlow server with debug logging (RUST_LOG=streamflow=debug)
- ✅ Reset pg_stat_statements for clean measurement
- ✅ Register workflow definitions
- ✅ Start CPU profiling (flamegraph attached to server process)
- ✅ Run the specified benchmark test
- ✅ Stop profiling and generate flamegraph
- ✅ Collect PostgreSQL slow queries (> 1ms)
- ✅ Save server logs

**Duration**: ~5-10 minutes (includes setup and compilation)

## Step 2: Review Results

The profiling data is saved to `profiling/results-TIMESTAMP/`:

```bash
# Find the latest profiling results
ls -lt profiling/results-*

# View the directory contents
ls -lh profiling/results-20251108-123456
```

**Output files:**
- `flamegraph.svg` - CPU flamegraph showing hot paths
- `server.log` - StreamFlow server logs with debug output
- `slow-queries.txt` - PostgreSQL queries slower than 1ms
- `flamegraph.log` - Profiler output/errors (if any)

## Step 3: Review Detailed Data

### Flamegraph (CPU Profiling)

Open the flamegraph in your browser:

```bash
open profiling/results-*/flamegraph.svg
```

**How to read a flamegraph**:
- **Width of bars** = percentage of total CPU time (wider = more time spent)
- **Height of bars** = call stack depth (taller = deeper call chains)
- **Color** = just for differentiation (no specific meaning)

**What to look for**:
- **Wide bars at the top** = hot paths (bottlenecks)
- **`serde_json`** functions = JSON serialization overhead
- **`sqlx`** / database functions = query execution time
- **`tokio::sync`** / `parking_lot` = lock contention
- **`poll`** functions = async overhead
- **Flat, wide regions** = tight loops or blocking operations

### Slow Queries

Review queries slower than 1ms:

```bash
cat profiling/results-*/slow-queries.txt
```

**What to look for**:
- Queries with **high mean_exec_time** (> 10ms is concerning)
- Queries with **high calls** count + slow execution (amplified impact)
- Queries that could benefit from indexes
- Sequential scans on large tables

**Example output**:
```
 query                                          | calls | mean_exec_time | total_exec_time
------------------------------------------------+-------+----------------+-----------------
 SELECT * FROM workflows WHERE status = $1     |  500  |     15.2       |     7600.0
 UPDATE activities SET status = $1 WHERE id=$2  |  1500 |     8.3        |    12450.0
```

### Server Logs

Review debug logs for timing and errors:

```bash
# View recent logs
tail -100 profiling/results-*/server.log

# Search for specific patterns
grep "ERROR" profiling/results-*/server.log
grep "took" profiling/results-*/server.log  # Find timing logs
```

Compare before/after stats:

```bash
diff profiling-results-*/db-stats-before.txt profiling-results-*/db-stats-after.txt
```

**What to look for**:
- **High `seq_scan` and `seq_tup_read`** = need indexes
- **Low `idx_scan`** = indexes not being used
- **High active connections** = pool exhaustion
- **Locks with `waiting > 0`** = lock contention

### Server Logs

Search for errors and warnings:

```bash
grep -i "error\|warn\|slow" profiling-results-*/server.log | less
```

## Step 4: Identify Top 3 Bottlenecks

Based on the profiling data, identify the top 3 bottlenecks:

### Example Analysis Process

1. **Check flamegraph**: Is database or serialization the hot path?
2. **Check slow queries**: Which queries take the most total time?
3. **Check EXPLAIN**: Are there sequential scans (missing indexes)?
4. **Check metrics**: Is throughput or latency the main problem?

### Common Bottlenecks

| Symptom                                      | Root Cause                     | Fix                                                                                             |
|---------------------------------------------|--------------------------------|--------------------------------------------------------------------------------------------------|
| Flamegraph shows `sqlx` taking ~80% CPU     | Slow database queries / hot SQL| Add missing indexes, optimize queries (use EXPLAIN ANALYZE), batch statements, add caching      |
| `EXPLAIN` shows "Seq Scan on activity_queue"| Missing index                  | Create a partial index: `CREATE INDEX CONCURRENTLY idx_activity_queue_pending ON activity_queue(namespace, name, scheduled_for) WHERE status = 'pending';` |
| Slow queries with high `mean_time_ms`       | Inefficient query or missing index | Rewrite the query, add appropriate indexes, use prepared statements, run `EXPLAIN (ANALYZE, BUFFERS)` |
| Connection stats show active ≈ max          | Pool exhaustion                | Increase pool `max_connections`, tune min/max, add backpressure or limit concurrent clients     |
| Flamegraph shows `serde_json` ~30% CPU      | Serialization overhead         | Use binary formats (MessagePack), reuse serializers, reduce payload size, serialize off hot path |
| Server log shows "timeout" or "lock wait"   | Lock contention                | Inspect long transactions, reduce lock scope, use row-level locks, avoid long-running migrations |

## Step 5: Implement Fixes

Based on identified bottlenecks, implement fixes following the priority order in `docs/performance-optimization-plan.md`:

### Quick Wins (Tier 1)

**Add missing indexes**:
```sql
-- Activity queue index for pending activities
CREATE INDEX CONCURRENTLY idx_activity_queue_pending
ON activity_queue(namespace, name, scheduled_for)
WHERE status = 'pending';

-- Event polling index
CREATE INDEX CONCURRENTLY idx_workflow_events_id
ON workflow_events(id);
```

**Increase connection pool** (in code):
```rust
PgPoolOptions::new()
    .min_connections(10)
    .max_connections(100)  // Increased from 20
    .connect(&database_url)
    .await?
```

**Reduce event polling backoff** (in code):
```rust
AdaptiveBackoff::new(
    Duration::from_millis(1),    // Min: 1ms (was 10ms)
    Duration::from_millis(500),  // Max: 500ms (was 5s)
    1.2,                         // Multiplier
)
```

## Step 6: Re-run Profiling

After implementing fixes, re-run profiling to measure improvement:

```bash
# Re-run the same benchmark
./scripts/profile_benchmarks.sh test_sequential_workflow_load

# Analyze new results
./scripts/analyze_profiling_results.sh profiling-results-NEWDATE

# Compare metrics manually
diff profiling-results-OLDDATE/metrics-summary.txt \
     profiling-results-NEWDATE/metrics-summary.txt
```

## Step 7: Track Progress

Update `docs/performance-optimization-plan.md` with:

1. Bottlenecks identified
2. Fixes implemented
3. Performance improvement (before/after metrics)
4. Next optimization targets

## Profiling Cheat Sheet

```bash
# Quick profiling run
./scripts/profile_benchmarks.sh

# Analyze latest results
./scripts/analyze_profiling_results.sh $(ls -dt profiling-results-* | head -1)

# View flamegraph
open $(ls -t profiling-results-*/flamegraph-*.svg | head -1)

# Check slow queries
cat $(ls -dt profiling-results-* | head -1)/slow-queries.txt

# Check EXPLAIN output
cat $(ls -dt profiling-results-* | head -1)/explain-analysis.txt

# Search server logs for errors
grep -i error $(ls -dt profiling-results-* | head -1)/server.log
```

## Troubleshooting

### Flamegraph fails with "permission denied"

Run with `sudo`:
```bash
sudo -E ./scripts/profile_benchmarks.sh
```

Or add your user to the `perf_event` group (Linux).

### pg_stat_statements not available

Install and enable the extension:
```sql
-- In postgresql.conf:
shared_preload_libraries = 'pg_stat_statements'

-- Restart PostgreSQL, then:
CREATE EXTENSION pg_stat_statements;
```

### Server fails to start

Check server logs:
```bash
cat profiling-results-*/server.log
```

Common issues:
- Port 8080 already in use: `lsof -i :8080` and kill the process
- Database connection failed: Check `DATABASE_URL`
- Missing environment variables: Check all `STREAMFLOW_*` vars are set

### Benchmark times out

The benchmark has a 30-second timeout per workflow. If workflows are taking longer:

1. This indicates severe performance issues
2. Check `server.log` for errors
3. Check database is responsive: `psql $DATABASE_URL -c "SELECT 1"`
4. Reduce benchmark size temporarily to get profiling data

## Reference

- **Full optimization plan**: `docs/performance-optimization-plan.md`
- **Performance testing guide**: `docs/performance-testing.md`
- **Architecture**: `docs/architecture.md`

## Quick Performance Targets

| Metric        | Current        | Target         | Gap       |
|---------------|----------------|----------------|-----------|
| Throughput    | ~2–27 wf/sec   | >100 wf/sec    | 4–50×     |
| P99 Latency   | 5–30 s         | <200 ms        | 25–150×   |
| Success Rate  | 94–100%        | >99%           | ✓         |

Once profiling identifies bottlenecks, refer to the optimization plan for detailed fix strategies.
