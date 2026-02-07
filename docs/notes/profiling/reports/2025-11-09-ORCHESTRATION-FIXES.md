# Orchestration Fixes - Implementation Summary

**Date**: November 9, 2025
**Branch**: epic-2-benchmark-w-shutdown

## Changes Implemented

### ✅ Priority 1: Fix Duplicate Dependency Collection

**File**: `core/src/orchestrator/dependency_evaluator.rs:80-115`

**Problem**:
- `get_preceding_activities()` was collecting dependencies from BOTH `depends_on` and `dependency_of` lists
- For parallel_bench_10's `end` activity: 20 dependencies instead of 10 (each appeared twice)
- Caused performance degradation and wasted CPU cycles

**Solution**:
```rust
// Use HashMap to deduplicate dependencies
let mut preceding_map: HashMap<String, Option<Vec<String>>> = HashMap::new();

// Explicit `depends_on` list takes priority
if let Some(preceding_list) = &activity.depends_on {
    for item in preceding_list {
        preceding_map.insert(item.activity_key.clone(), item.conditions.clone());
    }
}

// Only add from `dependency_of` if not already present
for other_activity in &definition.activities {
    if let Some(following_list) = &other_activity.dependency_of {
        for item in following_list {
            if item.activity_key == activity.key {
                preceding_map
                    .entry(other_activity.key.clone())
                    .or_insert_with(|| item.conditions.clone());
            }
        }
    }
}
```

**Expected Impact**:
- Reduce dependency checking overhead by 50%
- More accurate dependency evaluation
- Better performance for workflows with fan-in patterns

---

### ✅ Priority 2: Add Workflow Timeout Handling

**Files**:
- `core/src/orchestrator/config.rs:6-47` - Added timeout configuration
- `core/src/orchestrator/orchestrator.rs:295-382` - Implemented timeout checker

**Problem**:
- Workflows could hang indefinitely if activities get stuck
- No automatic cleanup for stuck workflows
- Difficult to debug hanging workflows

**Solution**:

1. **Configuration** (default: 5 minutes timeout, checked every 30 seconds):
```rust
pub struct OrchestratorConfig {
    pub workflow_timeout: Duration,           // 5 minutes default
    pub timeout_check_interval: Duration,     // 30 seconds default
    // ... other fields
}
```

2. **Background Timeout Checker Task**:
```rust
async fn timeout_checker_task(...) {
    loop {
        tokio::time::sleep(config.timeout_check_interval).await;
        check_and_timeout_stuck_workflows(&config, &event_source).await;
    }
}
```

3. **Stuck Workflow Detection**:
```rust
// Query for workflows running > timeout duration
SELECT id, name
FROM workflows
WHERE status = 'running'
  AND created_at < NOW() - make_interval(secs => $timeout_secs)
LIMIT 100
```

4. **Automatic Timeout**:
- Publishes `WorkflowFailed` event with timeout reason
- Orchestrator processes event normally
- Workflow transitions to `Failed` status

**Expected Impact**:
- Prevent indefinite workflow hangs
- Automatic cleanup of stuck workflows
- Better visibility into timeout issues

**Configuration Options**:
```rust
// Customize timeout settings
let config = OrchestratorConfig::new(pool)
    .with_workflow_timeout(Duration::from_secs(600))  // 10 minutes
    .with_timeout_check_interval(Duration::from_secs(60));  // Check every minute
```

---

### ✅ Priority 3: Improve Orchestrator Observability

**Files**:
- `core/src/orchestrator/orchestrator.rs` - Added detailed logging
- `core/src/orchestrator/dependency_evaluator.rs` - Added trace logging

**Problem**:
- Hard to debug why activities aren't being scheduled
- No visibility into dependency evaluation
- Difficult to identify performance bottlenecks

**Solution**:

1. **Activity State Distribution Logging** (line 187-198):
```rust
// Log activity state counts for each workflow event
tracing::debug!(
    "Activity state distribution: not_scheduled={}, pending={}, completed={}, failed={}",
    not_scheduled_count, pending_count, completed_count, failed_count
);
```

2. **Activity Scheduling Details** (line 211-220):
```rust
// Log which activities are being scheduled
tracing::info!(
    "Scheduling {} activities for workflow {}: [{}]",
    count,
    workflow_id,
    activity_keys.join(", ")
);
```

