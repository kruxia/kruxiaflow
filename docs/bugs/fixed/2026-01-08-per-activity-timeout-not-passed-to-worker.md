# Per-Activity Timeout Not Passed to Worker

**Date**: 2026-01-08
**Status**: Fixed
**Severity**: High

## Summary

Per-activity `timeout_seconds` settings configured in workflow definitions were not being passed to the internal worker, causing all activities to use the hardcoded 300-second default timeout regardless of their configured timeout.

## Symptoms

- Activities with `timeout_seconds: 900` in workflow settings still timeout after 300 seconds
- Log shows: `Activity execution timed out after 300s` even when 15-minute timeout was configured
- Large embedding jobs for books with 5000+ passages fail due to insufficient timeout

## Root Cause

In `api/src/handlers/workers.rs`, the code that extracts the timeout from activity settings was looking for the wrong field name:

```rust
// BUG: Looking for "timeout" but the field is "timeout_seconds"
let timeout_seconds = a
    .settings
    .as_ref()
    .and_then(|s| s.get("timeout"))  // WRONG!
    .and_then(|t| t.as_i64());
```

The workflow definition stores the timeout as `timeout_seconds` (which is correct per the `ActivitySettings` struct), but the API handler was looking for `timeout`. This caused `timeout_seconds` to always be `None`, falling back to the worker's default 300-second timeout.

## Steps to Reproduce

1. Create a workflow with a long-running activity:
   ```yaml
   - key: generate_embeddings
     worker: std
     activity_name: embedding
     settings:
       timeout_seconds: 900  # 15 minutes
   ```

2. Deploy the workflow and trigger it with a large document

3. Observe that the activity times out after 300 seconds instead of 900 seconds

4. Query the activity_queue to confirm timeout is stored correctly:
   ```sql
   SELECT timeout_duration, settings FROM activity_queue
   WHERE activity_key = 'generate_embeddings';
   -- Shows timeout_duration: 00:15:00 but worker ignores it
   ```

## Fix

Changed the field lookup from `"timeout"` to `"timeout_seconds"`:

```rust
// FIXED: Use correct field name
let timeout_seconds = a
    .settings
    .as_ref()
    .and_then(|s| s.get("timeout_seconds"))
    .and_then(|t| t.as_i64());
```

Additionally, made the default worker timeout configurable via `KRUXIAFLOW_ACTIVITY_TIMEOUT` environment variable (defaults to 300 for backward compatibility).

## Files Changed

- `api/src/handlers/workers.rs` - Fixed field name lookup
- `kruxiaflow/src/commands/serve.rs` - Added configurable `activity_timeout` parameter

## Testing

### Manual Testing

After fix:
1. Deploy workflow with `timeout_seconds: 900`
2. Trigger activity and verify it runs for up to 15 minutes
3. Confirm `timeout_seconds` is correctly passed in API response to worker

### Automated Tests

Regression tests added in `api/tests/worker_activity_integration_tests.rs`:

- `test_poll_returns_timeout_seconds_from_settings` - Verifies timeout_seconds is extracted from settings and returned to worker
- `test_poll_returns_various_timeout_values` - Tests multiple timeout values (1s, 10s, 30s, 60s)
- `test_poll_returns_null_timeout_when_not_configured` - Verifies null returned when no settings configured
- `test_poll_returns_null_timeout_when_settings_has_no_timeout` - Verifies null when settings exist but timeout is not set

Run with: `cargo test --package kruxiaflow-api --test worker_activity_integration_tests -- timeout`

## Related

- Queue timeout (`timeout_duration` in `activity_queue`) was already working correctly for stale activity reclamation
- The issue was specifically in passing the timeout to the worker for execution timeout enforcement
