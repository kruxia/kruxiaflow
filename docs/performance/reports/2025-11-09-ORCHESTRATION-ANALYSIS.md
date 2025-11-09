# Orchestration Bug Analysis - test_parallel_workflow_load

**Date**: November 9, 2025
**Test**: `test_parallel_workflow_load` (parallel_bench_10, 50 workflows, 1 worker)

## Test Results Summary

```
Total Workflows:     50
Successful:          49 (98%)
Failed:              1 (2%)
Duration:            31.77s
Throughput:          1.57 workflows/sec (Expected: >= 50 wf/sec)

Latency:
  P50:               530 ms
  P95:               1007 ms
  P99:               30027 ms (Expected: <= 200ms)
```

**Critical Issues**:
1. ❌ **One workflow timed out** after 30 seconds (never completed)
2. ❌ **Throughput 32x slower** than expected (1.57 vs 50 wf/sec)
3. ❌ **P99 latency 150x slower** than expected (30s vs 200ms)

---

## Workflow Structure Analysis

### parallel_bench_10 Definition

```
start → [parallel_0, parallel_1, ..., parallel_9] → end
  |              (10 activities)                    |
  ↓                                                 ↓
12 total activities (start + 10 parallel + end)
```

**Activity Relationships** (from `benchmark/src/scenarios.rs`):
1. **start** activity:
   - `following: [parallel_0, ..., parallel_9]` (fan-out to 10)

2. **Each parallel activity** (parallel_0 through parallel_9):
   - `preceding: [start]`
   - `following: [end]`

3. **end** activity:
   - `preceding: [parallel_0, ..., parallel_9]` (fan-in from 10)

**Key Constraint**: Running with **ONLY 1 WORKER**, so parallel activities execute sequentially

---

## Bug #1: Duplicate Dependency Collection 🐛

**Location**: `core/src/orchestrator/dependency_evaluator.rs:80-106`

```rust
fn get_preceding_activities(
    activity: &ActivityDefinition,
    definition: &WorkflowDefinition,
) -> Vec<(String, Option<Vec<String>>)> {
    let mut preceding = Vec::new();

    // Check explicit `preceding` list
    if let Some(preceding_list) = &activity.preceding {
        for item in preceding_list {
            preceding.push((item.activity_key.clone(), item.conditions.clone()));
        }
    }

    // Check if other activities list this one in `following`
    for other_activity in &definition.activities {
        if let Some(following_list) = &other_activity.following {
            for item in following_list {
                if item.activity_key == activity.key {
                    preceding.push((other_activity.key.clone(), item.conditions.clone()));
                }
            }
        }
    }

    preceding
}
```

**Issue**: This function collects BOTH:
- Explicit `preceding` relationships
- Implicit `preceding` inferred from `following` relationships

**Impact on `end` activity**:
- Explicit `preceding`: [parallel_0, ..., parallel_9] (10 items)
- Implicit (from `following`): [parallel_0, ..., parallel_9] (10 items)
- **Result**: 20 dependencies instead of 10 (each appears twice!)

**Symptoms**:
- Performance degradation (checking each dependency twice)
- Potential logic errors if dependency checks have side effects

**Fix**: Deduplicate the preceding list OR only use one source (prefer explicit `preceding`)

---

## Bug #2: Possible Workflow Hang (Hypothesis)

**Symptom**: 1 workflow timed out after 30 seconds

**Possible Root Causes**:

### Hypothesis A: Activity Never Scheduled
If the orchestrator fails to detect that an activity is ready:
- Activity remains in `NotScheduled` state
- Workflow never completes
- Client times out after 30s

**Check**: Look for activities stuck in `NotScheduled` state

### Hypothesis B: Activity Scheduled But Not Claimed
If an activity is scheduled but the worker never claims it:
- Activity sits in queue forever
- Workflow hangs

**Check**: Query `activity_queue` table for orphaned activities

### Hypothesis C: Activity Claimed But Never Completed
If a worker claims an activity but crashes/fails without reporting:
- Activity stuck in `Pending` state
- Orchestrator waiting for completion event that never comes

**Check**: Look for activities stuck in `Pending` state

### Hypothesis D: Completion Event Lost
If an ActivityCompleted event is published but orchestrator misses it:
- Activity is actually done
- Orchestrator still waiting
- Workflow hangs

**Check**: Query `workflow_events` table vs `workflow` state for mismatches

---

## Performance Analysis: Why So Slow?

### Expected Performance (with 1 worker)
```
Workflow: start → [10 parallel] → end
Activities per workflow: 12
Worker speed: ~10ms per activity (echo is fast)

Sequential execution time: 12 * 10ms = 120ms per workflow
Expected throughput: 1000ms / 120ms = ~8 workflows/sec
```

### Actual Performance
```
Throughput: 1.57 workflows/sec
Time per workflow: 636ms average (31.77s / 50 workflows)
```

**Analysis**: We're getting ~8x slower than theoretical minimum!

### Possible Bottlenecks

1. **Event Polling Latency**
   - Orchestrator polls every 10ms minimum
   - Each activity completion requires orchestrator to poll, evaluate, schedule next
   - 12 activities × 10ms polling = 120ms overhead per workflow

2. **Database Contention**
   - Advisory locks on workflow state
   - Multiple transactions per activity
   - SERIALIZABLE isolation level?

