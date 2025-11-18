use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use streamflow_core::storage::WorkflowStorage;
use streamflow_core::{ActivityOutput, ActivityOutputDefinition, OutputType};
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
            .join("streamflow")
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
                    if let Some(map) = outputs_map {
                        if let Some(value) = map.get(&output_def.name) {
                            outputs.push(ActivityOutput::value(
                                output_def.name.clone(),
                                value.clone(),
                            ));
                        }
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
    use sqlx::PgPool;
    use streamflow_core::storage::PostgresStorage;

    async fn setup_test_storage() -> Arc<dyn WorkflowStorage> {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@127.0.0.1:5432/streamflow".to_string()
        });

        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        Arc::new(PostgresStorage::new(pool))
    }

    #[tokio::test]
    async fn test_file_executor_creates_temp_dir() {
        let storage = setup_test_storage().await;
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity".to_string();

        let executor = FileExecutor::new(workflow_id, activity_key.clone(), storage)
            .await
            .expect("Failed to create file executor");

        assert!(executor.temp_dir().exists());

        // Cleanup
        executor.cleanup().await.expect("Failed to cleanup");
    }

    #[tokio::test]
    async fn test_file_executor_output_path() {
        let storage = setup_test_storage().await;
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

    #[tokio::test]
    async fn test_process_file_outputs_with_value() {
        let storage = setup_test_storage().await;
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
        assert_eq!(outputs[0].output_type, OutputType::Value);

        // Cleanup
        executor.cleanup().await.expect("Failed to cleanup");
    }
}
