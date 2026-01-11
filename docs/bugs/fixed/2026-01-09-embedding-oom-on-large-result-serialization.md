# Embedding Activity OOM on Large Result Serialization

**Date**: 2026-01-09
**Status**: Fixed
**Severity**: High
**Component**: Worker - Embedding Activity

## Summary

The embedding activity crashes with OOM (exit code 137) when processing large documents (5000+ passages), even though batch processing completes successfully. The crash occurs when serializing and returning the accumulated embeddings.

## Symptoms

1. Embedding batches complete successfully (all 11 batches for 5131 passages)
2. Immediately after "All embedding batches completed", kruxiaflow exits with code 137
3. PostgreSQL logs "could not receive data from client: Connection reset by peer"
4. Worker logs "Poll failed, will retry error=Failed to poll for activities"

## Root Cause

The embedding activity accumulates ALL embeddings in memory before returning:

```rust
// worker/src/activities/llm.rs lines 855-894
let mut all_embeddings: Vec<Vec<f64>> = Vec::with_capacity(total_inputs);

for (batch_idx, batch) in params.input.chunks(batch_size).enumerate() {
    let response = fallback_chain.embed(&batch_request).await?;
    all_embeddings.extend(response.embeddings);  // Accumulates in memory
}

// Returns all embeddings at once
let outputs = json!({
    "embeddings": all_embeddings,  // ~63MB for 5131 × 3072 embeddings
    ...
});
```

Memory calculation for 5131 passages:
- 5131 embeddings × 3072 dimensions × 8 bytes (f64) = ~126MB raw data
- JSON serialization with array notation adds ~50-100% overhead
- Total memory spike: ~200-250MB during serialization

When the container has limited memory, this spike causes OOM.

## Reproduction

1. Ingest a large PDF with 400+ pages (generates 5000+ passages)
2. Observe embedding activity processing batches successfully
3. Watch for OOM crash immediately after "All embedding batches completed"

```
INFO All embedding batches completed total_embeddings=5131 total_prompt_tokens=361202
postgres-1 | could not receive data from client: Connection reset by peer
kruxiaflow-1 exited with code 137
```

## Impact

- Large document ingestion fails at the final step
- Workflow appears stuck until timeout (5 minutes)
- All embedding API costs are wasted since results aren't stored
- Source remains in "processing" state

## Workaround

Increase container memory limit in docker-compose.yml:

```yaml
kruxiaflow:
  deploy:
    resources:
      limits:
        memory: 4G
```

This delays the problem but doesn't solve it for very large documents.

## Proposed Solution

Stream embeddings to workflow storage instead of accumulating in memory:

1. After each batch, append embeddings to a workflow file (JSON Lines format)
2. Return a file reference instead of the full embeddings array
3. Consuming activity reads from file reference

See: `/docs/features/streaming-embeddings-to-workflow-storage.md`

## Fix Applied

The streaming solution was implemented on 2026-01-09:

1. Added `ActivityContext` to pass workflow metadata and storage to activities
2. EmbeddingActivity now streams embeddings to workflow storage for large inputs (> 1000 passages)
3. Returns `embeddings_file` reference instead of inline embeddings array
4. Consumer activities (e.g., researcher's `store.passages`) read from file reference

Key changes:
- `worker/src/registry.rs` - Added ActivityContext, execute_with_context
- `worker/src/activities/llm.rs` - Streaming EmbeddingActivity implementation
- `worker/src/poller.rs` - Passes context to activities

## Testing

### Automated Tests

Regression tests added in `worker/tests/embedding_streaming_test.rs`:

- `test_streaming_embeddings_returns_file_reference` - Verifies streaming to storage returns embeddings_file reference
- `test_inline_embeddings_without_storage` - Verifies fallback to inline embeddings when no storage available
- `test_batch_processing_streams_incrementally` - Verifies batch processing streams to storage incrementally
- `test_output_structure_has_both_keys` - Verifies both embeddings and embeddings_file keys always present
- `test_direct_execute_returns_inline` - Verifies execute() fallback returns inline embeddings
- `test_usage_metrics_reported` - Verifies usage metrics are correctly reported

Run with:
```bash
cargo test --package kruxiaflow-worker --test embedding_streaming_test
```

## Related

- Feature: Batched Embeddings (implemented - handles API timeout but not memory)
- WorkflowStorage trait (available - supports streaming upload/download)
- PostgreSQL Large Objects (implemented - memory-efficient file storage)
