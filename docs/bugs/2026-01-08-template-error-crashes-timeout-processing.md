# Bug: Template Evaluation Errors Crash Timeout Event Processing

**Reported:** 2026-01-08
**Status:** Fix implemented, needs testing
**Severity:** High - causes workflows to get permanently stuck

## Summary

When a workflow times out, the `WorkflowFailed` event processing crashes due to template evaluation errors, causing the workflow to remain stuck in `running` state indefinitely.

## Symptoms

1. Workflows get stuck in `running` state beyond the timeout threshold
2. Timeout checker logs "Timing out workflow X" repeatedly every 30 seconds
3. Only ONE `WorkflowFailed` event exists per stuck workflow (subsequent events silently discarded)
4. Error in logs: `Template evaluation error: undefined value`

## Root Cause

When processing a `WorkflowFailed` timeout event:

1. `apply_event_to_state()` correctly sets `state.status = Failed`
2. `find_ready_activities()` is called to determine next activities
3. This evaluates template conditions like `{{activity.result.rows | length > 0}}`
4. **BUG:** Activities that never completed don't have `.result` - template evaluation throws "undefined value" error
5. Error propagates up, `process_workflow_event()` returns `Err(...)`
6. Transaction rolls back - workflow status never saved as `failed`
7. Consumer position updates after max retries (poison message), but workflow stays `running`

### Contributing Factor: Idempotent Event Insert

The `ON CONFLICT DO NOTHING` clause in `publish()` prevents duplicate events:

```sql
ON CONFLICT (workflow_id, event_type, activity_key, iteration) DO NOTHING
```

For timeout `WorkflowFailed` events, all fields are identical across attempts, so subsequent timeout events are silently discarded. This prevents recovery after the first event fails to process.

## Reproduction

1. Create a workflow with conditional dependencies that reference activity outputs:
   ```yaml
   - key: activity_b
     depends_on:
       - activity_key: activity_a
         condition: "{{activity_a.result.rows | length > 0}}"
   ```

2. Start workflow but ensure `activity_a` never completes (e.g., worker not running)

3. Wait for workflow timeout (default 300s)

4. Observe: Workflow stays `running`, timeout warnings repeat every 30s

## Fix

Modified `orchestrator.rs` to catch `TemplateFailed` errors and fail the workflow gracefully:

### 1. In `find_ready_activities()` call (~line 1089):

```rust
match find_ready_activities(&definition, &state) {
    Ok(activities) => activities,
    Err(super::OrchestratorError::TemplateFailed(msg)) => {
        // Mark all non-terminal activities as failed
        for activity_state in state.activities.values_mut() {
            if !matches!(activity_state.status,
                WorkflowActivityStatus::Completed |
                WorkflowActivityStatus::Failed |
                WorkflowActivityStatus::Skipped
            ) {
                activity_state.status = WorkflowActivityStatus::Failed;
                activity_state.set_error(format!("Template evaluation error: {}", msg));
            }
        }
        state.status = WorkflowStatus::Failed;
        save_materialized_state(&mut tx, event.workflow_id, &state).await?;
        tx.commit().await?;
        return Ok(());
    }
    Err(e) => return Err(e),
}
```

### 2. Same pattern for `find_skipped_activities()` (~line 1340)

### 3. In parameter resolution loop (~line 1180):

Mark individual activity as failed and continue with others instead of returning error.

## Files Changed

- `core/src/orchestrator/orchestrator.rs`

## Testing Required

1. **Unit test:** Process `WorkflowFailed` event when activities have unresolved template dependencies
2. **Integration test:** Workflow timeout with incomplete activities - verify status becomes `failed`
3. **Regression test:** Normal workflow completion still works
4. **Edge case:** Template error in single activity during parameter resolution - other activities should still schedule

## Related Issues

- Workflow definitions should use `default()` filter for optional activity references:
  ```yaml
  condition: "{{(activity.result.rows | default([])) | length > 0}}"
  ```

## Workaround (until fix deployed)

Manually update stuck workflows:
```sql
UPDATE workflows
SET status = 'failed', updated_at = NOW()
WHERE status = 'running'
  AND created_at < NOW() - INTERVAL '1 hour';
```
