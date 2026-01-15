# Bug: Activity Polling Uses Multiple Sequential Queries Instead of Single Batched Query

**Date**: 2026-01-15
**Status**: Resolved
**Severity**: High (Performance)
**Component**: Core / Activity Worker Service / PostgreSQL Queue
**Resolution Date**: 2026-01-15

## Summary

When workers poll for activities via `POST /api/v1/workers/poll`, the current implementation executes a separate database query for each activity claimed. If a worker requests `max_activities=10`, this results in up to 10 sequential database queries with `LIMIT 1` each. This is highly inefficient and likely the primary cause of CPU load when external workers are active.

## Current Behavior

The activity polling flow in `core/src/activity/worker_service.rs:89-123`:

```rust
pub async fn poll_activities(
    &self,
    activity_types: Vec<(String, String)>, // Vec of (worker, name)
    worker_id: String,
    max_activities: usize,
) -> ActivityWorkerResult<Vec<PendingActivityRecord>> {
    let mut claimed = Vec::new();

    // Poll for each activity type until we reach max_activities
    for (worker, name) in activity_types {
        while claimed.len() < max_activities {
            // Delegate to ActivityQueue - ONE QUERY PER ACTIVITY
            match self.queue.claim_next(&worker_id, &worker, &name).await? {
                Some(activity) => {
                    claimed.push(PendingActivityRecord::from(activity));
                }
                None => break, // No more activities of this type
            }
        }

        if claimed.len() >= max_activities {
            break;
        }
    }

    Ok(claimed)
}
```

Each call to `claim_next` executes a single `UPDATE ... LIMIT 1` query in `core/src/queue/postgres_queue.rs:131-166`:

```sql
UPDATE activity_queue
SET status = 'running'::activity_status,
    claimed_at = NOW(),
    claimed_by = $3::TEXT,
    retry_count = CASE
        WHEN status = 'running'::activity_status THEN retry_count + 1
        ELSE retry_count
    END
WHERE id = (
    SELECT id FROM activity_queue
    WHERE worker = $1
      AND name = $2
      AND (
          (status = 'pending'::activity_status AND scheduled_for <= NOW())
          OR
          (status = 'running'::activity_status
           AND NOW() > claimed_at + timeout_duration
           AND retry_count < max_retries)
      )
    ORDER BY scheduled_for ASC
    LIMIT 1  -- ONE ACTIVITY AT A TIME
    FOR UPDATE SKIP LOCKED
)
RETURNING ...
```

## Performance Impact

**Example Scenario**: Worker polls with `activity_types: ["builtin.http_request", "builtin.postgres_query", "custom.process"]` and `max_activities: 10`

**Current Implementation**:
- Executes up to **10 sequential queries** (one per activity)
- Each query has full query planning + execution overhead
- Network round-trips between Rust and PostgreSQL
- Sequential execution means no parallelization

**Expected Implementation**:
- Execute **1 query** that claims up to 10 activities matching any of the 3 activity types
- Single query plan
- Single network round-trip
- Atomically claims all activities using `FOR UPDATE SKIP LOCKED`

**CPU Impact**:
- Current approach causes excessive CPU load on kruxiaflow server when external workers are polling frequently
- With multiple workers polling at short intervals (e.g., 100ms), query overhead becomes significant
- Database connection pool pressure from sequential queries

## Expected Behavior

A single optimized query should claim up to `max_activities` activities matching **any** of the requested activity types:

```sql
UPDATE activity_queue
SET status = 'running'::activity_status,
    claimed_at = NOW(),
    claimed_by = $worker_id,
    retry_count = CASE
        WHEN status = 'running'::activity_status THEN retry_count + 1
        ELSE retry_count
    END
WHERE id = ANY(
    SELECT id FROM activity_queue
    WHERE (worker, name) IN (
        ('builtin', 'http_request'),
        ('builtin', 'postgres_query'),
        ('custom', 'process')
    )
    AND (
        (status = 'pending'::activity_status AND scheduled_for <= NOW())
        OR
        (status = 'running'::activity_status
         AND NOW() > claimed_at + timeout_duration
         AND retry_count < max_retries)
    )
    ORDER BY scheduled_for ASC
    LIMIT 10  -- Claim up to max_activities
    FOR UPDATE SKIP LOCKED
)
RETURNING id, workflow_id, activity_key, worker, name as activity_name,
          parameters, settings, retry_count, claimed_at, output_definitions, iteration;
```

