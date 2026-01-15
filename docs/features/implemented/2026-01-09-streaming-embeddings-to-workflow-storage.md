# Streaming Embeddings to Workflow Storage

**Date**: 2026-01-09
**Status**: Implemented
**Priority**: High
**Related Bug**: `2026-01-09-embedding-oom-on-large-result-serialization.md`

## Problem Statement

The embedding activity crashes with OOM when processing large documents (5000+ passages) because it accumulates all embeddings in memory before returning them in the activity result.

Current memory usage pattern:
```
Batch 1:  [500 embeddings] → accumulate
Batch 2:  [500 embeddings] → accumulate
...
Batch 11: [131 embeddings] → accumulate
Return:   [5131 embeddings] → JSON serialize → OOM crash
```

For 5131 embeddings × 3072 dimensions × 8 bytes = ~126MB raw + JSON overhead = ~250MB spike.

## Proposed Solution

Stream embeddings to workflow storage during batch processing instead of accumulating in memory.

### Option A: WorkflowStorage Integration (Recommended)

Modify the embedding activity to write embeddings to workflow storage batch-by-batch, then return a file reference.

**Memory usage pattern:**
```
Batch 1:  [500 embeddings] → write to storage → release memory
Batch 2:  [500 embeddings] → write to storage → release memory
...
Batch 11: [131 embeddings] → write to storage → release memory
Return:   { "embeddings_file": "postgres://workflow_id/activity_key/embeddings.jsonl" }
```

Peak memory: ~12MB (one batch of 500 × 3072 × 8 bytes)

**Changes Required:**

1. **Activity Context** - Pass `WorkflowStorage` and `workflow_id` to activities
2. **EmbeddingActivity** - Stream batches to storage for large inputs
3. **Output Format** - Return file reference for large outputs, inline for small
4. **Consumer Activities** - Read from file reference when provided

### Option B: Direct Database Streaming

For application-specific workflows, have the embedding activity write directly to the target database table.

**Pros:**
- Most memory efficient - embeddings go directly to final destination
- No intermediate storage

**Cons:**
- Tight coupling between embedding activity and application schema
- Not suitable for generic workflow engine
- Breaks separation of concerns

### Option C: Chunked Result Pagination

Return embeddings in chunks via multiple activity completions.

**Pros:**
- No storage changes needed

**Cons:**
- Complex workflow logic to handle chunked results
- Multiple API round trips
- Doesn't solve serialization memory spike

## Recommended Implementation: Option A

### Phase 1: Activity Context Enhancement

Add `ActivityContext` to provide activities with workflow metadata and storage:

```rust
/// Context available to activities during execution
pub struct ActivityContext {
    pub workflow_id: Uuid,
    pub activity_id: Uuid,
    pub activity_key: String,
    pub storage: Option<Arc<dyn WorkflowStorage>>,
}

#[async_trait]
pub trait ActivityImpl: Send + Sync {
    /// Execute with full context (new method)
    async fn execute_with_context(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult> {
        // Default: delegate to simple execute (backwards compatible)
        self.execute(parameters).await
    }

    /// Simple execute (existing, for backwards compatibility)
    async fn execute(&self, parameters: Value) -> Result<ActivityResult>;
}
```

### Phase 2: Streaming Embedding Output

Modify `EmbeddingActivity` to always stream embeddings to workflow storage:

```rust
// All embeddings are streamed - no threshold, consistent behavior

async fn execute_with_context(
    &self,
    parameters: Value,
    ctx: &ActivityContext,
) -> Result<ActivityResult> {
    let params: EmbeddingParams = serde_json::from_value(parameters)?;
    let total_inputs = params.input.len();

    // Require workflow storage for streaming
    let storage = ctx.storage.as_ref()
        .ok_or_else(|| anyhow!("Workflow storage required"))?;

    let filename = "embeddings.jsonl";
    let fallback_chain = params.model.to_fallback_chain()?;

    // Create streaming writer
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(16);
    let upload_handle = tokio::spawn({
        let storage = storage.clone();
        let workflow_id = ctx.workflow_id;
        let activity_key = ctx.activity_key.clone();
        async move {
            let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
                .map(Ok::<_, std::io::Error>);
            storage.upload_file(
                workflow_id,
                &activity_key,
                filename,
                Some("application/x-ndjson"),
                Box::pin(stream),
            ).await
        }
    });

    // Process batches and stream to storage
    let mut total_prompt_tokens = 0u32;
    let mut model_name = String::new();
    let mut provider_name = String::new();
    let mut embedding_count = 0usize;

    for batch in params.input.chunks(params.batch_size) {
        let response = fallback_chain.embed(&EmbeddingRequest {
            model: String::new(),
            input: batch.to_vec(),
        }).await?;

        // Write embeddings as JSON Lines (one per line)
        for embedding in response.embeddings {
            let line = serde_json::to_string(&embedding)?;
            tx.send(Bytes::from(format!("{}\n", line))).await?;
            embedding_count += 1;
        }

        total_prompt_tokens += response.usage.prompt_tokens;
        model_name = response.model;
        provider_name = response.provider;
    }

    // Close stream and wait for upload
    drop(tx);
    let metadata = upload_handle.await??;

    // Return file reference instead of embeddings
    let outputs = json!({
        "embeddings_file": format!("postgres://{}/{}/{}",
            ctx.workflow_id, ctx.activity_key, filename),
        "embedding_count": embedding_count,
        "model": model_name,
        "provider": provider_name,
        "usage": {
            "prompt_tokens": total_prompt_tokens,
        }
    });

    Ok(ActivityResult::value("result", outputs))
}
```

