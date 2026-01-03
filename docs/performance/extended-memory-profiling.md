# Extended Memory Profiling - 5 Minute Sustained Load Test

**Purpose**: Verify and characterize memory leak behavior during sustained workload

**Test Duration**: 5 minutes (300 seconds)
**Memory Samples**: 150 samples (every 2 seconds)
**Expected Workflows**: ~1,200-3,500 workflows (depending on throughput)

---

## Quick Start

```bash
# 1. Start the profiling environment
docker compose up kruxiaflow-profiling -d

# 2. Wait for server to be ready (watch logs)
docker compose logs -f kruxiaflow-profiling
# Wait for "Kruxia Flow server started" message

# 3. Set OAuth credentials
export KRUXIAFLOW_CLIENT_ID=kruxiaflow-dev-client
export KRUXIAFLOW_CLIENT_SECRET=a_zBZWlw8IsQaQm5C2xJPMgunAj4jkjzp4iTafATVcD8RU02yNEYqwCdLsoXIe8g
export DATABASE_URL=postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow_profiling

# 4. Run the 5-minute sustained throughput test
./scripts/profiling.sh --test test_sustained_throughput

# 5. Analyze results
cat var/benchmark-*/memory_analysis.txt
```

---

## What Gets Profiled

### Automatic Memory Tracking

The `docker-entrypoint-profiling.sh` script automatically:

1. **Samples memory every 2 seconds**:
   - RSS (Resident Set Size) - actual RAM usage
   - VSZ (Virtual Size) - total allocated virtual memory
   - CPU percentage

2. **Saves data to CSV**:
   - Docker container: `/opt/var/memory/memory_usage.csv`
   - Host (via volume): `var/memory/memory_usage.csv`

3. **Analyzed by benchmark script**:
   - Min/Max/Average memory usage
   - Memory growth over time
   - Growth rate (MB/second)
   - Leak detection warnings

### jemalloc Heap Profiling

Additionally, jemalloc creates heap dumps every 2^30 bytes (~1 GB) allocated:

- Location: `/opt/var/memory/jeprof.out.*.heap`
- Configured via `_RJEM_MALLOC_CONF` in docker-entrypoint-profiling.sh
- Can be analyzed post-test with `jeprof`

---

## Understanding the Output

### Memory Analysis Report

After the test completes, check `var/benchmark-*/memory_analysis.txt`:

```
Memory Usage Analysis
====================

RSS (Resident Set Size):
  Min:          126.00 MB
  Max:          222.00 MB
  Average:      174.00 MB
  Growth:        96.00 MB

VSZ (Virtual Size):
  Min:          450.00 MB
  Max:          546.00 MB
  Average:      498.00 MB

Duration:
  300 seconds (150 samples)

⚠️  WARNING: Potential memory leak detected
   Growth rate: 0.320 MB/second
```

### Memory Leak Thresholds

The script uses these heuristics:

| Growth Rate | Classification | Action |
|-------------|----------------|--------|
| **>0.1 MB/sec** | ⚠️ **WARNING: Potential memory leak** | Investigate immediately |
| **0.01-0.1 MB/sec** | ⚠️ **CAUTION: Memory growth** | Monitor, may be acceptable |
| **<0.01 MB/sec** | ✓ **Memory usage stable** | No action needed |

### Expected Behavior

For the 5-minute test at ~20-25 wf/sec:

**Without memory leak** (ideal):
```
RSS Growth:    10-20 MB  (0.03-0.07 MB/sec)
Assessment:    ✓ Memory usage stable
```

**With current suspected leak** (0.914 MB/sec from 105-second test):
```
RSS Growth:    274 MB (0.914 MB/sec)
Assessment:    ⚠️ WARNING: Potential memory leak
```

**Extrapolated for 5 minutes**:
```
Expected RSS:  Initial 126 MB → Final 400 MB (+274 MB)
Growth rate:   0.914 MB/sec
```

---

## Detailed Analysis Steps

### 1. View Memory Usage Over Time

```bash
# Get output directory
OUTPUT_DIR=$(ls -td var/benchmark-* | head -1)

# View CSV data
cat $OUTPUT_DIR/memory_usage.csv | head -20

# Example output:
# timestamp,rss_mb,vsz_mb,cpu_percent
# 1699564800,126,450,15.2
# 1699564802,128,452,14.8
# 1699564804,130,454,15.1
# ...
```

### 2. Plot Memory Growth (Optional)

If you have `gnuplot` installed:

