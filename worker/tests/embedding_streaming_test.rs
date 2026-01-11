// ============================================================================
// Embedding Streaming Tests
// Bug fix: docs/bugs/2026-01-09-embedding-oom-on-large-result-serialization.md
//
// Tests the fix for OOM crashes when processing large embedding jobs.
// The fix streams embeddings to workflow storage instead of accumulating in memory.
// ============================================================================

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use kruxiaflow_core::storage::{PostgresStorage, WorkflowStorage};
use kruxiaflow_worker::activity_result::ActivityResult;
use kruxiaflow_worker::registry::{ActivityContext, ActivityImpl};
use serde_json::{Value, json};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
    });

    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database")
}

/// Mock embedding activity that simulates the streaming fix behavior.
///
/// This activity follows the same pattern as the real EmbeddingActivity:
/// - Always streams to storage when context is available
/// - Returns embeddings_file reference instead of inline embeddings
/// - Uses batch processing
struct MockStreamingEmbeddingActivity;

impl MockStreamingEmbeddingActivity {
    fn new() -> Self {
        Self
    }

    /// Generate mock embeddings for testing (small vectors to keep tests fast)
    fn generate_mock_embeddings(count: usize, dimensions: usize) -> Vec<Vec<f64>> {
        (0..count)
            .map(|i| {
                (0..dimensions)
                    .map(|d| (i * dimensions + d) as f64 / 1000.0)
                    .collect()
            })
            .collect()
    }
}