3. **Missing Schedule Detection** (line 264-269):
```rust
// Log when no activities are ready (helps identify stalls)
tracing::debug!(
    "No activities ready to schedule for workflow {} (event: {:?})",
    workflow_id,
    event_type
);
```

4. **Dependency Evaluation Tracing** (dependency_evaluator.rs:43-115):
```rust
// Trace dependency checking at trace level
tracing::trace!(
    "Checking {} dependencies for activity {}: [{}]",
    count, activity_key, dependency_list
);

// Log why activity isn't ready
tracing::trace!(
    "Activity {} not ready: dependency {} is in state {:?}",
    activity_key, dep_key, dep_status
);

// Confirm when activity is ready
tracing::trace!(
    "Activity {} is ready: all {} dependencies satisfied",
    activity_key, dep_count
);
```

**Expected Impact**:
- Easy debugging with `RUST_LOG=kruxiaflow=debug` or `trace`
- Clear visibility into orchestration flow
- Identify bottlenecks and stuck activities quickly

**Log Levels**:
- `info` - Activity scheduling, workflow completion
- `debug` - State distribution, ready activity counts
- `trace` - Detailed dependency evaluation (use with `--level trace`)

---

## Testing the Fixes

### Run Benchmark with Debug Logging

```bash
# With info-level logging (recommended for benchmarks)
./scripts/profiling.sh --test test_parallel_workflow_load --level info

# With debug-level logging (more detail, some overhead)
./scripts/profiling.sh --test test_parallel_workflow_load --level debug

# With trace-level logging (very detailed, significant overhead)
./scripts/profiling.sh --test test_parallel_workflow_load --level trace
```

### Expected Improvements

1. **No More Duplicate Dependencies**:
   - `end` activity now has 10 dependencies instead of 20
   - Faster dependency evaluation
   - More accurate readiness checks

2. **Automatic Timeout Handling**:
   - Stuck workflows fail after 5 minutes (configurable)
   - No more 30-second client timeouts without explanation
   - Clear timeout reason in workflow status

3. **Better Debugging**:
   - See exactly which activities are ready at each step
   - Understand why activities aren't being scheduled
   - Track state transitions for each activity

### What to Look For

1. **Performance**:
   - Throughput should improve (closer to 50 wf/sec target)
   - P99 latency should decrease (closer to 200ms target)
   - Fewer workflows timing out

2. **Logs** (with debug level):
   ```
   Orchestrator starting with consumer_id=orchestrator, workflow_timeout=300s
   Timeout checker starting (check_interval=30s, timeout=300s)

   Scheduling 10 activities for workflow <id>: [parallel_0, parallel_1, ...]
   Activity state distribution: not_scheduled=2, pending=0, completed=10, failed=0

   Found 1 ready activities for workflow <id>
   Scheduling 1 activities for workflow <id>: [end]
   ```

3. **Timeout Behavior**:
   - If a workflow gets stuck, after 5 minutes:
   ```
   Found 1 stuck workflows (running > 300s), timing out
   Timing out workflow <id> (parallel_bench_10)
   ```

---

## Rollback Instructions

If these changes cause issues:

1. **Revert all changes**:
```bash
git checkout HEAD~1 -- core/src/orchestrator/
```

2. **Revert specific fix**:
```bash
# Revert only dependency deduplication
git checkout HEAD~1 -- core/src/orchestrator/dependency_evaluator.rs

# Revert only timeout handling
git checkout HEAD~1 -- core/src/orchestrator/config.rs core/src/orchestrator/orchestrator.rs
```

---

## Future Improvements

1. **Metrics**:
   - Add Prometheus metrics for orchestrator latency
   - Track time between ActivityCompleted → ActivityScheduled
   - Monitor timeout rate

2. **Smarter Timeouts**:
   - Per-workflow timeout configuration
   - Activity-level timeouts
   - Exponential backoff for retries

3. **Performance**:
   - Batch event processing
   - Parallel dependency evaluation
   - Cache workflow definitions

---

## Summary

All three priorities have been successfully implemented:

✅ **Fixed duplicate dependency bug** - 50% reduction in dependency checks
✅ **Added automatic timeout handling** - No more indefinite hangs
✅ **Improved observability** - Easy debugging with detailed logs

The changes are backward compatible and can be configured via environment variables or code.

**Next Step**: Run benchmarks to validate improvements!
