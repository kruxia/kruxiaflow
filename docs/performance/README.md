# Performance Documentation

This directory contains comprehensive performance analysis, optimization plans, and investigation results for StreamFlow.

## 📊 Current Status (December 2025)

**Capacity**: 200 concurrent workflows at 48 wf/sec sustained (100% success rate)
**Throughput**: ~2,880 workflows/minute
**Breaking Point**: 300 concurrent (latency threshold)
**Bottleneck**: Query volume (polling patterns), not CPU/memory

## 📁 Document Index

### Executive Summary
- **[INVESTIGATION-SUMMARY.md](INVESTIGATION-SUMMARY.md)** - START HERE
  - Complete investigation results
  - Key findings and next steps
  - Tools created and action items

### Detailed Reports
- **[investigation-results-2025-11-08.md](investigation-results-2025-11-08.md)**
  - Full investigation findings
  - Test results and analysis
  - Answers to all 4 investigation questions

- **[memory-leak-visualization.md](memory-leak-visualization.md)**
  - Visual analysis of memory growth
  - Timeline and phase breakdown
  - Root cause hypotheses ranked by likelihood

### Planning Documents
- **[performance-optimization-plan.md](performance-optimization-plan.md)**
  - Master optimization plan
  - Progress tracking (Phases 1-4)
  - Historical performance evolution
  - Immediate next steps

- **[investigation-plan-summary.md](investigation-plan-summary.md)**
  - Monitoring tools documentation
  - Investigation methodology
  - Usage instructions

### Historical Results
- **[performance-2025-11-08-17-08.md](performance-2025-11-08-17-08.md)**
  - Comprehensive profiling run
  - All 4 benchmark scenarios
  - Database query analysis

## 🚀 Quick Start

### Run Performance Investigation
```bash
# Full investigation with monitoring
./scripts/profiling.sh --test test_sustained_throughput --trace-level debug

# Results in: var/benchmark-<timestamp>/monitoring_analysis.txt
```

### Profile Memory
```bash
# Build with jemalloc and run profiling
./scripts/profile_memory.sh

# Results in: var/memory-profile-<timestamp>/
```

### Run All Benchmarks
```bash
# Full benchmark suite (all 4 scenarios)
./scripts/profiling.sh

# Results in: var/benchmark-<timestamp>/results.json
```

## 🛠️ Tools Available

### Monitoring Scripts
1. **`scripts/monitor_performance.sh`**
   - Real-time system monitoring
   - Memory, connections, threads, consumers
   - Auto-summary generation

2. **`scripts/analyze_monitoring.py`**
   - Automated analysis of monitoring data
   - Memory leak detection
   - Performance degradation analysis

3. **`scripts/extract_backoff_metrics.sh`**
   - Parse orchestrator backoff from logs
   - Calculate poll efficiency

4. **`scripts/profile_memory.sh`**
   - Jemalloc heap profiling
   - Flamegraph generation
   - Top allocation report

### Benchmark Integration
- Monitoring automatically enabled for `test_sustained_throughput`
- Analysis runs after test completion
- Results saved with benchmark output

## 📈 Key Findings (December 2025 Stress Testing)

### ✅ What's Working
- **Stable performance**: 48 wf/sec sustained at 200 concurrent
- **High success rate**: 100% up to 200 concurrent
- **Graceful degradation**: No crashes under overload, only timeouts
- **Memory stability**: Fixed (no leak detected in stress tests)

### 🔶 Current Limits
- **Breaking point**: 300 concurrent workflows
- **Bottleneck**: Query volume from polling (not query performance)
- **90% empty polls**: Workers polling when no work available
- **Connection pool saturation**: Before CPU/memory limits

### Optimization Results
| Change                   | Impact                    |
|--------------------------|---------------------------|
| Client poll 200ms        | +32% throughput ✅        |
| More workers (20→30)     | -10% throughput ❌        |
| Batch polling (1→10)     | -65% at high load ❌      |
| PostgreSQL memory tuning | No improvement ⚪         |

## 🎯 Next Steps