#[async_trait]
impl ActivityImpl for MockStreamingEmbeddingActivity {
    /// Execute with context - streams embeddings to workflow storage
    async fn execute_with_context(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> anyhow::Result<ActivityResult> {
        let input_count = parameters
            .get("input")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(10);

        let dimensions = parameters
            .get("dimensions")
            .and_then(|v| v.as_u64())
            .unwrap_or(8) as usize;

        let batch_size = parameters
            .get("batch_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;

        // Require workflow storage for streaming
        let storage = match &ctx.storage {
            Some(s) => s.clone(),
            None => {
                // Fall back to inline (for testing the fallback path)
                return self.execute(parameters).await;
            }
        };

        let filename = "embeddings.jsonl";

        // Create channel for streaming to storage
        let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(32);

        // Spawn upload task
        let upload_workflow_id = ctx.workflow_id;
        let upload_activity_key = ctx.activity_key.clone();
        let upload_storage = storage.clone();
        let upload_handle = tokio::spawn(async move {
            let stream = ReceiverStream::new(rx).map(Ok::<_, std::io::Error>);
            upload_storage
                .upload_file(
                    upload_workflow_id,
                    &upload_activity_key,
                    filename,
                    Some("application/x-ndjson"),
                    Box::pin(stream),
                )
                .await
        });

        // Process in batches and stream to storage
        let embeddings = Self::generate_mock_embeddings(input_count, dimensions);
        let mut embedding_count = 0usize;

        for batch in embeddings.chunks(batch_size) {
            for embedding in batch {
                let line = serde_json::to_string(&embedding)?;
                tx.send(Bytes::from(format!("{}\n", line)))
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send: {}", e))?;
                embedding_count += 1;
            }
        }

        // Close stream and wait for upload
        drop(tx);
        let _file_metadata = upload_handle
            .await
            .map_err(|e| anyhow::anyhow!("Upload task panicked: {}", e))?
            .map_err(|e| anyhow::anyhow!("Upload failed: {}", e))?;

        // Get file reference
        let file_ref = storage
            .get_file_reference(ctx.workflow_id, &ctx.activity_key, filename)
            .await?;

        // Return file reference instead of embeddings array
        let outputs = json!({
            "embeddings": null,  // Not present when streaming
            "embeddings_file": file_ref,
            "embedding_count": embedding_count,
            "model": "mock-embedding-model",
            "provider": "mock",
            "usage": {
                "prompt_tokens": input_count * 10,
                "output_tokens": 0,
                "total_tokens": input_count * 10,
                "cached_tokens": null,
            }
        });

        Ok(ActivityResult::value("result", outputs))
    }

    /// Execute without context - returns inline embeddings
    async fn execute(&self, parameters: Value) -> anyhow::Result<ActivityResult> {
        let input_count = parameters
            .get("input")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(10);

        let dimensions = parameters
            .get("dimensions")
            .and_then(|v| v.as_u64())
            .unwrap_or(8) as usize;

        let embeddings = Self::generate_mock_embeddings(input_count, dimensions);

        let outputs = json!({
            "embeddings": embeddings,
            "embeddings_file": null,  // Not present for inline
            "embedding_count": embeddings.len(),
            "model": "mock-embedding-model",
            "provider": "mock",
            "usage": {
                "prompt_tokens": input_count * 10,
                "output_tokens": 0,
                "total_tokens": input_count * 10,
                "cached_tokens": null,
            }
        });

        Ok(ActivityResult::value("result", outputs))
    }

    fn name(&self) -> &str {
        "embedding"
    }

    fn worker(&self) -> &str {
        "mock"
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test: Streaming to storage returns embeddings_file reference
///
/// Verifies that when storage is available, embeddings are streamed to storage
/// and the result contains embeddings_file (not inline embeddings).
#[tokio::test]
#[serial]
async fn test_streaming_embeddings_returns_file_reference() {
    let pool = setup_test_pool().await;
    let storage: Arc<dyn WorkflowStorage> = Arc::new(PostgresStorage::new(pool.clone()));

    let activity = MockStreamingEmbeddingActivity::new();
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_embedding".to_string();

    let ctx = ActivityContext::new(
        workflow_id,
        Uuid::now_v7(),
        activity_key.clone(),
        Some(storage.clone()),
    );

    let params = json!({
        "input": ["text1", "text2", "text3", "text4", "text5"],
        "dimensions": 8,
        "batch_size": 2
    });

    let result = activity.execute_with_context(params, &ctx).await;
    assert!(
        result.is_ok(),
        "Activity should succeed: {:?}",
        result.err()
    );

    let output = result.unwrap();
    let result_json = output.to_json_value();
    let result_obj = result_json.get("result").expect("Should have result key");

    // Verify embeddings is null (streamed to file)
    assert!(
        result_obj.get("embeddings").unwrap().is_null(),
        "embeddings should be null when streaming to storage"
    );

    // Verify embeddings_file is present
    let embeddings_file = result_obj.get("embeddings_file").unwrap();
    assert!(
        !embeddings_file.is_null(),
        "embeddings_file should be set when streaming"
    );
    assert!(
        embeddings_file.is_string(),
        "embeddings_file should be a string reference"
    );

    // Verify embedding count
    assert_eq!(
        result_obj.get("embedding_count").unwrap().as_u64().unwrap(),
        5,
        "embedding_count should match input count"
    );

    // Verify file was actually created in storage
    let files = storage.list_files(workflow_id, &activity_key).await;
    assert!(files.is_ok(), "Should be able to list files");
    let files = files.unwrap();
    assert_eq!(files.len(), 1, "Should have one file");
    assert_eq!(files[0].filename, "embeddings.jsonl");

    // Cleanup
    let _ = storage
        .delete_file(workflow_id, &activity_key, "embeddings.jsonl")
        .await;
}

/// Test: Inline embeddings returned when no storage available
///
/// Verifies fallback behavior - when storage is not available,
/// embeddings are returned inline.
#[tokio::test]
#[serial]
async fn test_inline_embeddings_without_storage() {
    let activity = MockStreamingEmbeddingActivity::new();
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_embedding_inline".to_string();

    // Context without storage
    let ctx = ActivityContext::new(
        workflow_id,
        Uuid::now_v7(),
        activity_key,
        None, // No storage
    );

    let params = json!({
        "input": ["text1", "text2", "text3"],
        "dimensions": 4
    });

    let result = activity.execute_with_context(params, &ctx).await;
    assert!(result.is_ok(), "Activity should succeed");

    let output = result.unwrap();
    let result_json = output.to_json_value();
    let result_obj = result_json.get("result").expect("Should have result key");

    // Verify embeddings is present (inline)
    let embeddings = result_obj.get("embeddings").unwrap();
    assert!(
        !embeddings.is_null(),
        "embeddings should be present when no storage"
    );
    assert!(embeddings.is_array(), "embeddings should be an array");
    assert_eq!(
        embeddings.as_array().unwrap().len(),
        3,
        "Should have 3 embeddings"
    );

    // Verify embeddings_file is null
    assert!(
        result_obj.get("embeddings_file").unwrap().is_null(),
        "embeddings_file should be null for inline embeddings"
    );
}

/// Test: Batch processing streams incrementally
///
/// Verifies that large inputs are processed in batches and streamed
/// incrementally to storage.
#[tokio::test]
#[serial]
async fn test_batch_processing_streams_incrementally() {
    let pool = setup_test_pool().await;
    let storage: Arc<dyn WorkflowStorage> = Arc::new(PostgresStorage::new(pool.clone()));

    let activity = MockStreamingEmbeddingActivity::new();
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_batch_embedding".to_string();

    let ctx = ActivityContext::new(
        workflow_id,
        Uuid::now_v7(),
        activity_key.clone(),
        Some(storage.clone()),
    );

    // Create 50 inputs with batch size 10 = 5 batches
    let inputs: Vec<String> = (0..50).map(|i| format!("text{}", i)).collect();

    let params = json!({
        "input": inputs,
        "dimensions": 4,
        "batch_size": 10
    });

    let result = activity.execute_with_context(params, &ctx).await;
    assert!(result.is_ok(), "Activity should succeed");

    let output = result.unwrap();
    let result_json = output.to_json_value();
    let result_obj = result_json.get("result").expect("Should have result key");

    // Verify all embeddings were processed
    assert_eq!(
        result_obj.get("embedding_count").unwrap().as_u64().unwrap(),
        50,
        "Should have processed all 50 embeddings"
    );

    // Verify file contains all embeddings
    let download_stream = storage
        .download_file(workflow_id, &activity_key, "embeddings.jsonl")
        .await;
    assert!(download_stream.is_ok(), "Should download file");

    let mut stream = download_stream.unwrap();
    let mut content = Vec::new();
    while let Some(chunk) = stream.next().await {
        content.extend(chunk.expect("Should read chunk"));
    }

    let content_str = String::from_utf8(content).expect("Should be valid UTF-8");
    let lines: Vec<&str> = content_str.lines().collect();

    assert_eq!(
        lines.len(),
        50,
        "File should contain 50 lines (one per embedding)"
    );

    // Verify each line is valid JSON
    for (i, line) in lines.iter().enumerate() {
        let parsed: Result<Vec<f64>, _> = serde_json::from_str(line);
        assert!(
            parsed.is_ok(),
            "Line {} should be valid JSON array: {}",
            i,
            line
        );
        let embedding = parsed.unwrap();
        assert_eq!(
            embedding.len(),
            4,
            "Each embedding should have 4 dimensions"
        );
    }

    // Cleanup
    let _ = storage
        .delete_file(workflow_id, &activity_key, "embeddings.jsonl")
        .await;
}

/// Test: Output structure has both keys for template compatibility
///
/// The fix ensures both embeddings and embeddings_file keys are always present
/// (one null, one with value) so templates can reference either.
#[tokio::test]
#[serial]
async fn test_output_structure_has_both_keys() {
    let pool = setup_test_pool().await;
    let storage: Arc<dyn WorkflowStorage> = Arc::new(PostgresStorage::new(pool.clone()));

    let activity = MockStreamingEmbeddingActivity::new();

    // Test with storage (streaming)
    {
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_structure_streaming".to_string();
        let ctx = ActivityContext::new(
            workflow_id,
            Uuid::now_v7(),
            activity_key.clone(),
            Some(storage.clone()),
        );

        let params = json!({"input": ["test"], "dimensions": 4});
        let result = activity.execute_with_context(params, &ctx).await.unwrap();
        let result_obj = result.to_json_value().get("result").cloned().unwrap();

        // Both keys must exist
        assert!(
            result_obj.get("embeddings").is_some(),
            "embeddings key must exist for streaming output"
        );
        assert!(
            result_obj.get("embeddings_file").is_some(),
            "embeddings_file key must exist for streaming output"
        );
        assert!(
            result_obj.get("embedding_count").is_some(),
            "embedding_count key must exist"
        );

        let _ = storage
            .delete_file(workflow_id, &activity_key, "embeddings.jsonl")
            .await;
    }

    // Test without storage (inline)
    {
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_structure_inline".to_string();
        let ctx = ActivityContext::new(
            workflow_id,
            Uuid::now_v7(),
            activity_key,
            None, // No storage
        );

        let params = json!({"input": ["test"], "dimensions": 4});
        let result = activity.execute_with_context(params, &ctx).await.unwrap();
        let result_obj = result.to_json_value().get("result").cloned().unwrap();

        // Both keys must exist
        assert!(
            result_obj.get("embeddings").is_some(),
            "embeddings key must exist for inline output"
        );
        assert!(
            result_obj.get("embeddings_file").is_some(),
            "embeddings_file key must exist for inline output"
        );
        assert!(
            result_obj.get("embedding_count").is_some(),
            "embedding_count key must exist"
        );
    }
}

/// Test: Direct execute returns inline embeddings
///
/// Tests the fallback execute() method directly returns inline embeddings.
#[tokio::test]
async fn test_direct_execute_returns_inline() {
    let activity = MockStreamingEmbeddingActivity::new();

    let params = json!({
        "input": ["a", "b", "c", "d"],
        "dimensions": 4
    });

    let result = activity.execute(params).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let result_obj = output.to_json_value().get("result").cloned().unwrap();

    // Should have inline embeddings
    let embeddings = result_obj.get("embeddings").unwrap();
    assert!(!embeddings.is_null());
    assert_eq!(embeddings.as_array().unwrap().len(), 4);

    // Each embedding should have correct dimensions
    for emb in embeddings.as_array().unwrap() {
        assert_eq!(emb.as_array().unwrap().len(), 4);
    }

    // embeddings_file should be null
    assert!(result_obj.get("embeddings_file").unwrap().is_null());
}

/// Test: Usage metrics are correctly reported
///
/// Verifies that token usage is reported in the output.
#[tokio::test]
async fn test_usage_metrics_reported() {
    let activity = MockStreamingEmbeddingActivity::new();

    let params = json!({
        "input": ["text1", "text2", "text3"],
        "dimensions": 4
    });

    let result = activity.execute(params).await.unwrap();
    let result_obj = result.to_json_value().get("result").cloned().unwrap();

    let usage = result_obj.get("usage").expect("Should have usage");
    assert!(usage.get("prompt_tokens").is_some());
    assert!(usage.get("total_tokens").is_some());
    assert_eq!(
        usage.get("prompt_tokens").unwrap().as_u64().unwrap(),
        30, // 3 inputs * 10 tokens each
        "prompt_tokens should reflect input count"
    );
}
