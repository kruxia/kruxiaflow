# Performance Reports Index

This directory contains unified performance reports for StreamFlow benchmarks.

---

## Recent Reports

### [2025-11-10-PARALLEL-WORKFLOW-INVESTIGATION.md](./2025-11-10-PARALLEL-WORKFLOW-INVESTIGATION.md)
**Parallel Workflow Investigation** - November 10, 2025

Investigation of apparent parallel workflow regression that turned out to be a logging artifact.

- **Status**: ✅ No Bug - System Working Correctly
- **Issue**: Appeared to show only 1/10 parallel activities completing
- **Root Cause**: INFO logs only show when scheduling occurs; fan-in processes 9 events "silently"
- **Finding**: All events processed correctly, dependency logic perfect
- **Key Insights**:
  - Fan-in dependency evaluation working as designed
  - Logging artifact created illusion of missing events
  - Performance variance due to test isolation vs sequential execution
  - System architecture validated

**Use this report for**: Understanding parallel workflow execution, interpreting logs, debugging fan-in patterns

---

### [25-11-09-23-46-PRODUCTION-BASELINE.md](./25-11-09-23-46-PRODUCTION-BASELINE.md)
**Production Baseline** - November 9, 2025, 23:46 UTC

The current production-ready performance baseline.

- **Status**: ✅ Production Ready
- **Logging**: INFO (verbose_tracing=false)
- **Git SHA**: 7a12655 (with bugfix)
- **Key Metrics**:
  - Sequential: 16.77 wf/sec (100% success)
  - Parallel: 22.77 wf/sec (100% success)
  - High Concurrency: 56.40 wf/sec (100% success)
  - Sustained: 23.71 wf/sec (99.6% success)
- **Highlights**:
  - 6-14x improvement over buggy code
  - Bugfix verified - no duplicate scheduling
  - 99.6-100% success rate
  - Sub-second latency for most operations
  - Ready for production deployment

**Use this report for**: Production monitoring baselines, capacity planning, SLA definitions

---

### [25-11-09-23-33-TRACE-BUGFIX-VERIFIED.md](./25-11-09-23-33-TRACE-BUGFIX-VERIFIED.md)
**Bugfix Verification with TRACE Logging** - November 9, 2025, 23:33 UTC

Verification run with detailed trace logging to confirm bugfix and analyze orchestration timing.

- **Status**: ✅ Bugfix Verified
- **Logging**: TRACE (verbose_tracing=true)
- **Git SHA**: 7a12655 (with bugfix)
- **Key Metrics**:
  - Sequential: 16.28 wf/sec (100% success)
  - Parallel: 17.73 wf/sec (100% success)
  - High Concurrency: 52.20 wf/sec (100% success)
  - Sustained: 23.92 wf/sec (99.5% success)
- **Highlights**:
  - 11,337 ActivityScheduled events skipped (fix confirmed)
  - 6-11x improvement over buggy code
  - Detailed orchestration timing captured
  - Dependency evaluation: ~35-80µs (sub-100µs!)
  - Total event processing: ~2-10ms
  - TRACE overhead: 3-28% (acceptable)

**Use this report for**: Understanding orchestration internals, debugging performance issues, optimization targets

---

### [25-11-09-23-09-CRITICAL-BUG-FOUND.md](./25-11-09-23-09-CRITICAL-BUG-FOUND.md)
**Critical Bug Discovery** - November 9, 2025, 23:09 UTC

The benchmark run that discovered the activity re-scheduling loop bug.

- **Status**: ❌ Critical Bug Found
- **Logging**: INFO (verbose_tracing=false)
- **Git SHA**: 7a12655 (before bugfix)
- **Key Metrics** (with bug):
  - Sequential: 2.88 wf/sec (99% success)
  - Parallel: 1.64 wf/sec (98% success) - **BROKEN**
  - High Concurrency: 9.32 wf/sec (99.7% success)
  - Sustained: 19.05 wf/sec (99% success)
- **Bug Description**:
  - Orchestrator re-scheduled already-scheduled activities
  - 10x database overhead (100+ inserts instead of 10)
  - Parallel workflows slowest (should be fastest)
  - High timeout rates
- **Fix**:
  - Skip ActivityScheduled events (observability only)
  - Update state immediately when scheduling

**Use this report for**: Understanding the bug, lessons learned, regression prevention

---

### Investigation Reports (2025-11-08)

**Context**: Investigation of sustained throughput degradation and memory leak

