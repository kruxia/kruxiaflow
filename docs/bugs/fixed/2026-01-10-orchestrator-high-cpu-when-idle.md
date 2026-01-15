# Bug: Orchestrator High CPU When Idle

**Date**: 2026-01-10
**Status**: Resolved
**Resolved**: 2026-01-14
**Severity**: Medium
**Component**: Orchestrator

## Symptoms

The orchestrator runs at 100% CPU even when the system is idle (no active workflows):

```
NAME                      CPU %     MEM USAGE / LIMIT     MEM %
researcher-kruxiaflow-1   99.50%    23.05MiB / 256MiB     9.00%
```

## Root Cause

Multiple factors combine to cause excessive polling:

### 1. Aggressive Minimum Poll Interval

**File**: `core/src/orchestrator/config.rs:22`

```rust
poll_interval_min: Duration::from_millis(10),  // 100 polls/sec
```

When any events are found, backoff resets to 10ms, resulting in up to 100 database polls per second.

### 2. Backoff Resets on Any Event Processing

**File**: `core/src/orchestrator/orchestrator.rs:165`

```rust
// Got events - reset backoff
backoff.reset();
```

This line executes after processing events, even if all events failed. Combined with the stuck workflow timeout issue (see `2026-01-10-stuck-workflows-not-resolved-on-timeout.md`), this creates a feedback loop:

1. Timeout checker publishes 3 WorkflowFailed events every 30 seconds
2. Orchestrator polls, gets these events
3. Events fail to process (due to related bug)
4. Backoff resets to 10ms anyway
5. System polls rapidly for the next 30 seconds until timeout checker runs again

### 3. No CPU-Aware Throttling

The orchestrator has no mechanism to reduce polling when:
- CPU usage is high
- No meaningful work is being done
- Events are repeatedly failing

## Impact

- Excessive CPU usage (100%) when system should be idle
- Increased database load from frequent polling
- Higher infrastructure costs
- Reduced capacity for actual workflow processing

## Proposed Fixes

### Fix 1: Increase Minimum Poll Interval (Quick)

Change the minimum poll interval from 10ms to 50-100ms:

```rust
poll_interval_min: Duration::from_millis(50),  // 20 polls/sec max
```

Trade-off: Slightly higher latency for new workflows (~25ms average vs ~5ms).

### Fix 2: Only Reset Backoff on Successful Processing

**File**: `core/src/orchestrator/orchestrator.rs`

Move `backoff.reset()` inside the success path:

```rust
for event in &events {
    if let Err(e) = process_workflow_event(...) {
        // ... error handling ...
    } else {
        // Success - clear failure count, update position, and reset backoff
        event_failures.remove(&event.id);
        event_source.update_position(CONSUMER_ID, event.id).await?;
        backoff.reset();  // Only reset on successful processing
    }
}

// Remove the unconditional backoff.reset() at line 165
```

### Fix 3: Add Backoff for Failed Events (Recommended)

When events fail to process, apply backoff before retrying:

```rust
} else {
    // Failed - increase backoff for this event type
    backoff.increase();
    continue;
}
```

### Fix 4: Implement Idle Detection

Track whether the system is doing meaningful work:

```rust
let mut successful_events = 0;
for event in &events {
    if process_workflow_event(...).is_ok() {
        successful_events += 1;
    }
}

if successful_events > 0 {
    backoff.reset();
} else {
    backoff.increase();
}
```

## Related Issues

- `2026-01-10-stuck-workflows-not-resolved-on-timeout.md` - Root cause of the failing events

## Workaround

1. Fix the stuck workflows immediately:
   ```sql
   UPDATE workflows
   SET status = 'failed', updated_at = NOW()
   WHERE status = 'running'
     AND created_at < NOW() - INTERVAL '5 minutes';
   ```

2. Increase poll interval via environment variable (if supported) or rebuild with higher minimum.

## Resolution

### Changes Applied (2026-01-14)

1. **Increased minimum poll interval from 10ms to 50ms** (`core/src/orchestrator/config.rs`)
   - Reduces maximum poll rate from 100/sec to 20/sec
   - Trade-off: ~25ms average latency increase (acceptable for most workflows)

2. **Increased maximum poll interval from 500ms to 1000ms**
   - Reduces CPU usage when idle by allowing longer sleeps

3. **Made all polling parameters configurable via environment variables:**
   - `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS` (default: 50)
   - `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS` (default: 1000)
   - `KRUXIAFLOW_ORCHESTRATOR_BACKOFF_MULTIPLIER` (default: 1.5)
   - `KRUXIAFLOW_ORCHESTRATOR_WORKFLOW_TIMEOUT_SECS` (default: 300)
   - `KRUXIAFLOW_ORCHESTRATOR_TIMEOUT_CHECK_INTERVAL_SECS` (default: 30)

4. **Added CLI parameters for orchestrator command:**
   - `--poll-interval-min` / `--poll-interval-max`
   - `--backoff-multiplier`

5. **Updated serve command to use `OrchestratorConfig::from_env()`**

### Worker Polling

Worker polling is configurable via:
- `KRUXIAFLOW_WORKER_POLL_INTERVAL_MS` (default: 100ms)
- `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES` (default: 1)

### Queue Configuration

Additional queue configuration via environment variables:
- `KRUXIAFLOW_QUEUE_POLL_INTERVAL` (default: 100ms)
- `KRUXIAFLOW_QUEUE_BATCH_SIZE` (default: 100)
- `KRUXIAFLOW_QUEUE_DEFAULT_TIMEOUT` (default: 60s)
- `KRUXIAFLOW_QUEUE_DEFAULT_MAX_RETRIES` (default: 3)

## Testing

1. Start orchestrator with no active workflows
2. Monitor CPU usage - should be near 0%
3. Create and complete a workflow
4. Monitor CPU usage - should return to near 0% after completion
