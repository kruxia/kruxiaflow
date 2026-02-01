# Bug: Activity Polling Should Filter by Worker, Not Activity Type

**Date**: 2026-01-15
**Status**: Implemented - Pending Benchmark
**Severity**: Medium (Design / Fairness)
**Component**: Core / Activity Worker Service / PostgreSQL Queue / API
**Implementation Date**: 2026-01-15

## Summary

The current activity polling implementation iterates through activity types sequentially, creating unfair priority for activity types earlier in the list. If a worker supports multiple activity types and there's pending work for all of them, only the first types get claimed.

**Solution**: Change the polling interface to filter by `worker` name only, not individual `(worker, name)` pairs. This allows a single efficient query that fairly claims activities across all types for that worker.

## Current Behavior

### API Request

```json
POST /api/v1/workers/poll
{
  "worker_id": "worker-abc-123",
  "activity_types": [
    ["builtin", "echo"],
    ["builtin", "http_request"],
    ["builtin", "postgres_query"],
    ["builtin", "llm_prompt"]
  ],
  "max_activities": 10
}
```

### Implementation (Sequential)

```rust
// core/src/queue/postgres_queue.rs
async fn claim_next(
    &self,
    worker_id: &str,
    activity_types: Vec<(String, String)>,  // List of (worker, name) pairs
    max_activities: usize,
) -> Result<Vec<QueuedActivity>> {
    let mut results = Vec::new();

    // PROBLEM: Iterates sequentially - first types get priority
    for activity_type in &activity_types {
        if results.len() >= max_activities {
            break;
        }
        let remaining = max_activities - results.len();
        let mut claimed = self
            .claim_next_single_type(worker_id, activity_type, remaining)
            .await?;
        results.append(&mut claimed);
    }

    Ok(results)
}
```

### Problem Scenario

Given:
- 10 pending `builtin.echo` activities
- 10 pending `builtin.http_request` activities
- 10 pending `builtin.postgres_query` activities
- Worker polls with `max_activities=10`

Result:
- Claims 10 `builtin.echo` activities
- Claims 0 `builtin.http_request` activities
- Claims 0 `builtin.postgres_query` activities

The `http_request` and `postgres_query` activities are starved until all `echo` activities are processed.

## Expected Behavior

### New API Request

```json
POST /api/v1/workers/poll
{
  "worker_id": "worker-abc-123",
  "worker": "builtin",
  "max_activities": 10
}
```

### New Implementation (Worker-Level)

```rust
// core/src/queue/postgres_queue.rs
async fn claim_next(
    &self,
    worker_id: &str,
    worker: &str,           // Single worker name (e.g., "builtin")
    max_activities: usize,
) -> Result<Vec<QueuedActivity>> {
    // Single query filtering only on worker
    sqlx::query!(
        r#"
        UPDATE activity_queue
        SET status = 'running'::activity_status,
            claimed_at = NOW(),
            claimed_by = $1::TEXT,
            retry_count = CASE
                WHEN status = 'running'::activity_status THEN retry_count + 1
                ELSE retry_count
            END
        WHERE id = ANY(
            SELECT id FROM activity_queue
            WHERE worker = $2
              AND (
                  (status = 'pending'::activity_status AND scheduled_for <= NOW())
                  OR
                  (status = 'running'::activity_status
                   AND NOW() > claimed_at + timeout_duration
                   AND retry_count < max_retries)
              )
            ORDER BY scheduled_for ASC
            LIMIT $3
            FOR UPDATE SKIP LOCKED
        )
        RETURNING ...
        "#,
        worker_id,
        worker,
        max_activities as i64
    )
}
```

### Expected Result

Given the same scenario:
- Claims activities ordered by `scheduled_for` regardless of type
- Fair distribution based on scheduling order, not activity type
- Single efficient query using `idx_queue_claimable` (starts with `worker` column)

## Benefits

1. **Fair scheduling**: Activities claimed in `scheduled_for` order across all types
2. **Single query**: One efficient query vs N sequential queries
3. **Simpler API**: Worker specifies which worker it handles, not individual activity names
4. **Better index usage**: `idx_queue_claimable (worker, name, status, scheduled_for)` can use leading `worker` column efficiently
5. **Reduced complexity**: No activity type list management on worker side

## Implementation Plan

### 1. Update `ActivityQueue` Trait

**File**: `core/src/queue/mod.rs`

```rust
// Before
async fn claim_next(
    &self,
    worker_id: &str,
    activity_types: Vec<(String, String)>,
    max_activities: usize,
) -> Result<Vec<QueuedActivity>>;

// After
async fn claim_next(
    &self,
    worker_id: &str,
    worker: &str,
    max_activities: usize,
) -> Result<Vec<QueuedActivity>>;
```