### Priority 1: Reduce Polling Overhead (Post-MVP)
To scale beyond 200 concurrent workflows:
1. **WebSocket/SSE for workflow status** - Replace client polling
2. **Adaptive worker backoff** - Reduce empty poll attempts
3. **Redis activity queue** - Higher throughput for activity polling

### Priority 2: Connection Pool Optimization
```bash
# Consider PgBouncer for connection pooling
# Current: 200 API + 50 orchestrator connections
```

### Priority 3: Horizontal Scaling Validation
- Test multiple API server instances
- Validate orchestrator failover
- Document scaling patterns

## 📊 Benchmark Scenarios

### 1. Sequential (100 workflows, 5 activities)
- **Current**: 16.52 wf/sec
- **Target**: 100 wf/sec
- **Status**: ✅ Passing, 6× from target

### 2. Parallel (50 workflows, 10 parallel activities)
- **Current**: 1.56 wf/sec (92% success)
- **Target**: 50 wf/sec
- **Status**: 🔴 Failures, needs investigation

### 3. High Concurrency (300 workflows, 3 activities, 100 concurrent)
- **Current**: 35.87 wf/sec
- **Target**: 100 wf/sec
- **Status**: ✅ Passing, 2.8× from target (BEST)

### 4. Sustained Throughput (60s, 20 concurrent)
- **Current**: 20.78 wf/sec
- **Target**: 100 wf/sec
- **Status**: ⚠️ Memory leak detected

## 🔬 Investigation Methodology

### 1. System Monitoring
- Memory usage tracking (RSS, VSZ, CPU)
- Database connection monitoring
- Thread lifecycle tracking
- Event consumer position tracking

### 2. Performance Analysis
- Linear vs step-wise degradation detection
- Connection pool saturation analysis
- Memory leak identification (R² correlation)
- Throughput consistency verification

### 3. Memory Profiling
- Jemalloc heap profiling
- Allocation call graph generation
- Top allocation identification
- Growth pattern analysis

## 📚 Related Documentation

### Internal
- [Architecture](../architecture.md) - System design
- [Implementation Plans](../implementation/) - Feature plans
- [Post-MVP](../post-mvp.md) - Future optimizations

### External
- [StreamFlow Benchmark](../../benchmark/) - Test code
- [Core Orchestrator](../../core/src/orchestrator/) - Orchestration logic
- [Event Source](../../core/src/events/) - Event streaming

## 🤝 Contributing

### Adding New Benchmarks
1. Add test to `benchmark/tests/load_tests.rs`
2. Update `scripts/profiling.sh` test list
3. Document expected metrics
4. Add to CI pipeline

### Performance Regression Detection
1. Run before/after benchmarks
2. Compare `results.json` files
3. Ensure <10% regression in any metric
4. Document any intentional trade-offs

### Profiling New Issues
1. Use monitoring scripts for long-running tests
2. Use jemalloc profiling for memory issues
3. Use tracing for latency issues
4. Document findings in this directory

## 📞 Support

For questions about performance:
1. Review [INVESTIGATION-SUMMARY.md](INVESTIGATION-SUMMARY.md)
2. Check [performance-optimization-plan.md](performance-optimization-plan.md)
3. Run profiling tools yourself
4. Create issue with profiling data

## 🎯 Success Metrics

### MVP Goals
- [x] Throughput: 48 wf/sec sustained ✅
- [x] Capacity: 200 concurrent workflows ✅
- [x] Success Rate: 100% ✅
- [x] Memory: Stable (no leak) ✅
- [x] Graceful Degradation: Confirmed ✅
- [x] Stress Testing: Complete ✅

### Current Performance (December 2025)
| Metric               | Value                     |
|----------------------|---------------------------|
| Concurrent Capacity  | 200 workflows             |
| Throughput           | 48 wf/sec (2,880 wf/min)  |
| Success Rate         | 100%                      |
| P99 Latency @ 200    | 4,224ms                   |
| Breaking Point       | 300 concurrent            |

**Next Milestone**: Scale beyond 200 concurrent (requires architectural changes)
