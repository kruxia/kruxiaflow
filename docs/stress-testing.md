# StreamFlow Stress Testing Guide

This guide explains how to run stress tests to identify system breaking points and determine capacity limits.

## Overview

Stress testing helps you understand:

1. **Breaking Points**: At what concurrency level does the system start failing?
2. **Bottlenecks**: What limits performance (CPU, memory, database)?
3. **Capacity**: How many workflows can the system handle?
4. **Graceful Degradation**: How does the system behave under overload?

## Quick Start

### Prerequisites

1. StreamFlow server running
2. PostgreSQL database with migrations applied
3. OAuth credentials configured

```bash
# Set credentials
export STREAMFLOW_CLIENT_ID="your-client-id"
export STREAMFLOW_CLIENT_SECRET="your-client-secret"

# Start server if not running
streamflow serve --port 8080
```

### Run a Quick Stress Test

```bash
# Quick test: 100 → 1,000 concurrent workflows
./scripts/stress-test.sh --quick
```

### Run a Standard Stress Test

```bash
# Standard test: 100 → 5,000 concurrent workflows
./scripts/stress-test.sh --standard
```

### Run a Full Stress Test

```bash
# Full test: 100 → 10,000 concurrent workflows
./scripts/stress-test.sh --full
```

## Test Profiles

| Profile    | Initial | Peak    | Step Size | Duration/Step | Est. Time |
|------------|---------|---------|-----------|---------------|-----------|
| `--quick`  | 100     | 1,000   | 100       | 15s           | ~3 min    |
| `--standard` | 100   | 5,000   | 300       | 30s           | ~10 min   |
| `--full`   | 100     | 10,000  | 500       | 30s           | ~15 min   |

## Custom Tests

### Custom Peak Concurrency

```bash
# Test up to 2,000 concurrent workflows
./scripts/stress-test.sh --peak 2000
```

### Custom Step Size

```bash
# Smaller steps for more granular results
./scripts/stress-test.sh --peak 5000 --step-size 200
```

### Custom Workflow

```bash
# Use a different workflow definition
./scripts/stress-test.sh --standard --workflow parallel_bench_10
```

### Stop on Failure

```bash
# Stop immediately when breaking point detected
./scripts/stress-test.sh --full --stop-on-failure
```

## Using the CLI Directly

For more control, use the stress-test binary directly:

```bash
cargo run --package streamflow-profiling --bin stress-test --release -- \
  --initial-concurrent 100 \
  --peak-concurrent 5000 \
  --step-size 250 \
  --step-duration 45 \
  --workflow sequential_bench_5 \
  --error-threshold 0.03 \
  --latency-threshold 3000 \
  --output-dir my-stress-test
```

### CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--quick` | - | Quick preset (100 → 1,000) |
| `--standard` | - | Standard preset (100 → 5,000) |
| `--full` | - | Full preset (100 → 10,000) |
| `--initial-concurrent` | 100 | Starting concurrent workflows |
| `--peak-concurrent` | 10,000 | Maximum concurrent workflows |
| `--step-size` | 500 | Increase per step |
| `--step-duration` | 30 | Seconds per step |
| `--cooldown` | 5 | Seconds between steps |
| `--workflow` | sequential_bench_5 | Workflow definition |
| `--error-threshold` | 0.05 | Error rate for breaking point |
| `--latency-threshold` | 5000 | P99 latency (ms) for breaking point |
| `--stop-on-failure` | true | Stop at breaking point |
| `--workflow-timeout` | 60 | Workflow completion timeout (s) |
| `--output-dir` | auto | Directory for results |
| `--monitor-resources` | true | Enable resource monitoring |
| `--monitor-interval` | 1000 | Resource sample interval (ms) |

## Understanding Results

### Output Files

After a stress test, you'll find:

```
var/stress-test-YYYYMMDD-HHMMSS/
├── stress-test-results.json    # Full results in JSON
├── stress-test-summary.md      # Human-readable summary
├── bottleneck-report.md        # Bottleneck analysis
└── capacity-planning.md        # Capacity recommendations
```

### Breaking Point

A breaking point is detected when any of these occur:

1. **Error Rate Exceeded**: More than 5% of workflows fail
2. **Latency Exceeded**: P99 latency above 5 seconds
3. **Throughput Degraded**: More than 50% drop from baseline

### Capacity Estimate

The report includes capacity estimates:

- **Safe Concurrent**: Comfortable operating limit (80% of max)
- **Max Concurrent**: Maximum tested without failures
- **Limiting Factor**: What caused the breaking point

### Bottleneck Categories

| Category | Indicators | Remediation |
|----------|------------|-------------|
| CPU | CPU > 85% | Add instances or CPU cores |
| Memory | Memory growth, OOM risk | Profile and fix leaks |
| Database | Pool exhaustion, slow queries | Increase connections, optimize queries |
| Network | Connection errors | Check network stability |

