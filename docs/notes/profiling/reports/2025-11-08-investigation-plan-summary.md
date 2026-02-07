# Performance Investigation: Sustained Throughput Degradation

## Investigation Goals

Answer the key questions from the performance optimization plan:
1. Does performance degrade linearly over 60 seconds or step-wise?
2. Is connection pool exhausted after 30-40 seconds?
3. Are event consumers multiplying instead of reusing?
4. Is there a memory leak in event processing or state management?

## Tools Implemented

### 1. Performance Monitoring Script (`scripts/monitor_performance.sh`)

Continuously monitors system metrics during test execution:
- **Memory usage**: RSS, VSZ, CPU% sampled every 2 seconds
- **Database connections**: Total, active, idle, waiting connections
- **Event consumer positions**: Tracks position for each consumer
- **Thread count**: Monitors worker thread lifecycle
- **System stats**: Database size, event count, workflow count, queue size

**Usage**:
```bash
./scripts/monitor_performance.sh \
    --server-pid <PID> \
    --db-url <URL> \
    --duration 90 \
    --interval 2 \
    --output-dir <DIR>
```

**Outputs**:
- `memory_usage.csv` - Time-series memory data
- `db_connections.csv` - Connection pool usage over time
- `consumer_positions.csv` - Event consumer checkpoint progression
- `thread_count.csv` - Thread lifecycle tracking
- `system_stats.csv` - Database growth metrics
- `monitoring_summary.txt` - Auto-generated summary

### 2. Monitoring Analysis Script (`scripts/analyze_monitoring.py`)

Analyzes collected monitoring data to answer investigation questions:

**Memory Analysis**:
- Detects linear vs non-linear growth patterns (R² correlation)
- Identifies sudden memory jumps (>20% in one interval)
- Calculates growth percentage and leak likelihood

**Connection Pool Analysis**:
- Tracks peak usage and saturation
- Detects pool exhaustion (waiting connections)
- Analyzes usage progression over time windows

**Consumer Analysis**:
- Identifies consumer multiplication
- Tracks event processing per consumer

**Thread Lifecycle**:
- Detects thread accumulation
- Compares first half vs second half of test

**Performance Degradation**:
- Measures event processing rate in thirds
- Identifies linear vs step-wise degradation

**Usage**:
```bash
python3 ./scripts/analyze_monitoring.py <output_dir>
```

### 3. Backoff Metrics Extraction (`scripts/extract_backoff_metrics.sh`)

Extracts orchestrator backoff intervals from server logs:
- Parses debug log lines for backoff intervals
- Tracks empty polls vs event polls
- Calculates empty/event poll ratio

**Usage**:
```bash
./scripts/extract_backoff_metrics.sh <server_log> <output_csv>
```

### 4. Orchestrator Instrumentation

Added debug logging to track backoff state:
```rust
tracing::debug!("No events found, backoff interval: {:?}", interval);
tracing::debug!("Polled {} events, resetting backoff", events.len());
```

This allows correlation of backoff behavior with performance degradation.

## Integration with Benchmark Script

The monitoring is automatically triggered for the `test_sustained_throughput` test:
1. Monitoring starts before test execution (90-second duration)
2. Test runs (60+ seconds)
3. Monitoring waits for completion
4. Backoff metrics extracted from server logs
5. Analysis script runs and generates report

**Location**: `scripts/profiling.sh` lines 455-497

## Expected Findings

Based on the current hypothesis, we expect to find:

### Scenario A: Memory Leak
- Memory growth >50% over 60 seconds
- Linear growth pattern (high R²)
- Possible leak in event processing or state management

### Scenario B: Connection Pool Exhaustion
- Connections peak and stay high
- Waiting connections appear after 30-40 seconds
- Performance degrades when pool saturates

### Scenario C: Consumer Multiplication
- Multiple "orchestrator" consumers detected
- Event processing duplicated across consumers
- Resource contention increases over time

### Scenario D: Thread Accumulation
- Thread count increases in second half of test
- Old worker threads not terminating
- Resource exhaustion from thread leak

### Scenario E: Backoff Degradation
- Backoff intervals increase over time
- Empty poll ratio increases
- System spends more time waiting than working

## Next Steps After Analysis

Based on findings, prioritize fixes:
1. **Memory leak** → Profile allocation hotspots, fix leaking data structures
2. **Pool exhaustion** → Increase max_connections, optimize connection reuse
3. **Consumer multiplication** → Fix consumer registration/cleanup
4. **Thread accumulation** → Fix worker lifecycle management
5. **Backoff issues** → Tune backoff parameters, add event notifications

## Files Modified

1. `scripts/monitor_performance.sh` (new) - Monitoring script
2. `scripts/analyze_monitoring.py` (new) - Analysis script
3. `scripts/extract_backoff_metrics.sh` (new) - Backoff extraction
4. `scripts/profiling.sh` (modified) - Integrated monitoring
5. `core/src/orchestrator/orchestrator.rs` (modified) - Added backoff logging

## Running the Investigation

```bash
# Run sustained test with full monitoring
./scripts/profiling.sh --test test_sustained_throughput --trace-level debug

# Results will be in var/benchmark-<timestamp>/monitoring/
# Analysis report: var/benchmark-<timestamp>/monitoring_analysis.txt
```

## Verification

The analysis script will provide definitive answers to all four investigation questions:
- ✅ or 🔴 for each hypothesis
- Quantitative data (percentages, counts, rates)
- Recommendations for next steps
