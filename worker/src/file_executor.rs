use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use kruxiaflow_core::storage::WorkflowStorage;
use kruxiaflow_core::{ActivityOutputDefinition, OutputType};
use kruxiaflow_worker::ActivityOutput;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// File execution context for an activity
///
/// Handles file downloads before execution and uploads after execution
pub struct FileExecutor {
    /// Workflow ID
    workflow_id: Uuid,

    /// Activity key
    activity_key: String,

    /// Temporary directory for this activity's files
    temp_dir: PathBuf,

    /// Storage backend for file operations
    storage: Arc<dyn WorkflowStorage>,
}

impl FileExecutor {
    /// Create a new file executor for an activity
    pub async fn new(
        workflow_id: Uuid,
        activity_key: String,
        storage: Arc<dyn WorkflowStorage>,
    ) -> Result<Self> {
        // Create temp directory for this activity
        let temp_dir = std::env::temp_dir()
            .join("kruxiaflow")
            .join(workflow_id.to_string())
            .join(&activity_key);

        fs::create_dir_all(&temp_dir)
            .await
            .context("Failed to create temp directory")?;

        Ok(Self {
            workflow_id,
            activity_key,
            temp_dir,
            storage,
        })
    }

    /// Get the path for an output file
    pub fn output_file_path(&self, filename: &str) -> PathBuf {
        self.temp_dir.join(filename)
    }

    /// Get the temporary directory path
    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    /// Check if output file exists
    pub async fn output_file_exists(&self, filename: &str) -> bool {
        self.output_file_path(filename).exists()
    }

    /// Download a file from storage to temp directory
    ///
    /// File reference format: {workflow_id}/{activity_key}/{filename}
    pub async fn download_file(&self, file_ref: &str, local_filename: &str) -> Result<PathBuf> {
        // Parse file reference
        let parts: Vec<&str> = file_ref.split('/').collect();
        if parts.len() != 3 {
            anyhow::bail!("Invalid file reference format: {}", file_ref);
        }

        let source_workflow_id =
            Uuid::parse_str(parts[0]).context("Invalid workflow ID in file reference")?;
        let source_activity_key = parts[1];
        let source_filename = parts[2];

        // Download from storage
        let mut stream = self
            .storage
            .download_file(source_workflow_id, source_activity_key, source_filename)
            .await
            .context("Failed to download file from storage")?;

        // Write to local file
        let local_path = self.output_file_path(local_filename);
        let mut file = fs::File::create(&local_path)
            .await
            .context("Failed to create local file")?;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read chunk from storage")?;
            file.write_all(&chunk)
                .await
                .context("Failed to write chunk to local file")?;
        }

        file.sync_all().await.context("Failed to sync local file")?;

        Ok(local_path)
    }

    /// Upload a file from temp directory to storage
    pub async fn upload_file(&self, filename: &str, content_type: Option<&str>) -> Result<String> {
        let file_path = self.output_file_path(filename);

        if !file_path.exists() {
            anyhow::bail!("File not found: {}", filename);
        }

        // Read file and create stream
        let file_content = fs::read(&file_path).await.context("Failed to read file")?;

        let chunks = vec![Ok(Bytes::from(file_content))];
        let stream = futures::stream::iter(chunks);
        let stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = std::io::Result<Bytes>> + Send + Unpin>,
        > = Box::pin(stream);

        // Upload to storage
        let metadata = self
            .storage
            .upload_file(
                self.workflow_id,
                &self.activity_key,
                filename,
                content_type,
                stream,
            )
            .await
            .context("Failed to upload file to storage")?;

        // Generate file reference
        let file_ref = format!("{}/{}/{}", self.workflow_id, self.activity_key, filename);

        tracing::debug!(
            "Uploaded file {} ({} bytes) to storage",
            filename,
            metadata.size
        );

        Ok(file_ref)
    }

    /// Process file outputs after activity execution
    ///
    /// Uploads files to storage and returns ActivityOutput structs with file references
    pub async fn process_file_outputs(
        &self,
        output_definitions: &[ActivityOutputDefinition],
        activity_outputs: Value,
    ) -> Result<Vec<ActivityOutput>> {
        let mut outputs = Vec::new();

        // Parse activity outputs if it's an object
        let outputs_map = if let Value::Object(map) = &activity_outputs {
            Some(map)
        } else {
            None
        };

        for output_def in output_definitions {
            match output_def.output_type {
                OutputType::Value => {
                    // Regular value output - extract from activity_outputs
                    if let Some(map) = outputs_map
                        && let Some(value) = map.get(&output_def.name)
                    {
                        outputs.push(ActivityOutput::value(
                            output_def.name.clone(),
                            value.clone(),
                        ));
                    }
                }
                OutputType::File => {
                    // File output - upload to storage
                    let filename = &output_def.name;
                    let file_path = self.output_file_path(filename);

                    if file_path.exists() {
                        // Upload file to storage
                        let file_ref = self
                            .upload_file(filename, None)
                            .await
                            .context(format!("Failed to upload file output: {}", filename))?;

                        outputs.push(ActivityOutput::file(output_def.name.clone(), file_ref));
                    } else {
                        // File not created - this is an error
                        anyhow::bail!("Activity did not create expected file output: {}", filename);
                    }
                }
                OutputType::Folder => {
                    // Folder output (post-MVP) - not implemented yet
                    anyhow::bail!("Folder outputs are not yet supported (post-MVP feature)");
                }
            }
        }

        Ok(outputs)
    }

    /// Cleanup temporary files
    pub async fn cleanup(&self) -> Result<()> {
        if self.temp_dir.exists() {
            fs::remove_dir_all(&self.temp_dir)
                .await
                .context("Failed to cleanup temp directory")?;
        }
        Ok(())
    }
}

