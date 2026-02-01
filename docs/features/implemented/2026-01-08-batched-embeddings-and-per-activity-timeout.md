# Batched Embeddings and Per-Activity Timeout Fix

**Date**: 2026-01-08
**Status**: Implemented
**Components**: API, Worker, Serve Command

## Summary

This release includes three related changes to support long-running embedding operations for large documents:

1. **Bug Fix**: Per-activity timeout settings now correctly passed to workers
2. **Enhancement**: Configurable default activity timeout via environment variable
3. **Feature**: Automatic batching for large embedding requests

## Problem Statement

When processing large documents (5000+ passages), the embedding activity would timeout:

1. Per-activity `timeout_seconds` configured in workflow YAML was ignored
2. The hardcoded 300-second worker timeout was insufficient
3. Single API call for 5000+ embeddings took 15+ minutes

## Changes

### 1. Per-Activity Timeout Bug Fix

**File**: `api/src/handlers/workers.rs`

The API was looking for the wrong field name when extracting per-activity timeout from settings:

```rust
// Before (BUG)
let timeout_seconds = a
    .settings
    .as_ref()
    .and_then(|s| s.get("timeout"))  // Wrong field name!
    .and_then(|t| t.as_i64());

// After (FIXED)
let timeout_seconds = a
    .settings
    .as_ref()
    .and_then(|s| s.get("timeout_seconds"))  // Correct field name
    .and_then(|t| t.as_i64());
```

This fix allows workflows to specify per-activity timeouts:

```yaml
- key: generate_embeddings
  worker: std
  activity_name: embedding
  settings:
    timeout_seconds: 900  # 15 minutes - now correctly applied
```

### 2. Configurable Default Activity Timeout

**File**: `kruxiaflow/src/commands/serve.rs`

Added `KRUXIAFLOW_ACTIVITY_TIMEOUT` environment variable to configure the default worker activity timeout:

```rust
/// Default activity execution timeout in seconds
#[arg(
    long,
    env = "KRUXIAFLOW_ACTIVITY_TIMEOUT",
    default_value = "300",
    help = "Default activity execution timeout in seconds (can be overridden per-activity)"
)]
pub activity_timeout: u64,
```

Usage:
```bash
# Set default timeout to 15 minutes
KRUXIAFLOW_ACTIVITY_TIMEOUT=900 kruxiaflow serve
```

### 3. Automatic Batching for Embeddings

**File**: `worker/src/activities/llm.rs`

The embedding activity now automatically batches large inputs to prevent timeouts:

```rust
/// Embedding Activity parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingParams {
    pub model: ModelSpec,
    pub input: Vec<String>,

    /// Batch size for large inputs (default: 500)
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}
```

Behavior:
- Inputs <= batch_size: Processed in single API call (unchanged)
- Inputs > batch_size: Automatically split into batches
- Progress logged after each batch
- Results flattened to match original interface

Example log output for 5131 passages:
```
INFO Processing embeddings in batches total_inputs=5131 batch_size=500 num_batches=11
INFO Processing embedding batch batch=1 batch_size=500 progress="500/5131"
INFO Batch completed batch=1 embeddings_generated=500
INFO Processing embedding batch batch=2 batch_size=500 progress="1000/5131"
...
INFO All embedding batches completed total_embeddings=5131 total_prompt_tokens=...
```

## Workflow Configuration

No workflow changes required for basic usage. Optional configuration:

```yaml
- key: generate_embeddings
  worker: std
  activity_name: embedding
  parameters:
    model: google/gemini-embedding-001
    input: "{{extract_content.passages | map(attribute='content') | list}}"
    batch_size: 500  # Optional: customize batch size
  settings:
    timeout_seconds: 1800  # 30 minutes for very large documents
    budget:
      limit: 0.50
```

## Benefits

1. **Large document support**: Books with 5000+ passages now process successfully
2. **Progress visibility**: Logs show batch-by-batch progress
3. **Configurable**: Both timeout and batch size can be tuned per workflow
4. **Backward compatible**: Existing workflows work unchanged
5. **Memory efficient**: Processes in chunks rather than all at once

## Testing

1. Small document (<500 passages): Single API call, fast completion
2. Medium document (500-2000 passages): 1-4 batches, ~2-4 minutes
3. Large document (5000+ passages): 10+ batches, ~10-15 minutes

### Automated Test Coverage

Tests added in `worker/tests/activity_timeout_tests.rs`:
- `test_custom_timeout_allows_longer_execution` - Custom timeout overrides default
- `test_activity_times_out_when_exceeding_custom_timeout` - Timeout enforcement
- `test_fast_activity_succeeds_with_any_timeout` - Fast activities complete
- `test_timeout_precision` - Timeout timing is accurate
- `test_multiple_activities_different_timeouts` - Independent timeout enforcement
- `test_context_execution_respects_timeout` - Context-aware timeout
- `test_context_execution_succeeds_with_sufficient_timeout` - Context success path

Tests added in `worker/tests/embedding_streaming_test.rs`:
- `test_batching_splits_large_inputs` - Batch splitting verification
- `test_single_batch_for_small_inputs` - Small input handling
- `test_default_batch_size` - Default batch size applied

## Migration

No migration required. Existing workflows benefit automatically from:
- Batched embedding (transparent)
- Per-activity timeout (if already configured in YAML)

To take full advantage:
1. Set `timeout_seconds` in workflow for long-running activities
2. Optionally set `KRUXIAFLOW_ACTIVITY_TIMEOUT` for global default

## Related Issues

- Large book ingestion timeout after 5 minutes
- Per-activity timeout not being applied to workers
- No visibility into embedding progress for large documents