3. **Worker Polling Latency**
   - Worker polls for activities with some interval
   - Delay between activity becoming available and worker claiming it

4. **Event Processing Delay**
   - Time between worker publishing ActivityCompleted and orchestrator processing it

---

## Debugging Steps

### 1. Check for Stuck Workflows/Activities

```sql
-- Find workflows that never completed
SELECT w.id, w.name, w.status, w.created_at,
       EXTRACT(EPOCH FROM (NOW() - w.created_at)) as age_seconds
FROM workflows w
WHERE w.status = 'running'
  AND w.created_at < NOW() - INTERVAL '30 seconds'
ORDER BY w.created_at;

-- Find activities stuck in NotScheduled
SELECT w.id as workflow_id, w.name,
       jsonb_object_keys(w.state_data->'activities') as activity_key,
       w.state_data->'activities'->jsonb_object_keys(w.state_data->'activities')->'status' as status
FROM workflows w
WHERE w.state_data->'activities' ? activity_key
  AND w.state_data->'activities'->activity_key->>'status' = 'not_scheduled'
  AND w.created_at < NOW() - INTERVAL '30 seconds';

-- Find activities stuck in Pending (claimed but never completed)
SELECT w.id as workflow_id,
       jsonb_object_keys(w.state_data->'activities') as activity_key,
       w.state_data->'activities'->jsonb_object_keys(w.state_data->'activities') as activity_state
FROM workflows w
WHERE w.state_data->'activities' @>
      jsonb_build_object(jsonb_object_keys(w.state_data->'activities'),
                         jsonb_build_object('status', 'pending'))
  AND w.created_at < NOW() - INTERVAL '30 seconds';

-- Check activity queue for orphaned tasks
SELECT aq.*, w.name, w.status
FROM activity_queue aq
LEFT JOIN workflows w ON aq.workflow_id = w.id
WHERE aq.claimed_by IS NULL
  AND aq.created_at < NOW() - INTERVAL '1 minute'
ORDER BY aq.created_at;
```

### 2. Check Event Flow

```sql
-- Count events per workflow
SELECT workflow_id, event_type, COUNT(*) as count
FROM workflow_events
WHERE workflow_id = '019a6be4-99c5-74b0-a7cc-62e575444344'  -- Replace with failed workflow ID
GROUP BY workflow_id, event_type
ORDER BY event_type;

-- Expected counts for parallel_bench_10:
-- WorkflowCreated: 1
-- ActivityScheduled: 12  (start + 10 parallel + end)
-- ActivityCompleted: 12
-- WorkflowCompleted: 1

-- Verify event sequence
SELECT id, event_type, activity_key, created_at
FROM workflow_events
WHERE workflow_id = '019a6be4-99c5-74b0-a7cc-62e575444344'
ORDER BY created_at;
```

### 3. Examine Orchestrator Behavior

Add debug logging to see:
- When activities are detected as ready
- When activities are scheduled
- Duration of dependency evaluation

### 4. Check Worker Behavior

```sql
-- Worker activity claim rate
SELECT
    DATE_TRUNC('second', claimed_at) as second,
    COUNT(*) as activities_claimed
FROM activity_queue
WHERE claimed_at IS NOT NULL
  AND claimed_at > NOW() - INTERVAL '1 minute'
GROUP BY second
ORDER BY second;
```

---

## Recommended Fixes

### Priority 1: Fix Duplicate Dependencies

**In `dependency_evaluator.rs`**:

```rust
fn get_preceding_activities(
    activity: &ActivityDefinition,
    definition: &WorkflowDefinition,
) -> Vec<(String, Option<Vec<String>>)> {
    use std::collections::HashSet;

    let mut preceding = Vec::new();
    let mut seen = HashSet::new();

    // Check explicit `preceding` list
    if let Some(preceding_list) = &activity.preceding {
        for item in preceding_list {
            if seen.insert(item.activity_key.clone()) {
                preceding.push((item.activity_key.clone(), item.conditions.clone()));
            }
        }
    }

    // Check if other activities list this one in `following`
    for other_activity in &definition.activities {
        if let Some(following_list) = &other_activity.following {
            for item in following_list {
                if item.activity_key == activity.key && seen.insert(other_activity.key.clone()) {
                    preceding.push((other_activity.key.clone(), item.conditions.clone()));
                }
            }
        }
    }

    preceding
}
```

### Priority 2: Add Workflow Timeout Handling

Detect and fail workflows that are stuck:
- Monitor workflows in `running` state for > 5 minutes
- Automatically transition to `failed` with timeout reason
- Prevent indefinite hangs

### Priority 3: Improve Observability

Add metrics/logs for:
- Time from ActivityCompleted to next activity being scheduled
- Number of activities in each state per workflow
- Orchestrator poll latency
- Worker poll latency

---

## Next Steps

1. **Query the database** for the failed workflow (019a6be4-99c5-74b0-a7cc-62e575444344)
2. **Check event sequence** to see where it got stuck
3. **Examine activity states** to identify which activity never completed
4. **Apply the deduplication fix** and re-run the benchmark
5. **Profile the orchestrator** to identify latency sources

---

## Questions to Answer

1. Which activity is the failed workflow stuck on?
2. Is the activity in NotScheduled, Pending, or Completed state?
3. Are there corresponding events in workflow_events?
4. Is the activity still in the activity_queue?
5. What is the average time between ActivityCompleted and the next ActivityScheduled?
