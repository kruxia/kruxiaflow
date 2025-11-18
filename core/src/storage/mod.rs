mod error;
mod models;
mod postgres_storage;

pub use error::{Result, StorageError};
pub use models::{FileMetadata, FileReference};
pub use postgres_storage::PostgresStorage;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::Stream;
use std::pin::Pin;
use uuid::Uuid;

/// WorkflowStorage interface for file management
///
/// This interface abstracts storage operations to support multiple backends:
/// - PostgreSQL Large Objects (MVP)
/// - S3-compatible storage (Post-MVP)
/// - Filesystem storage (Post-MVP)
#[async_trait]
pub trait WorkflowStorage: Send + Sync {
    /// Upload a file (streaming, no full memory load)
    async fn upload_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
        content_type: Option<&str>,
        data: Pin<
            Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Unpin>,
        >,
    ) -> Result<FileMetadata>;

    /// Download a file (streaming, no full memory load)
    async fn download_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send>>>;

    /// Get file metadata without downloading content
    async fn get_file_metadata(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<FileMetadata>;

    /// List all files for an activity
    async fn list_files(&self, workflow_id: Uuid, activity_key: &str) -> Result<Vec<FileMetadata>>;

    /// Delete a specific file
    async fn delete_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<()>;

    /// Delete all files for a workflow (cleanup)
    async fn delete_workflow_files(&self, workflow_id: Uuid) -> Result<()>;

    /// Get a file reference (path or URL) for activity consumption
    /// This returns a reference that the activity can use to access the file
    async fn get_file_reference(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<String>;
}
