# Bug Report: Activity Polling Query Optimization Investigation

**Date**: 2026-01-15
**Status**: Archived - No Change Required
**Severity**: Originally High (Performance) - Revised to Not a Bug
**Component**: Core / Activity Worker Service / PostgreSQL Queue
**Archive Date**: 2026-01-15

## Summary

This report documents an investigation into whether batching multiple activity type queries into a single query would improve performance. **Conclusion: The original sequential approach is optimal.** Multiple fast indexed queries outperform complex batched queries.

## Original Hypothesis

When workers poll for activities via `POST /api/v1/workers/poll`, the implementation executes separate database queries for each activity type. The hypothesis was that batching these into a single query would reduce overhead and improve performance.

## Investigation Results

### Approaches Tested

| Approach | Implementation | Performance vs Original |
|----------|---------------|------------------------|
| Original | Sequential queries per activity type | Baseline |
| v1: unnest() | `WHERE (worker, name) IN (SELECT * FROM unnest($2, $3))` | **-35% (regression)** |
| v2: activity_type column | Added generated column + `ANY()` matching | **-29% (regression)** |
| v3: Hybrid | Single-type fast path + batched fallback | **No improvement** |
| v4: Sequential with LIMIT | Iterate types, use LIMIT per type | **Equivalent to baseline** |

### Benchmark Data

**Before optimization attempts** (baseline):
```json
{
  "scenario": "High-Concurrency-3",
  "throughput_wf_per_sec": 120.34,
  "latency_p50_ms": 897.43
}
```

**After v1 (unnest)** - 35% regression:
```json
{
  "scenario": "High-Concurrency-3",
  "throughput_wf_per_sec": 78.74,
  "latency_p50_ms": 1450.12
}
```

**After v2 (activity_type column)** - Still slower:
```json
{
  "scenario": "High-Concurrency-3",
  "throughput_wf_per_sec": 85.27,
  "latency_p50_ms": 1068.14
}
```

### Root Cause Analysis

Query plan analysis revealed why batched queries are slower:

**Original single-type query** (fast):
```
Index Scan using idx_queue_claimable on activity_queue
  Index Cond: ((worker = 'builtin') AND (name = 'echo') AND (status = 'pending'))
  Planning Time: 0.130 ms
  Execution Time: 0.022 ms
```

**Batched query with unnest()** (slow):
```
Seq Scan on activity_queue
  Filter: (ROW(worker, name) = ANY ($1))
  Planning Time: 0.292 ms
  Execution Time: 1.683 ms
```

**Key findings**:
1. PostgreSQL cannot use composite indexes efficiently with `IN (SELECT ... FROM unnest())`
2. Row tuple comparisons `(worker, name) IN (...)` prevent index usage
3. Even with a generated `activity_type` column and dedicated index, the query planner chose less efficient access patterns
4. The overhead of complex query planning exceeds the cost of multiple simple queries

### Why Hybrid Approach Failed

The hybrid approach (use fast path for single activity type, batched for multiple) didn't help because:

- The builtin worker registers **7 activity types**: echo, http_request, postgres_query, postgres_transaction, llm_prompt, embedding, email_send
- Even when only `builtin.echo` is used in benchmarks, workers poll for all 7 types
- This means `activity_types.len() == 7`, always triggering the slow batched path

### Why Sequential is Optimal

For typical workloads:

1. **Early termination**: We stop as soon as we find enough work (often on first type with pending activities)
2. **Index optimization**: Each query uses `idx_queue_claimable (worker, name, status, scheduled_for)` optimally
3. **Lower planning overhead**: Simple queries have ~0.13ms planning vs ~0.29ms for complex queries
4. **Predictable performance**: N simple queries = N × 0.022ms execution time

For a worker polling 7 activity types but only finding work in 1:
- Sequential: 1-2 fast queries (stop when work found)
- Batched: 1 slow query (always scans for all types)

## Final Implementation

The `claim_next` method now:
1. Accepts `Vec<(String, String)>` for activity types and `max_activities: usize`
2. Iterates through activity types sequentially
3. Uses fast single-type queries with `LIMIT` per type
4. Stops as soon as `max_activities` are claimed

```rust
async fn claim_next(
    &self,
    worker_id: &str,
    activity_types: Vec<(String, String)>,
    max_activities: usize,
) -> Result<Vec<QueuedActivity>> {
    let mut results = Vec::new();

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

## Schema Changes

A migration was added during investigation (`20260115000001_optimize_activity_batched_claim`):
- Added `activity_type` generated column (`worker || ':' || name`)
- Added `idx_queue_batched_claim` index

These remain in the schema but are not used by the current implementation. They may be useful for future analytics queries.

## Lessons Learned

1. **Benchmark before optimizing**: The original assumption that batching would help was wrong
2. **Query plans matter more than query count**: PostgreSQL's query planner makes simple queries very efficient
3. **Index selectivity is critical**: Composite indexes work best with simple equality predicates
4. **Test with realistic workloads**: The builtin worker's 7 activity types revealed the hybrid approach's flaw

## Files Modified

- `core/src/queue/mod.rs` - Updated `ActivityQueue::claim_next` signature
- `core/src/queue/postgres_queue.rs` - Implemented sequential claiming with LIMIT
- `core/src/activity/worker_service.rs` - Simplified to single `claim_next` call
- `api/src/state.rs` - Updated mock implementation
- `core/tests/queue_tests.rs` - Updated test calls
- `core/tests/scheduling_integration_tests.rs` - Updated test calls
- `core/tests/batched_claim_tests.rs` - Added new tests for batched claiming behavior

## Conclusion

**No performance bug exists.** The original sequential query approach is optimal for PostgreSQL with proper indexing. The investigation confirmed that:

- Multiple simple indexed queries > One complex batched query
- Query planning overhead dominates for small result sets
- Early termination in sequential approach is a feature, not a bug

The API signature was updated to be cleaner (accepting all activity types at once), but the underlying implementation remains sequential for performance reasons.