### Phase 3: Consumer Activity Updates

Activities that consume embeddings need to handle both formats:

```yaml
# Workflow YAML - consumer activity reads from file if provided
- key: store_passages
  worker: researcher
  activity_name: store.passages
  parameters:
    db_url: "{{INPUT.db_url}}"
    source_id: "{{INPUT.source_id}}"
    passages: "{{extract_content.passages}}"
    # Support both inline and file reference
    embeddings: "{{generate_embeddings.result.embeddings}}"
    embeddings_file: "{{generate_embeddings.result.embeddings_file}}"
```

Consumer activity implementation uses kruxiaflow's REST API for proper separation:
```rust
// If embeddings_file is provided, download via HTTP streaming
// If embeddings array is provided, use directly
let embeddings = if let Some(file_ref) = params.embeddings_file {
    // Parse postgres://{workflow_id}/{activity_key}/{filename}
    let (workflow_id, activity_key, filename) = parse_file_reference(&file_ref)?;

    // Download via kruxiaflow REST API (streams the file)
    // GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
    download_embeddings_via_http(&workflow_id, &activity_key, &filename).await?
} else {
    params.embeddings
};
```

This approach:
- Maintains separation of concerns (researcher doesn't access kruxiaflow's database)
- Uses kruxiaflow's existing file download endpoint which streams from PostgreSQL Large Objects
- Processes JSON Lines as they stream in, avoiding full file buffering

## File Format: JSON Lines (JSONL)

Embeddings stored as newline-delimited JSON:

```jsonl
[0.123, -0.456, 0.789, ...]
[0.234, -0.567, 0.890, ...]
[0.345, -0.678, 0.901, ...]
```

Benefits:
- Streaming reads without loading entire file
- Standard format, easy to parse
- Each line is independently valid JSON

## Migration Path

1. **Backwards Compatible**: Small inputs still return inline embeddings
2. **Opt-in Threshold**: `streaming_threshold` parameter to control behavior
3. **Consumer Detection**: Activities check for `embeddings_file` presence

## Testing Plan

1. **Small Document**: < 1000 passages, verify inline behavior unchanged
2. **Large Document**: 5000+ passages, verify streaming to storage
3. **Memory Profiling**: Confirm peak memory stays bounded
4. **Consumer Read**: Verify store.passages reads from file reference
5. **Error Handling**: Storage failures, partial writes

## Implementation Steps

1. [x] Add `ActivityContext` struct with `workflow_id` and `storage`
2. [x] Add `execute_with_context` to `ActivityImpl` trait (with default impl)
3. [x] Update `ActivityRegistry` to pass context to activities
4. [x] Update `EmbeddingActivity` to stream large outputs
5. [x] Update researcher `store.passages` to read from file reference
6. [x] Add integration tests for large document ingestion
7. [x] Update documentation (this file)

## Test Coverage

Tests added in `worker/tests/embedding_streaming_test.rs`:
- `test_streaming_embeddings_returns_file_reference` - Verifies file reference returned
- `test_inline_embeddings_without_storage` - Verifies fallback to inline
- `test_batch_processing_streams_incrementally` - Verifies batched streaming
- `test_output_structure_has_both_keys` - Verifies template compatibility
- `test_batching_splits_large_inputs` - Verifies batch splitting (100 inputs, batch_size=25)
- `test_single_batch_for_small_inputs` - Verifies single batch for small inputs
- `test_default_batch_size` - Verifies default batch_size is applied
- `test_streaming_failure_returns_error` - Verifies error handling on failure
- `test_partial_write_on_failure` - Verifies graceful handling of partial writes
- `test_empty_input_handled` - Verifies empty input handling
- `test_large_batch_data_integrity` - Verifies data integrity across batches

## Estimated Effort

- Phase 1 (Context): 2-3 hours
- Phase 2 (Streaming): 3-4 hours
- Phase 3 (Consumer): 2-3 hours
- Testing: 2-3 hours

Total: ~10-13 hours

## Alternatives Considered

1. **Increase Container Memory**: Delays problem, doesn't solve it
2. **Smaller Batch Size**: Doesn't help - all batches still accumulated
3. **Base64 Encoding**: Reduces JSON overhead but doesn't solve memory issue
4. **External Storage (S3)**: More complex setup, PostgreSQL Large Objects sufficient