## Root Cause

The current implementation has two inefficiencies:

1. **API loops through activity types sequentially** (`for (worker, name) in activity_types`)
2. **Each activity is claimed individually** (`claim_next` with `LIMIT 1`)

This was likely implemented this way for simplicity, but it doesn't scale well with:
- High worker polling frequency
- Multiple activity types per worker
- High `max_activities` values

## Files Affected

- `core/src/queue/mod.rs` - `ActivityQueue` trait: update `claim_next` signature
- `core/src/queue/postgres_queue.rs` - `PostgresQueue::claim_next` implementation (lines 119-216): replace single `LIMIT 1` query with batched query
- `core/src/activity/worker_service.rs` - `poll_activities` method (lines 89-123): remove for-loop, call updated `claim_next` once
- `core/tests/queue_tests.rs` - Update test calls to `claim_next`
- `core/tests/scheduling_integration_tests.rs` - Update test calls to `claim_next`

## Proposed Fix

**Decision**: Update the existing `claim_next` method signature rather than adding a new method. Since there's only one implementation (PostgresQueue), this is the cleanest approach - no new methods, no for-loops, just a single optimized query.

### Updated Trait Signature

```rust
// In core/src/queue/mod.rs
trait ActivityQueue {
    async fn claim_next(
        &self,
        worker_id: &str,
        activity_types: Vec<(String, String)>, // Changed from single (worker, name)
        max_activities: usize,                  // Added
    ) -> Result<Vec<QueuedActivity>>;         // Changed from Option<QueuedActivity>
}
```

### Simplified Service Method

```rust
// In core/src/activity/worker_service.rs
pub async fn poll_activities(
    &self,
    activity_types: Vec<(String, String)>,
    worker_id: String,
    max_activities: usize,
) -> ActivityWorkerResult<Vec<PendingActivityRecord>> {
    // Single call, no loop needed
    let claimed = self.queue
        .claim_next(&worker_id, activity_types, max_activities)
        .await?
        .into_iter()
        .map(PendingActivityRecord::from)
        .collect();

    Ok(claimed)
}
```

## Implementation Notes

### PostgreSQL Query Construction

For the batched query, we need to dynamically build the `IN` clause:

```rust
// Build the activity type filter
let type_conditions: Vec<String> = activity_types
    .iter()
    .map(|(w, n)| format!("('{}', '{}')", w, n))
    .collect();
let type_filter = type_conditions.join(", ");

let query = format!(
    r#"
    UPDATE activity_queue
    SET status = 'running'::activity_status,
        claimed_at = NOW(),
        claimed_by = $1,
        retry_count = CASE
            WHEN status = 'running'::activity_status THEN retry_count + 1
            ELSE retry_count
        END
    WHERE id = ANY(
        SELECT id FROM activity_queue
        WHERE (worker, name) IN ({})
        AND (
            (status = 'pending'::activity_status AND scheduled_for <= NOW())
            OR
            (status = 'running'::activity_status
             AND NOW() > claimed_at + timeout_duration
             AND retry_count < max_retries)
        )
        ORDER BY scheduled_for ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
    )
    RETURNING id, workflow_id, activity_key, worker, name as activity_name,
              parameters, settings, retry_count, claimed_at, output_definitions, iteration
    "#,
    type_filter
);

sqlx::query(&query)
    .bind(worker_id)
    .bind(max_activities as i32)
    .fetch_all(&self.pool)
    .await?
```

**Note**: Need to ensure proper SQL injection protection. Consider using `sqlx::query_builder::QueryBuilder` or parameterized arrays instead of string formatting.

### Alternative: PostgreSQL Array Parameter

Use PostgreSQL arrays to avoid dynamic SQL:

```rust
// Create parallel arrays for workers and names
let workers: Vec<&str> = activity_types.iter().map(|(w, _)| w.as_str()).collect();
let names: Vec<&str> = activity_types.iter().map(|(_, n)| n.as_str()).collect();

sqlx::query!(
    r#"
    UPDATE activity_queue
    SET status = 'running'::activity_status,
        claimed_at = NOW(),
        claimed_by = $1,
        retry_count = CASE
            WHEN status = 'running'::activity_status THEN retry_count + 1
            ELSE retry_count
        END
    WHERE id = ANY(
        SELECT id FROM activity_queue
        WHERE worker = ANY($2::text[])
          AND name = ANY($3::text[])
        -- Note: This matches worker OR name, need to pair them correctly
        -- May need unnest() or other approach for proper pairing
    )
    ...
    "#,
    worker_id,
    &workers[..],
    &names[..]
)
```

