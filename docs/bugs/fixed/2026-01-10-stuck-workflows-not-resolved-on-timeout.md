# Bug: Stuck Workflows Not Resolved on Timeout

**Date**: 2026-01-10
**Status**: Fixed
**Fixed Date**: 2026-01-10
**Severity**: High
**Component**: Orchestrator

## Symptoms

Workflows that are detected as "stuck" (running longer than the configured timeout) are repeatedly timed out but never resolve to failed status:

```
61.002022028s  WARN kruxiaflow_core::orchestrator::orchestrator: Found 3 stuck workflows (running > 300s), timing out
61.002035736s  WARN kruxiaflow_core::orchestrator::orchestrator: Timing out workflow 019ba715-bc4f-7030-9814-af430cfc71ba (extract_citations)
...
92.091015167s  WARN kruxiaflow_core::orchestrator::orchestrator: Found 3 stuck workflows (running > 300s), timing out
92.091028417s  WARN kruxiaflow_core::orchestrator::orchestrator: Timing out workflow 019ba715-bc4f-7030-9814-af430cfc71ba (extract_citations)
```

The same workflows are found stuck repeatedly because their status remains "running" in the database despite `WorkflowFailed` events being published.

## Root Cause

In `orchestrator.rs`, when a `WorkflowFailed` event (from the timeout checker) is processed:

1. **Line 1015**: `apply_event_to_state(&mut state, event)` correctly sets `state.status = WorkflowStatus::Failed`
2. **Lines 1056-1340**: The code continues to evaluate and schedule activities
3. **Line 1435**: `save_materialized_state` saves the Failed status to the database

**The Problem**: Between steps 2 and 3, there are multiple operations that can fail with early returns:

| Line   | Operation                      | Can Error |
|--------|--------------------------------|-----------|
| 1086   | `find_ready_activities`        | Yes (?)   |
| 1137   | Parameter serialization        | Yes (?)   |
| 1202   | Template resolution            | Yes (Err) |
| 1220   | Budget enrichment              | Yes (?)   |
| 1241   | Scheduled time computation     | Yes (?)   |
| 1269   | `activity_queue.schedule`      | Yes (?)   |
| 1339   | Publish ActivityScheduled      | Yes (?)   |
| 1356   | `find_skipped_activities`      | Yes (?)   |
| 1410   | Publish completion event       | Yes (?)   |

If any of these operations fail, the function returns early with an error, and the transaction is rolled back. The `WorkflowFailed` status is never persisted.

**Additional Issues**:

1. `find_ready_activities` does not check the workflow status - it can still return "ready" activities for a Failed workflow
2. Activities should never be scheduled for a workflow in a terminal state (Failed/Completed)
3. The event is retried on the next poll, but encounters the same error, creating an infinite loop

## Expected Behavior

When a `WorkflowFailed` timeout event is processed:
1. The workflow status should be set to Failed
2. The status should be persisted immediately
3. No activities should be scheduled
4. The workflow should not appear in subsequent "stuck workflow" queries

## Proposed Fix

Add an early exit after `apply_event_to_state` for terminal workflow states:

```rust
// After line 1015: apply_event_to_state(&mut state, event)?;

// If workflow transitioned to a terminal state, save immediately and return
// This handles timeout events and prevents scheduling on failed workflows
let is_now_terminal = matches!(
    state.status,
    WorkflowStatus::Completed | WorkflowStatus::Failed
);

if is_now_terminal && event.event_type == WorkflowEventType::WorkflowFailed {
    tracing::info!(
        workflow_id = %event.workflow_id,
        workflow_name = %state.definition_name,
        "Workflow marked as failed (timeout or explicit failure)"
    );

    save_materialized_state(&mut tx, event.workflow_id, &state).await?;
    tx.commit().await?;
    return Ok(());
}
```

**Alternative (more comprehensive)**: Check for terminal state before any scheduling logic:

```rust
// Before line 1056 (find_ready_activities)
let is_terminal = matches!(
    state.status,
    WorkflowStatus::Completed | WorkflowStatus::Failed
);

if is_terminal {
    // Skip activity scheduling entirely, just save state
    save_materialized_state(&mut tx, event.workflow_id, &state).await?;
    tx.commit().await?;
    return Ok(());
}
```

## Impact

- Workflows that timeout are never cleaned up
- Resources remain allocated to stuck workflows
- User-facing workflow status shows "running" indefinitely
- Timeout checking consumes resources repeatedly for the same workflows

## Related Files

- `core/src/orchestrator/orchestrator.rs:799-1463` - `process_workflow_event`
- `core/src/orchestrator/orchestrator.rs:1891-1951` - `check_and_timeout_stuck_workflows`
- `core/src/orchestrator/workflow_state.rs:382-384` - WorkflowFailed state transition
- `core/src/orchestrator/dependency_evaluator.rs:17-31` - `find_ready_activities`

## Testing

1. Create a workflow with activities that will exceed timeout
2. Verify that after timeout, workflow status becomes "failed"
3. Verify that the workflow is not detected as "stuck" on subsequent checks
4. Verify that activities are not scheduled after timeout

## Workaround

Manually update the workflow status in the database:

```sql
UPDATE workflows
SET status = 'failed', updated_at = NOW()
WHERE id IN (
    '019ba715-bc4f-7030-9814-af430cfc71ba',
    '019ba716-a0c3-7310-8807-fda75cc30fdb',
    '019ba91a-3d97-7182-bbae-093e85096383'
);
```

## Fix

Implemented the **alternative (more comprehensive)** fix: Check for terminal state before any scheduling logic.

### Change in `core/src/orchestrator/orchestrator.rs`

Added early exit check in `process_workflow_event` after `apply_event_to_state` and cost recording:

```rust
// 3.6. Early exit for terminal workflow states
// If workflow is already Completed or Failed, skip all scheduling logic
// This prevents errors in scheduling from blocking workflow state persistence
// and avoids scheduling activities for workflows that should not run anymore
let is_terminal = matches!(
    state.status,
    WorkflowStatus::Completed | WorkflowStatus::Failed
);

if is_terminal {
    tracing::info!(
        workflow_id = %event.workflow_id,
        workflow_name = %state.definition_name,
        status = %state.status,
        "Workflow in terminal state, skipping activity scheduling"
    );

    // Save the terminal state and return early
    save_materialized_state(&mut tx, event.workflow_id, &state).await?;
    tx.commit().await?;
    return Ok(());
}
```

### Behavior After Fix

1. `apply_event_to_state` sets `state.status = Failed` (or `Completed`)
2. **NEW**: Check if workflow is in terminal state
3. **NEW**: If terminal, save state immediately and return - skip all scheduling logic
4. No template evaluation, dependency checking, or activity scheduling occurs
5. Transaction commits successfully with the terminal status persisted

### Tests Added (`core/tests/orchestrator_integration_tests.rs`)

- `test_workflow_failed_event_persists_status_immediately` - Verifies Failed status is persisted
- `test_failed_workflow_does_not_schedule_activities` - Verifies no activities scheduled after failure
- `test_events_on_already_failed_workflow_handled_gracefully` - Verifies late events don't cause errors
- `test_completed_workflow_early_exit` - Verifies Completed workflows also benefit from early exit