### 2. Update `PostgresQueue::claim_next`

**File**: `core/src/queue/postgres_queue.rs`

- Remove `claim_next_single_type` helper method
- Implement single query filtering on `worker` column only
- Keep `FOR UPDATE SKIP LOCKED` for concurrent safety
- Order by `scheduled_for ASC` for fair scheduling

```rust
async fn claim_next(
    &self,
    worker_id: &str,
    worker: &str,
    max_activities: usize,
) -> Result<Vec<QueuedActivity>> {
    if max_activities == 0 {
        return Ok(vec![]);
    }

    let activities = sqlx::query!(
        r#"
        UPDATE activity_queue
        SET status = 'running'::activity_status,
            claimed_at = NOW(),
            claimed_by = $1::TEXT,
            retry_count = CASE
                WHEN status = 'running'::activity_status THEN retry_count + 1
                ELSE retry_count
            END
        WHERE id = ANY(
            SELECT id FROM activity_queue
            WHERE worker = $2
              AND (
                  (status = 'pending'::activity_status AND scheduled_for <= NOW())
                  OR
                  (status = 'running'::activity_status
                   AND NOW() > claimed_at + timeout_duration
                   AND retry_count < max_retries)
              )
            ORDER BY scheduled_for ASC
            LIMIT $3
            FOR UPDATE SKIP LOCKED
        )
        RETURNING id, workflow_id, activity_key, worker, name as activity_name,
                  parameters, settings, retry_count, claimed_at, output_definitions, iteration
        "#,
        worker_id,
        worker,
        max_activities as i64
    )
    .fetch_all(&self.pool)
    .await?;

    // ... convert to QueuedActivity structs
}
```

### 3. Update `WorkerService::poll_activities`

**File**: `core/src/activity/worker_service.rs`

```rust
// Before
pub async fn poll_activities(
    &self,
    activity_types: Vec<(String, String)>,
    worker_id: String,
    max_activities: usize,
) -> ActivityWorkerResult<Vec<PendingActivityRecord>>

// After
pub async fn poll_activities(
    &self,
    worker: &str,
    worker_id: &str,
    max_activities: usize,
) -> ActivityWorkerResult<Vec<PendingActivityRecord>>
```

### 4. Update API Request/Response Types

**File**: `api/src/handlers/workers.rs`

```rust
// Before
#[derive(Deserialize)]
pub struct PollActivitiesRequest {
    pub worker_id: String,
    pub activity_types: Vec<(String, String)>,
    pub max_activities: Option<usize>,
}

// After
#[derive(Deserialize)]
pub struct PollActivitiesRequest {
    pub worker_id: String,
    pub worker: String,
    pub max_activities: Option<usize>,
}
```

### 5. Update API Handler

**File**: `api/src/handlers/workers.rs`

Update `poll_activities` handler to pass `worker` instead of `activity_types`.

### 6. Update Mock Implementation

**File**: `api/src/state.rs`

Update the mock `ActivityQueue` implementation for API tests.

### 7. Update Worker Client

**File**: `worker/src/poller.rs`

```rust
// Before: Worker sends list of activity types
let request = PollActivitiesRequest {
    worker_id: self.worker_id.clone(),
    activity_types: self.activity_types.clone(),
    max_activities: Some(self.max_activities),
};

// After: Worker sends worker name
let request = PollActivitiesRequest {
    worker_id: self.worker_id.clone(),
    worker: self.worker_name.clone(),
    max_activities: Some(self.max_activities),
};
```

### 8. Update Worker Configuration

**File**: `worker/src/config.rs`

```rust
// Before
pub struct WorkerConfig {
    pub activity_types: Vec<(String, String)>,
    // ...
}

// After
pub struct WorkerConfig {
    pub worker: String,  // e.g., "builtin", "custom"
    // ...
}
```

### 9. Update Tests

**Files**:
- `core/tests/queue_tests.rs`
- `core/tests/scheduling_integration_tests.rs`
- `core/tests/batched_claim_tests.rs`
- `api/tests/*.rs`
- `worker/tests/*.rs`

Update all test calls to use new signature.

## Index Changes

The existing `idx_queue_claimable` index includes `name` which is no longer needed:

```sql
-- Current (suboptimal for worker-level filtering)
CREATE INDEX idx_queue_claimable ON activity_queue (worker, name, status, scheduled_for)
WHERE status IN ('pending', 'running');
```

### New Index Design

Since we're now filtering only by `worker`, the index should be updated:

```sql
-- Drop old index
DROP INDEX IF EXISTS idx_queue_claimable;

-- New index without 'name' column
CREATE INDEX idx_queue_claimable ON activity_queue (worker, status, scheduled_for)
WHERE status IN ('pending', 'running');
```

### Benchmark: Include Status Column or Not?

During implementation, benchmark these two index designs:

**Option A: With status in index columns**
```sql
CREATE INDEX idx_queue_claimable ON activity_queue (worker, status, scheduled_for)
WHERE status IN ('pending', 'running');
```
- Pro: Can filter on status efficiently for different status queries
- Con: Larger index, status already filtered by partial index predicate

**Option B: Without status in index columns**
```sql
CREATE INDEX idx_queue_claimable ON activity_queue (worker, scheduled_for)
WHERE status IN ('pending', 'running');
```
- Pro: Smaller index, status filtering handled by partial index predicate
- Con: Less flexible if we need to filter by specific status

**Benchmark queries**:
```sql
-- Test with EXPLAIN ANALYZE
EXPLAIN ANALYZE
SELECT id FROM activity_queue
WHERE worker = 'builtin'
  AND status = 'pending'::activity_status
  AND scheduled_for <= NOW()
ORDER BY scheduled_for ASC
LIMIT 10
FOR UPDATE SKIP LOCKED;
```

### Migration

**File**: `migrations/20260115000002_optimize_worker_level_claiming.up.sql`

```sql
-- Optimize idx_queue_claimable for worker-level filtering (remove 'name' column)
DROP INDEX IF EXISTS idx_queue_claimable;

-- Benchmark determined: [status included/excluded based on benchmark results]
CREATE INDEX idx_queue_claimable ON activity_queue (worker, scheduled_for)
WHERE status IN ('pending', 'running');
```

**File**: `migrations/20260115000002_optimize_worker_level_claiming.down.sql`

```sql
-- Restore original index with 'name' column
DROP INDEX IF EXISTS idx_queue_claimable;

CREATE INDEX idx_queue_claimable ON activity_queue (worker, name, status, scheduled_for)
WHERE status IN ('pending', 'running');
```

## Migration Path

### API Versioning

Option A: **Breaking change** - Update API v1 (acceptable for pre-release)
Option B: **Non-breaking** - Support both `activity_types` and `worker` parameters temporarily

Recommended: Option A (breaking change) since this is pre-release software.

### Worker Compatibility

External workers using the Python SDK will need to update their polling requests. Update SDK documentation and examples.

## Testing Requirements

1. **Unit tests**:
   - Claim activities across multiple types fairly
   - Verify `scheduled_for` ordering is respected
   - Empty result when no activities for worker
   - Concurrent claiming safety with `FOR UPDATE SKIP LOCKED`

2. **Integration tests**:
   - Worker polls and receives mixed activity types
   - Fair distribution when multiple types have pending work
   - Stale activity reclamation works across types

3. **Benchmark tests**:
   - Compare performance: worker-level vs activity-type-level filtering
   - Verify index usage with `EXPLAIN ANALYZE`

## Files Affected

| File | Change |
|------|--------|
| `migrations/20260115000002_optimize_worker_level_claiming.up.sql` | New index without `name` column |
| `migrations/20260115000002_optimize_worker_level_claiming.down.sql` | Rollback migration |
| `core/src/queue/mod.rs` | Update `claim_next` signature |
| `core/src/queue/postgres_queue.rs` | Implement worker-level query |
| `core/src/activity/worker_service.rs` | Update `poll_activities` |
| `api/src/handlers/workers.rs` | Update request type and handler |
| `api/src/state.rs` | Update mock implementation |
| `worker/src/poller.rs` | Update poll request |
| `worker/src/config.rs` | Simplify configuration |
| `worker/src/builtin.rs` | Update builtin worker setup |
| `core/tests/queue_tests.rs` | Update tests |
| `core/tests/scheduling_integration_tests.rs` | Update tests |
| `core/tests/batched_claim_tests.rs` | Update/rename tests |
| `sdk/python/kruxiaflow/worker.py` | Update Python SDK |
| `docs/api-reference.md` | Update API documentation |

## Priority

**Medium** - This is a design improvement for fairness, not a critical bug. The current implementation works correctly but may cause activity starvation in specific scenarios.

## References

- Previous investigation: `docs/bugs/archived/2026-01-15-inefficient-activity-polling-multiple-queries.md`
- Activity queue schema: `migrations/20240101000001_initial_schema.up.sql`
- Index definitions: `migrations/20240601000001_add_queue_indexes.up.sql`
