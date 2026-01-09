# Bug: Stale Running Activities Not Reclaimed After Worker/Orchestrator Restart

**Date**: 2026-01-08
**Status**: Open
**Severity**: High
**Component**: Core / Orchestrator / Timeout Checker

## Summary

When a worker or the orchestrator restarts while an activity is in `running` status, the timeout checker fails to reclaim the orphaned activity. The activity remains stuck in `running` status indefinitely, even though its configured timeout has long elapsed, causing the workflow to hang.

## Symptoms

1. Activity is claimed by a worker and set to `running` status
2. Worker or orchestrator process restarts (e.g., container rebuild, crash, deployment)
3. Activity remains in `running` status with the old worker ID in `claimed_by`
4. Timeout checker logs that it is starting but never reclaims the stale activity
5. Workflow status eventually becomes `failed` (due to workflow-level timeout), but the activity queue entry remains stuck
6. The activity never completes, preventing downstream activities from running

## Steps to Reproduce

1. Start a workflow with an activity that takes several seconds (e.g., `embedding` or `llm_prompt`)
2. While the activity is `running`, restart the kruxiaflow container:
   ```bash
   docker compose restart kruxiaflow
   ```
3. Query the activity queue to observe the stuck activity:
   ```sql
   SELECT activity_key, status, timeout_duration, claimed_at,
          NOW() - claimed_at as time_since_claim, claimed_by
   FROM activity_queue
   WHERE status = 'running';
   ```
4. Observe that `time_since_claim` exceeds `timeout_duration` but status remains `running`
5. Check logs - timeout checker logs startup but no reclamation:
   ```
   Timeout checker starting (check_interval=30s, timeout=300s)
   ```

## Observed Behavior

From the investigation on 2026-01-08:

```sql
-- Activity stuck for 9+ minutes with 60s timeout
SELECT activity_key, status, timeout_duration, claimed_at,
       NOW() - claimed_at as time_since_claim
FROM activity_queue WHERE status = 'running';

    activity_key     | status  | timeout_duration |          claimed_at           | time_since_claim
---------------------+---------+------------------+-------------------------------+------------------
 generate_embeddings | running | 00:01:00         | 2026-01-08 15:58:35.173974+00 | 00:09:06.382949
```

The activity had:
- `timeout_duration`: 1 minute
- `time_since_claim`: 9+ minutes
- `claimed_by`: Worker ID from previous orchestrator instance that no longer exists

The timeout checker should have reclaimed this activity within ~90 seconds (30s check interval + 60s timeout), but it never did.

## Expected Behavior

The timeout checker should:
1. Periodically scan for activities in `running` status
2. Check if `NOW() - claimed_at > timeout_duration`
3. If timeout exceeded:
   - Set status back to `pending`
   - Clear `claimed_by` and `claimed_at`
   - Increment `retry_count`
   - If `retry_count >= max_retries`, mark as `failed`
4. Emit an event so the orchestrator can re-evaluate workflow state

## Workaround

Manually reset the stuck activity:

```sql
UPDATE activity_queue
SET status = 'pending',
    claimed_by = NULL,
    claimed_at = NULL,
    retry_count = retry_count + 1
WHERE id = '<activity_id>' AND status = 'running';
```

## Root Cause Analysis

Potential causes to investigate:

1. **Timeout checker not running**: The timeout checker task may not be spawned or may have panicked silently after startup

2. **Worker ID filtering**: The timeout checker may only be checking activities claimed by the *current* worker instance, missing activities claimed by previous instances

3. **Query predicate issue**: The SQL query to find timed-out activities may have incorrect predicates (e.g., checking wrong timestamp column)

4. **Race condition on restart**: When orchestrator restarts, there may be a timing window where the old consumer ID is still considered valid

5. **Event emission**: Timeout checker may be updating the activity but failing to emit the `ActivityFailed` or `ActivityTimedOut` event needed for the orchestrator to react

## Investigation Points

1. Check `core/src/orchestrator/orchestrator.rs` - timeout checker implementation
2. Check `core/src/queue/postgres_queue.rs` - activity reclamation SQL
3. Verify the timeout checker query includes all `running` activities, not just those claimed by current worker
4. Add logging to timeout checker to show activities being scanned and decisions made

## Impact

- **Workflow reliability**: Durable execution guarantee is broken - workflows don't survive restarts
- **Production deployments**: Any deployment that restarts kruxiaflow can orphan running activities
- **Manual intervention required**: Stuck workflows require manual database updates to recover

## Related

- This may be related to the distinction between activity-level timeout vs workflow-level timeout
- The workflow eventually fails due to workflow timeout (300s), but the activity remains stuck

## Files to Investigate

- `core/src/orchestrator/orchestrator.rs` (timeout checker task)
- `core/src/queue/postgres_queue.rs` (activity queue operations)
- `core/src/orchestrator/mod.rs` (orchestrator initialization)