```bash
OUTPUT_DIR=$(ls -td var/benchmark-* | head -1)

gnuplot <<EOF
set terminal png size 1200,600
set output '$OUTPUT_DIR/memory_plot.png'
set datafile separator ','
set xlabel 'Time (seconds)'
set ylabel 'Memory (MB)'
set title 'Memory Usage Over 5 Minutes'
set grid
set key left top
plot '$OUTPUT_DIR/memory_usage.csv' using (\$1-$(head -2 $OUTPUT_DIR/memory_usage.csv | tail -1 | cut -d',' -f1)):2 with lines title 'RSS', \
     '' using (\$1-$(head -2 $OUTPUT_DIR/memory_usage.csv | tail -1 | cut -d',' -f1)):3 with lines title 'VSZ'
EOF

open $OUTPUT_DIR/memory_plot.png
```

### 3. Analyze Heap Dumps (If Available)

If jemalloc created heap dumps:

```bash
# Find heap dumps
ls -lh var/memory/jeprof.out.*.heap

# Analyze final heap dump
FINAL_HEAP=$(ls -t var/memory/jeprof.out.*.heap | head -1)

# Generate allocation report
jeprof --show_bytes --text target/profiling/kruxiaflow "$FINAL_HEAP" > allocation_report.txt

# View top allocators
head -30 allocation_report.txt

# Generate flamegraph (requires graphviz)
jeprof --show_bytes --svg target/profiling/kruxiaflow "$FINAL_HEAP" > flamegraph.svg
open flamegraph.svg
```

### 4. Compare Multiple Runs

To track leak behavior over time:

```bash
# Run 1
./scripts/profiling.sh --test test_sustained_throughput --output-dir var/memory-test-run1

# Run 2 (different code version)
./scripts/profiling.sh --test test_sustained_throughput --output-dir var/memory-test-run2

# Compare growth rates
echo "Run 1:"
grep "Growth rate" var/memory-test-run1/memory_analysis.txt

echo "Run 2:"
grep "Growth rate" var/memory-test-run2/memory_analysis.txt
```

---

## Troubleshooting

### No Memory Data Collected

**Symptom**: `memory_analysis.txt` says "No memory data collected"

**Causes**:
1. Server not running in profiling mode
2. Memory tracking file not created
3. Server crashed during test

**Solution**:
```bash
# Verify profiling container is running
docker ps | grep kruxiaflow-profiling

# Check if memory file exists
ls -lh var/memory/memory_usage.csv

# Check server logs
docker compose logs kruxiaflow-profiling | tail -50

# Restart profiling container
docker compose down
docker compose up kruxiaflow-profiling -d
```

### Memory File Empty

**Symptom**: CSV file exists but has only header

**Solution**:
```bash
# The monitor script may not have started. Check logs:
docker compose logs kruxiaflow-profiling | grep "memory monitoring"

# Should see: "Starting Kruxia Flow server with memory monitoring..."

# If not, rebuild container:
docker compose build kruxiaflow-profiling
docker compose up kruxiaflow-profiling -d
```

### Test Times Out or Fails

**Symptom**: Test fails with timeout errors, workflows don't complete

**Causes**:
1. System under too much load
2. Memory exhaustion
3. Database connection issues

**Solution**:
```bash
# Check available memory
free -h

# Check database status
docker exec kruxiaflow-postgres psql -U kruxiaflow -d kruxiaflow_profiling -c "SELECT 1;"

# Reduce concurrency
# Edit benchmark/tests/load_tests.rs:
# Change: run_sustained_load_test(&client, definition_name, duration, 20)
# To:     run_sustained_load_test(&client, definition_name, duration, 10)

# Or reduce test duration
# Change: Duration::from_secs(300)
# To:     Duration::from_secs(120)  # 2 minutes
```

### Heap Dumps Not Created

**Symptom**: No `jeprof.out.*.heap` files in `var/memory/`

**Explanation**: Heap dumps are only created every ~1 GB of allocations. In a 5-minute test:

```
Throughput:     ~20-25 wf/sec
Workflows:      ~1,500 workflows
Memory per wf:  ~100 KB (estimated)
Total alloc:    ~150 MB (not enough for heap dump)
```

**To force heap dumps**:

Modify `docker-entrypoint-profiling.sh` line 21:

```bash
# Before (dumps every 1GB)
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:$PROFILE_DIR/jeprof.out,lg_prof_interval:30"

# After (dumps every 128MB)
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:$PROFILE_DIR/jeprof.out,lg_prof_interval:27"
```

Then rebuild: `docker compose build kruxiaflow-profiling && docker compose up kruxiaflow-profiling -d`

---

## Interpreting Results

### Scenario 1: No Memory Leak ✅