- **[25-11-08-INVESTIGATION-COMPLETE.md](./25-11-08-INVESTIGATION-COMPLETE.md)** - Investigation completion summary
- **[25-11-08-INVESTIGATION-SUMMARY.md](./25-11-08-INVESTIGATION-SUMMARY.md)** - High-level investigation summary
- **[25-11-08-investigation-plan-summary.md](./25-11-08-investigation-plan-summary.md)** - Investigation plan and questions
- **[25-11-08-investigation-results.md](./25-11-08-investigation-results.md)** - Detailed investigation results
- **[25-11-08-memory-leak-visualization.md](./25-11-08-memory-leak-visualization.md)** - Memory leak visualization and analysis

---

### Orchestration Analysis (2025-11-09)

**Context**: Discovery and fix of activity re-scheduling loop bug

- **[25-11-09-ORCHESTRATION-ANALYSIS.md](./25-11-09-ORCHESTRATION-ANALYSIS.md)** - Detailed analysis of orchestration bug
- **[25-11-09-ORCHESTRATION-FIXES.md](./25-11-09-ORCHESTRATION-FIXES.md)** - Implementation summary of fixes

---

## Report Format

All reports follow this structure:

### Header
- Date and time (UTC)
- Git SHA
- Logging level (INFO/TRACE/DEBUG)
- Report type
- Source benchmark directory

### Executive Summary
- Status (Ready/Broken/Verified)
- Key findings
- Quick metrics

### Performance Results
- Throughput, success rate, latency (P50/P95/P99)
- Comparison to other runs
- Key observations

### Analysis
- Detailed investigation
- Evidence and data
- Root cause analysis (if applicable)

### Conclusions
- Summary of findings
- Recommendations
- Next steps

---

## Report Naming Convention

Report files: `YY-MM-DD-HH-MM-REPORT_TITLE.md`

Example: `25-11-09-23-46-PRODUCTION-BASELINE.md`

**Note**: Original benchmark data directories (`var/benchmark-*/`) have been archived. All critical information from those runs is preserved in these unified reports.

---

## Key Performance Metrics

### Throughput
- **Sequential workflows**: 5-activity workflows, low concurrency
- **Parallel workflows**: 10-activity fan-out/fan-in, moderate concurrency
- **High concurrency**: 3-activity workflows, 100 concurrent
- **Sustained**: 3-activity workflows, 60+ seconds, 20 concurrent

### Latency
- **P50**: Median latency (50th percentile)
- **P95**: 95th percentile latency
- **P99**: 99th percentile latency (tail latency)

### Reliability
- **Success rate**: % of workflows completed successfully
- **Timeout rate**: % of workflows that hit 30s client timeout
- **Error rate**: % of workflows that failed with errors

---

## Performance Evolution

| Metric | Before Fix | After Fix (TRACE) | After Fix (INFO) | Improvement |
|--------|-----------|------------------|------------------|-------------|
| **Sequential** | 2.88 wf/sec | 16.28 wf/sec | **16.77 wf/sec** | **5.8x** |
| **Parallel** | 1.64 wf/sec | 17.73 wf/sec | **22.77 wf/sec** | **13.9x** |
| **High Concurrency** | 9.32 wf/sec | 52.20 wf/sec | **56.40 wf/sec** | **6.1x** |
| **Sustained** | 19.05 wf/sec | 23.92 wf/sec | **23.71 wf/sec** | **1.2x** |

---

## Production Targets

Based on the production baseline:

| Metric | Conservative | Target | Stretch |
|--------|--------------|--------|---------|
| Sequential | 15 wf/sec | 20 wf/sec | 30 wf/sec |
| Parallel | 20 wf/sec | 25 wf/sec | 35 wf/sec |
| High Concurrency | 50 wf/sec | 60 wf/sec | 80 wf/sec |
| Sustained | 20 wf/sec | 25 wf/sec | 35 wf/sec |
| Success Rate | >99% | >99.5% | >99.9% |
| P99 Latency | <5s | <3s | <2s |

**Current performance meets "Conservative" targets and approaches "Target" goals.**

---

## How to Read a Report

1. **Start with Executive Summary** - quick overview of status and key findings
2. **Review Performance Results** - understand throughput, latency, reliability
3. **Check Analysis section** - detailed investigation and evidence
4. **Read Conclusions** - summary and recommendations
5. **Cross-reference Related Reports** - links to other reports for context

---

## Related Documentation

- **Performance Guide**: `docs/performance/memory-profiling-guide.md`
- **Investigation Logs**: `docs/performance/*.md`
- **Architecture**: `docs/architecture.md`
- **Benchmarking Script**: `scripts/profiling.sh`

---

## Questions?

For questions about these reports:
- Review the source code at the reported Git SHA
- Consult the architecture documentation (`docs/architecture.md`)
- Re-run benchmarks with `scripts/profiling.sh`
- Check performance investigation logs in `docs/performance/`

---

Last updated: November 10, 2025