## Running Tests via Cargo

For integration with CI or scripting:

```bash
# Run all stress tests
cargo test --package streamflow-profiling --test stress_tests -- --ignored --nocapture

# Run specific test
cargo test --package streamflow-profiling --test stress_tests test_stress_quick -- --ignored --nocapture

# Run graceful degradation test
cargo test --package streamflow-profiling --test stress_tests test_graceful_degradation -- --ignored --nocapture

# Run recovery test
cargo test --package streamflow-profiling --test stress_tests test_recovery_after_overload -- --ignored --nocapture
```

## Graceful Degradation Testing

To verify the system degrades gracefully under extreme load:

```bash
cargo test --package streamflow-profiling --test stress_tests test_graceful_degradation -- --ignored --nocapture
```

This test:
1. Pushes past normal capacity
2. Continues even after breaking point
3. Verifies errors are handled (not crashes)
4. Reports overall success rate

## Recovery Testing

To verify the system recovers after overload:

```bash
cargo test --package streamflow-profiling --test stress_tests test_recovery_after_overload -- --ignored --nocapture
```

This test:
1. Establishes baseline throughput
2. Applies heavy overload
3. Allows cooldown period
4. Verifies recovery to 80%+ of baseline

## Best Practices

### Before Testing

1. **Isolate the environment**: Don't test against production
2. **Fresh database**: Start with clean state for reproducible results
3. **Monitor externally**: Use Grafana/Prometheus for additional visibility
4. **Plan for duration**: Full tests can take 15+ minutes

### During Testing

1. **Watch for crashes**: The test should complete without server crashes
2. **Monitor resources**: Check CPU, memory, disk on the server
3. **Check database**: Watch for connection pool exhaustion

### After Testing

1. **Review breaking point**: Understand what limited capacity
2. **Check recommendations**: Review the bottleneck report
3. **Document baseline**: Save results for future comparison
4. **Plan improvements**: Prioritize based on bottleneck analysis

## Troubleshooting

### Server Not Accessible

```
Error: StreamFlow server not accessible at http://localhost:8080
```

Start the server:
```bash
streamflow serve --port 8080
```

### Credential Errors

```
Error: STREAMFLOW_CLIENT_ID environment variable not set
```

Set OAuth credentials:
```bash
export STREAMFLOW_CLIENT_ID="your-client-id"
export STREAMFLOW_CLIENT_SECRET="your-client-secret"
```

### Workflow Definition Not Found

Register workflow definitions:
```bash
cargo run --package streamflow-profiling --bin register-workflows
```

### Test Hangs

If a test seems stuck:
1. Check server logs for errors
2. Check database connection status
3. Reduce peak concurrency and try again

### Memory Issues

If the test runner runs out of memory:
1. Reduce `--peak-concurrent`
2. Reduce `--step-size`
3. Run on a machine with more RAM

## Baseline Results (December 2025)

### Tested Configuration

| Component            | Configuration                    |
|----------------------|----------------------------------|
| PostgreSQL           | max_connections=300, 2GB RAM     |
| API Server Pool      | max=200, min=20, timeout=10s     |
| Workers              | 20 workers, poll batch=1         |
| Client Poll Interval | 200ms                            |

### Capacity Findings

| Concurrent Workflows | Throughput  | Success Rate | P99 Latency |
|----------------------|-------------|--------------|-------------|
| 100                  | 49 wf/sec   | 100%         | 2,660ms     |
| 200                  | 48 wf/sec   | 100%         | 4,224ms     |
| 300                  | ~25 wf/sec  | 98.7%        | 11,000ms+   |

**Recommended Operating Limit**: 200 concurrent workflows (~48 wf/sec, 2,880 wf/min)

### Optimization Findings

| Change                     | Impact                              |
|----------------------------|-------------------------------------|
| Client poll 50ms → 200ms   | **+32% throughput** (recommended)   |
| Workers 20 → 30            | -10% throughput (not recommended)   |
| Poll batch 1 → 10          | -65% at high load (not recommended) |
| PostgreSQL memory tuning   | No measurable improvement           |

### Bottleneck Analysis

The primary bottleneck is **query volume**, not query performance:
- Individual queries execute in <1ms
- 90% of activity polls return empty (no work available)
- Status polling from clients creates high query load
- Connection pool saturates before CPU/memory limits

### Scaling Recommendations

For capacity beyond 200 concurrent workflows:
1. **Replace status polling with WebSocket/SSE push**
2. **Add Redis for activity queue** (reduces DB polling)
3. **Use PgBouncer** for connection pooling
4. **Horizontal scaling** with multiple API instances

## Related Documentation

- [Performance Testing Guide](./performance/README.md)
- [Architecture](./architecture.md)
- [US-2.4 Implementation Plan](./implementation/US-2.4-stress-testing-capacity-planning.md)
