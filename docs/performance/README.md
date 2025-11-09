# Performance Documentation

This directory contains comprehensive performance analysis, optimization plans, and investigation results for StreamFlow.

## 📊 Current Status (2025-11-08)

**Performance**: 20.78 wf/sec sustained (99.9% success rate)
**Target**: 100+ wf/sec
**Gap**: 5× improvement needed
**Blocker**: 🔴 Memory leak (297% growth)

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

## 📈 Key Findings

### ✅ What's Working
- **Stable performance**: 20.78 wf/sec sustained
- **High success rate**: 99.9% (1,804/1,806)
- **No degradation**: Performance constant over time
- **Healthy infrastructure**: Threads, connections all good
- **62% improvement**: From previous 12.79 wf/sec

### 🔴 Critical Issues
- **Memory leak**: 297% growth (31 → 124 MB)
- **~35 KB per workflow** with no cleanup
- **Production blocker**: Would leak ~6.9 GB/day

### Root Cause (Most Likely)
1. Workflow states not cleaned after completion (90%)
2. Event buffers accumulating (70%)
3. SQLx statement cache unbounded (40%)

## 🎯 Next Steps

### Priority 1: Fix Memory Leak (CRITICAL)
```bash
# Step 1: Profile to find exact source
./scripts/profile_memory.sh

# Step 2: Review top allocations
cat var/memory-profile-*/allocation_report.txt | head -30

# Step 3: Implement fix in identified location
# Likely: core/src/orchestrator/workflow_state.rs
```

### Priority 2: Verify Performance
```bash
# Run full suite to confirm 62% gain is real
./scripts/profiling.sh
```

### Priority 3: Add to CI
- Memory leak detection
- Performance regression tests
- Automated profiling

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
- [ ] Throughput: >100 wf/sec sustained
- [ ] Latency: P99 <200ms
- [x] Success Rate: 99.9%+ ✅
- [ ] Memory: <20% growth over 90s
- [x] Monitoring: Automated ✅
- [x] Profiling: Tools ready ✅

### Current Progress
- Throughput: 20.78/100 wf/sec (21%)
- Latency: 1,685ms P99 (8× target)
- Success: 99.9% ✅
- Memory: 297% growth 🔴
- Tools: Complete ✅

**Next Milestone**: Fix memory leak → 50+ wf/sec