/// Create output file for activity to write to
pub async fn create_output_file(path: &Path, content: &[u8]) -> Result<()> {
    let mut file = fs::File::create(path)
        .await
        .context("Failed to create output file")?;

    file.write_all(content)
        .await
        .context("Failed to write to output file")?;

    file.sync_all().await.context("Failed to sync file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kruxiaflow_core::storage::PostgresStorage;
    use sqlx::PgPool;

    fn test_storage(pool: PgPool) -> Arc<dyn WorkflowStorage> {
        Arc::new(PostgresStorage::new(pool))
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_file_executor_creates_temp_dir(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key.clone(), storage)
            .await
            .expect("Failed to create file executor");

        assert!(executor.temp_dir().exists());

        // Cleanup
        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_file_executor_output_path(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let output_path = executor.output_file_path("test.txt");
        assert!(output_path.ends_with("test.txt"));

        // Cleanup
        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_process_file_outputs_with_value(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let output_defs = vec![ActivityOutputDefinition {
            name: "result".to_string(),
            output_type: OutputType::Value,
        }];

        let activity_outputs = serde_json::json!({
            "result": {"status": "success"}
        });

        let outputs = executor
            .process_file_outputs(&output_defs, activity_outputs)
            .await
            .expect("Failed to process outputs");

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "result");
        assert_eq!(outputs[0].output_type, kruxiaflow_worker::OutputType::Value);

        // Cleanup
        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_process_file_outputs_non_object_value(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let output_defs = vec![ActivityOutputDefinition {
            name: "result".to_string(),
            output_type: OutputType::Value,
        }];

        // Non-object value (array) — value outputs won't be found
        let activity_outputs = serde_json::json!([1, 2, 3]);

        let outputs = executor
            .process_file_outputs(&output_defs, activity_outputs)
            .await
            .expect("Should succeed but with no outputs");

        assert_eq!(outputs.len(), 0);

        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_process_file_outputs_folder_type_errors(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let output_defs = vec![ActivityOutputDefinition {
            name: "results_dir".to_string(),
            output_type: OutputType::Folder,
        }];

        let activity_outputs = serde_json::json!({});

        let result = executor
            .process_file_outputs(&output_defs, activity_outputs)
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not yet supported")
        );

        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_process_file_outputs_missing_file(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let output_defs = vec![ActivityOutputDefinition {
            name: "report.pdf".to_string(),
            output_type: OutputType::File,
        }];

        let activity_outputs = serde_json::json!({});

        let result = executor
            .process_file_outputs(&output_defs, activity_outputs)
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("did not create expected file output")
        );

        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_download_file_invalid_reference_format(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        // Invalid format: missing parts
        let result = executor.download_file("just-one-part", "local.txt").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid file reference format")
        );

        // Invalid format: too many parts
        let result = executor.download_file("a/b/c/d", "local.txt").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid file reference format")
        );

        // Invalid UUID
        let result = executor
            .download_file("not-a-uuid/activity/file.txt", "local.txt")
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid workflow ID")
        );

        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_output_file_exists(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        // File doesn't exist yet
        assert!(!executor.output_file_exists("nonexistent.txt").await);

        // Create a file
        let path = executor.output_file_path("test_output.txt");
        create_output_file(&path, b"hello world").await.unwrap();

        // Now it exists
        assert!(executor.output_file_exists("test_output.txt").await);

        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[tokio::test]
    async fn test_create_output_file() {
        let temp_dir = std::env::temp_dir()
            .join("kruxiaflow_test")
            .join(Uuid::now_v7().to_string());
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("test_output.txt");
        create_output_file(&file_path, b"test content")
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "test content");

        // Cleanup
        tokio::fs::remove_dir_all(&temp_dir).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_upload_file_not_found(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let result = executor.upload_file("nonexistent.txt", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("File not found"));

        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_cleanup_nonexistent_dir(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_cleanup".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        // Remove dir first
        let _ = tokio::fs::remove_dir_all(executor.temp_dir()).await;

        // Cleanup of non-existent dir should succeed
        executor
            .cleanup()
            .await
            .expect("Cleanup should succeed even if dir is gone");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_process_file_outputs_value_missing_key(pool: PgPool) {
        let storage = test_storage(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key, storage)
            .await
            .expect("Failed to create file executor");

        let output_defs = vec![ActivityOutputDefinition {
            name: "missing_key".to_string(),
            output_type: OutputType::Value,
        }];

        // Object that doesn't contain the expected key
        let activity_outputs = serde_json::json!({"other_key": 42});

        let outputs = executor
            .process_file_outputs(&output_defs, activity_outputs)
            .await
            .expect("Should succeed but with no matching outputs");

        assert_eq!(outputs.len(), 0);

        executor.cleanup().await.expect("Failed to cleanup");
    }
}