```
Growth:        15 MB over 300 seconds
Growth rate:   0.05 MB/second
Assessment:    ✓ Memory usage stable
```

**Interpretation**: Normal memory usage. Small growth is expected from:
- Connection pooling warmup
- Query cache population
- JIT compilation
- Internal buffers

**Action**: No fix needed. System is production-ready.

---

### Scenario 2: Small Memory Growth ⚠️

```
Growth:        45 MB over 300 seconds
Growth rate:   0.15 MB/second
Assessment:    ⚠️ CAUTION: Memory growth observed
```

**Interpretation**: Moderate growth. Acceptable for short-lived processes, but may accumulate over days.

**Extrapolation**:
- 1 hour: 540 MB growth
- 1 day: 13 GB growth ⚠️
- 1 week: 91 GB growth ❌

**Action**:
- If deploying for <1 hour: Acceptable
- If deploying for days/weeks: Investigate and fix

---

### Scenario 3: Significant Memory Leak ❌

```
Growth:        274 MB over 300 seconds
Growth rate:   0.914 MB/second
Assessment:    ⚠️ WARNING: Potential memory leak detected
```

**Interpretation**: Clear memory leak. System will OOM (Out Of Memory) over time.

**Extrapolation**:
- 1 hour: 3.3 GB growth ⚠️
- 6 hours: 20 GB growth ❌ (will OOM on most systems)

**Action**: **Must fix before production deployment**

**Investigation Steps**:
1. Analyze heap dumps with `jeprof` (see above)
2. Check for:
   - Workflow state accumulation (not cleaned up after completion)
   - Event buffer growth (no truncation/archival)
   - SQLx prepared statement cache unbounded
   - Connection leaks
3. Refer to `docs/performance/memory-profiling-guide.md` for detailed debugging

---

## Expected Suspects (Based on Performance Plan)

From `docs/performance/performance-optimization-plan.md`, the likely causes are:

### 1. Workflow State Accumulation (Most Likely)

**Evidence**: Completed workflows not cleaned up from `workflows` table

**Query to check**:
```sql
SELECT status, COUNT(*)
FROM workflows
GROUP BY status;

-- Expected after 5 min test: ~1,500 completed workflows
-- If all are still in memory: ~150 MB leak
```

**Fix**: Implement cleanup for completed workflows

---

### 2. Event Buffer Growth

**Evidence**: Events accumulating in `workflow_events` table without truncation

**Query to check**:
```sql
SELECT COUNT(*), pg_size_pretty(pg_total_relation_size('workflow_events'))
FROM workflow_events;

-- Expected after 5 min test: ~10,000-15,000 events
-- If not truncated: ~100 MB leak
```

**Fix**: Implement event archival/truncation

---

### 3. SQLx Statement Cache

**Evidence**: Prepared statement cache growing unbounded

**Note**: This is less likely but possible.

**Fix**: Configure max cache size in connection pool

---

## Next Steps After Verification

### If Memory Leak Confirmed

1. **Quantify the leak**:
   - Record growth rate (MB/sec)
   - Extrapolate to production runtime
   - Determine criticality

2. **Generate heap dumps for analysis**:
   - Reduce `lg_prof_interval` to 27 (128 MB dumps)
   - Re-run test
   - Analyze with `jeprof`

3. **Implement fixes**:
   - See `docs/performance/performance-optimization-plan.md` Section "Fix memory leak"
   - Implement workflow state cleanup
   - Add event truncation
   - Configure SQLx cache limits

4. **Verify fix**:
   - Re-run 5-minute test
   - Confirm growth rate <0.05 MB/sec

### If No Memory Leak

1. **Document baseline**:
   - Save results as `var/memory-baseline.json`
   - Include in performance regression tests

2. **Update performance plan**:
   - Remove memory leak from post-MVP tasks
   - Update `docs/performance/performance-optimization-plan.md`

3. **Production deployment**:
   - System is ready for long-running deployment
   - Add memory monitoring alerts (RSS > 500 MB)

---

## Reference

- **Performance Optimization Plan**: `docs/performance/performance-optimization-plan.md`
- **Memory Profiling Guide**: `docs/performance/memory-profiling-guide.md`
- **Benchmark Script**: `scripts/profiling.sh`
- **Sustained Test Implementation**: `benchmark/tests/load_tests.rs:283-304`
- **Memory Tracking Script**: `docker-entrypoint-profiling.sh:42-65`

---

**Last Updated**: 2025-11-10
**Test Duration**: Changed from 60s to 300s (5 minutes)
**Memory Samples**: 150 samples (one every 2 seconds)