**Challenge**: Properly matching (worker, name) pairs with arrays requires more complex SQL (e.g., using `unnest()` with row constructors).

## Testing Requirements

1. **Unit tests** for `claim_multiple`:
   - Single activity type, single activity available
   - Multiple activity types, mix of available activities
   - Request more activities than available
   - No activities available (empty result)
   - Concurrent claims from multiple workers (verify `FOR UPDATE SKIP LOCKED` works)

2. **Integration tests**:
   - Worker polls with `max_activities=10`, verify single query executed
   - Multiple workers polling simultaneously
   - Mix of pending and stale activities claimed correctly

3. **Performance benchmarks**:
   - Compare query count: current (N queries) vs fixed (1 query)
   - Measure CPU usage with external workers polling at 100ms intervals
   - Measure throughput: activities/second with current vs fixed implementation

## Related Performance Issues

- Worker polling frequency (currently 100ms default in `worker/src/config.rs`)
- Database connection pool sizing
- Query planning overhead for repeated similar queries

## Priority Justification

**High Severity** because:
- Directly impacts production performance
- Affects all external workers
- CPU usage scales linearly with worker count × polling frequency × max_activities
- Easy to fix with significant performance gains
- No architectural changes required, just query optimization

## References

- `core/src/activity/worker_service.rs:89-123` - Main polling loop
- `core/src/queue/postgres_queue.rs:119-216` - Single-activity claim query
- `api/src/handlers/workers.rs:294-373` - HTTP API handler for polling
- `worker/src/poller.rs:81-151` - Worker-side polling logic

## Resolution

**Implemented**: 2026-01-15

The bugfix was successfully implemented by:

1. **Updated `ActivityQueue` trait** (core/src/queue/mod.rs:30-36):
   - Changed `claim_next` signature to accept `Vec<(String, String)>` for activity types
   - Added `max_activities: usize` parameter
   - Changed return type from `Option<QueuedActivity>` to `Vec<QueuedActivity>`

2. **Implemented batched query** (core/src/queue/postgres_queue.rs:118-216):
   - Single SQL query using `unnest()` to match (worker, name) pairs from parallel arrays
   - Uses `LIMIT $max_activities` to claim multiple activities atomically
   - Maintains `FOR UPDATE SKIP LOCKED` for safe concurrent claiming
   - Returns early if activity_types is empty (avoids unnecessary database call)

3. **Simplified worker service** (core/src/activity/worker_service.rs:89-105):
   - Removed nested loops
   - Single call to `queue.claim_next()` with all activity types and max limit
   - Reduced code from ~30 lines to ~10 lines

4. **Updated all test calls**:
   - Updated core/tests/queue_tests.rs (12 tests)
   - Updated core/tests/scheduling_integration_tests.rs (1 test)
   - Changed from `claim_next(worker_id, "worker", "name")` to `claim_next(worker_id, vec![("worker".to_string(), "name".to_string())], 1)`
   - Changed assertions from `is_some()` to `!is_empty()` and unwrapping from `.unwrap()` to `[0]`

5. **Added comprehensive tests** (core/tests/batched_claim_tests.rs):
   - test_claim_multiple_activities_single_type: Verify claiming multiple activities of same type
   - test_claim_multiple_activities_multiple_types: Verify claiming from different activity types in single call
   - test_claim_respects_max_activities_limit: Ensure max_activities parameter is honored
   - test_claim_with_empty_activity_types: Verify empty input returns empty result
   - test_claim_when_fewer_available_than_requested: Verify partial claims work correctly
   - test_batched_claim_concurrent_workers: Verify concurrent claims remain safe with batching

**Performance Impact**:
- **Before**: N sequential queries (where N = min(available_activities, max_activities))
  - Example: max_activities=10 → up to 10 separate database queries
- **After**: 1 batched query regardless of max_activities
  - Example: max_activities=10 → exactly 1 database query

**Test Results**:
- All existing tests pass (12 queue tests, 6 scheduling tests, 237 core lib tests)
- All new batched claiming tests pass (6 new tests)
- No regressions detected
