# US-5.4: Object Storage and File Management - Implementation Plan

**Epic**: Epic 5 - Built-In Activity Library
**User Story**: US-5.4
**Status**: Not Started
**Priority**: High (Required for Example 3)
**Estimated Duration**: 2-3 days
**Dependencies**: US-3.1 (Sequential Workflows) ✅ Complete

---

## User Story

**As** a data engineer
**I want** any activity to produce and consume files via object storage
**So that** I don't store large data in workflow state (JSON) and can pass files between activities

### Acceptance Criteria

- **Backend storage**: Multi-provider support (AWS S3, Google Cloud Storage, Azure Blob, MinIO, local filesystem)
- **File production**: Activities declare `outputs` with `type: file` or `type: folder`
  - Example: `outputs: [{name: "processed_data", type: file}]`
  - Files stored with path: `{workflow_id}/{activity_key}/{filename}`
  - Activity specifies filename(s) when reporting completion
- **File consumption**: Activities reference files from previous activities via template expressions
  - `{{FILE.previous_activity.filename}}` - Returns file reference/path for activity to download
  - `{{FOLDER.previous_activity.folder_name}}` - Returns folder reference/path
  - Framework automatically downloads file before activity execution (or provides path/URL to activity)
- **Lifecycle management**:
  - Files scoped to workflow_id and activity_key
  - Automatic cleanup based on workflow retention policy (e.g., delete after 30 days)
  - Files persisted until workflow retention expires
- **Implementation details**:
  - Stream large files (no full memory load)
  - Support for multiple files per activity
  - Metadata: workflow_id, activity_key, filename, size, content_type
- **Activity interface**:
  - Activities receive file paths or URLs (not inline content)
  - Activities write to provided output paths
  - Framework handles upload/download transparently
- **CRITICAL**: No special "storage activities" - file handling is a cross-cutting capability available to ALL activities

---

## Architecture Overview

### Service Interface Pattern

Following StreamFlow's service interface pattern (ActivityQueue, EventSource, etc.), we introduce a new **WorkflowStorage** interface:

```mermaid
flowchart TB
    subgraph Activities["Activity Execution"]
        Worker[Worker]
        Activity[Activity Executor]
    end

    subgraph Framework["StreamFlow Framework"]
        Orchestrator[Orchestrator]
        TemplateResolver[Template Resolver]
        StorageInterface[WorkflowStorage Interface]
    end

    subgraph Implementations["Storage Implementations"]
        PGStorage[PostgresStorage<br/>Large Objects]
        S3Storage[S3Storage<br/>Post-MVP]
        FSStorage[FilesystemStorage<br/>Post-MVP]
    end

    Worker -->|Execute Activity| Activity
    Activity -->|Upload File| StorageInterface
    Activity -->|Download File| StorageInterface

    TemplateResolver -->|Resolve FILE refs| StorageInterface
    Orchestrator -->|Cleanup| StorageInterface

    StorageInterface -.->|MVP| PGStorage
    StorageInterface -.->|Post-MVP| S3Storage
    StorageInterface -.->|Post-MVP| FSStorage

    style Framework fill:#e1f5ff
    style Implementations fill:#ffe1f5
```

---

## Implementation Tasks

### 1. Define WorkflowStorage Service Interface

**File**: `core/src/storage/mod.rs` (new)

**Purpose**: Abstract storage operations to support multiple backends (PostgreSQL Large Objects for MVP, S3/GCS/Azure post-MVP)

```rust
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::Stream;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub filename: String,
    pub size: i64,
    pub content_type: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct FileReference {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub filename: String,
}

/// WorkflowStorage interface for file management
#[async_trait]
pub trait WorkflowStorage: Send + Sync {
    /// Upload a file (streaming, no full memory load)
    async fn upload_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
        content_type: Option<&str>,
        data: impl Stream<Item = Result<Bytes, std::io::Error>> + Send,
    ) -> Result<FileMetadata>;

    /// Download a file (streaming, no full memory load)
    async fn download_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<impl Stream<Item = Result<Bytes, std::io::Error>> + Send>;

    /// Get file metadata without downloading content
    async fn get_file_metadata(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<FileMetadata>;

    /// List all files for an activity
    async fn list_files(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
    ) -> Result<Vec<FileMetadata>>;

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

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Upload failed: {0}")]
    UploadFailed(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
```

**Test Cases**:
- ✅ Interface compiles and traits are properly defined
- ✅ Error types cover all failure modes

---

### 2. Implement PostgresStorage (MVP Backend)

**File**: `core/src/storage/postgres_storage.rs` (new)

**Technology**: PostgreSQL Large Objects (LO)
- Large Objects support streaming upload/download
- Files stored in `pg_largeobject` system table
- Metadata stored in custom `workflow_files` table
- Transactional operations (file + metadata update together)

**Database Schema**: See migration `migrations/20251116000001_workflow_files.up.sql`

**Implementation**:

```rust
use sqlx::{PgPool, Postgres};
use futures::stream::{Stream, TryStreamExt};
use bytes::Bytes;

pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

#[async_trait]
impl WorkflowStorage for PostgresStorage {
    async fn upload_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
        content_type: Option<&str>,
        data: impl Stream<Item = Result<Bytes, std::io::Error>> + Send,
    ) -> Result<FileMetadata> {
        let mut tx = self.pool.begin().await?;

        // Create Large Object
        let oid: u32 = sqlx::query_scalar("SELECT lo_create(0)")
            .fetch_one(&mut *tx)
            .await?;

        // Open Large Object for writing
        let fd: i32 = sqlx::query_scalar("SELECT lo_open($1, 131072)") // 131072 = INV_WRITE
            .bind(oid as i32)
            .fetch_one(&mut *tx)
            .await?;

        // Stream write data to Large Object
        let mut total_size = 0i64;
        futures::pin_mut!(data);

        while let Some(chunk) = data.try_next().await? {
            let chunk_size = chunk.len() as i64;
            sqlx::query("SELECT lowrite($1, $2)")
                .bind(fd)
                .bind(&chunk[..])
                .execute(&mut *tx)
                .await?;
            total_size += chunk_size;
        }

        // Close Large Object
        sqlx::query("SELECT lo_close($1)")
            .bind(fd)
            .execute(&mut *tx)
            .await?;

        // Insert metadata
        let metadata: FileMetadata = sqlx::query_as(
            r#"
            INSERT INTO workflow_files
                (workflow_id, activity_key, filename, oid, size, content_type)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (workflow_id, activity_key, filename)
            DO UPDATE SET oid = $4, size = $5, content_type = $6, created_at = NOW()
            RETURNING workflow_id, activity_key, filename, size, content_type, created_at
            "#
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .bind(oid as i32)
        .bind(total_size)
        .bind(content_type)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(metadata)
    }

    async fn download_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<impl Stream<Item = Result<Bytes, std::io::Error>> + Send> {
        // Get OID from metadata
        let oid: i32 = sqlx::query_scalar(
            "SELECT oid FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3"
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::FileNotFound(filename.to_string()))?;

        // Return stream that reads from Large Object
        // Implementation uses lo_open + loread in chunks
        // (Detailed streaming implementation omitted for brevity)

        todo!("Implement streaming download from Large Object")
    }

    async fn get_file_metadata(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<FileMetadata> {
        sqlx::query_as(
            "SELECT workflow_id, activity_key, filename, size, content_type, created_at
             FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3"
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::FileNotFound(filename.to_string()))
    }

    async fn list_files(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
    ) -> Result<Vec<FileMetadata>> {
        sqlx::query_as(
            "SELECT workflow_id, activity_key, filename, size, content_type, created_at
             FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2
             ORDER BY created_at"
        )
        .bind(workflow_id)
        .bind(activity_key)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    async fn delete_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Get OID
        let oid: Option<i32> = sqlx::query_scalar(
            "SELECT oid FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3"
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(oid) = oid {
            // Delete Large Object
            sqlx::query("SELECT lo_unlink($1)")
                .bind(oid)
                .execute(&mut *tx)
                .await?;

            // Delete metadata
            sqlx::query(
                "DELETE FROM workflow_files
                 WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3"
            )
            .bind(workflow_id)
            .bind(activity_key)
            .bind(filename)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn delete_workflow_files(&self, workflow_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Get all OIDs for this workflow
        let oids: Vec<i32> = sqlx::query_scalar(
            "SELECT oid FROM workflow_files WHERE workflow_id = $1"
        )
        .bind(workflow_id)
        .fetch_all(&mut *tx)
        .await?;

        // Delete all Large Objects
        for oid in oids {
            sqlx::query("SELECT lo_unlink($1)")
                .bind(oid)
                .execute(&mut *tx)
                .await?;
        }

        // Delete all metadata
        sqlx::query("DELETE FROM workflow_files WHERE workflow_id = $1")
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn get_file_reference(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<String> {
        // For PostgreSQL storage, return an internal reference
        // Format: postgres://{workflow_id}/{activity_key}/{filename}
        Ok(format!("postgres://{}/{}/{}", workflow_id, activity_key, filename))
    }
}
```

**Test Cases**:
- ✅ Upload small file (< 1MB)
- ✅ Upload large file (> 100MB) via streaming
- ✅ Download file and verify content matches upload
- ✅ List files for activity
- ✅ Delete specific file
- ✅ Delete all workflow files
- ✅ Handle duplicate uploads (upsert behavior)
- ✅ Handle missing files gracefully

---

### 3. Extend Activity Output Definition

**File**: `core/src/activity/models.rs`

**Current State**:
```rust
pub struct ActivityOutput {
    pub name: String,
    pub value: serde_json::Value,
}
```

**New State**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputType {
    #[serde(rename = "value")]
    Value,   // Default: JSON value

    #[serde(rename = "file")]
    File,    // File reference

    #[serde(rename = "folder")]
    Folder,  // Folder reference (post-MVP)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityOutputDefinition {
    pub name: String,

    #[serde(default)]
    pub output_type: OutputType,  // Default: Value
}

pub struct ActivityOutput {
    pub name: String,
    pub output_type: OutputType,

    // For Value: JSON data
    // For File: file reference (workflow_id/activity_key/filename)
    pub value: serde_json::Value,
}
```

**YAML Example**:
```yaml
activities:
  fetch_doc:
    activity: http_request
    parameters:
      method: GET
      url: "{{INPUT.doc_url}}"
    outputs:
      - name: document
        type: file  # Declares this is a file, not JSON
```

**Test Cases**:
- ✅ Parse `type: file` in YAML
- ✅ Default to `type: value` when not specified
- ✅ Validate output types at workflow definition validation

---

### 4. Update Template Resolver for FILE References

**File**: `core/src/orchestrator/template_resolver.rs`

**New Template Expressions**:
- `{{FILE.activity_key.output_name}}` - Resolve to file reference
- `{{FOLDER.activity_key.output_name}}` - Resolve to folder reference (post-MVP)

**Implementation**:
```rust
impl TemplateResolver {
    pub async fn resolve_file_reference(
        &self,
        workflow: &WorkflowExecution,
        activity_key: &str,
        output_name: &str,
    ) -> Result<String> {
        // Get activity result
        let result = self.get_activity_result(workflow, activity_key)?;

        // Find output
        let output = result.outputs.iter()
            .find(|o| o.name == output_name)
            .ok_or_else(|| TemplateError::OutputNotFound(output_name.to_string()))?;

        // Verify it's a file output
        if output.output_type != OutputType::File {
            return Err(TemplateError::InvalidFileReference(
                format!("{}.{} is not a file output", activity_key, output_name)
            ));
        }

        // Extract file reference from value
        let file_ref = output.value.as_str()
            .ok_or_else(|| TemplateError::InvalidFileReference(
                "File reference is not a string".to_string()
            ))?;

        Ok(file_ref.to_string())
    }

    pub fn resolve_template(&self, template: &str, workflow: &WorkflowExecution) -> Result<String> {
        // Add FILE and FOLDER to template context
        let mut context = self.build_base_context(workflow);

        // Add FILE resolver
        context.insert("FILE", FileReferenceResolver {
            workflow_id: workflow.id,
            storage: self.storage.clone(),
        });

        // Use MiniJinja to resolve
        let env = self.build_jinja_env();
        let tmpl = env.template_from_str(template)?;
        Ok(tmpl.render(&context)?)
    }
}

struct FileReferenceResolver {
    workflow_id: Uuid,
    storage: Arc<dyn WorkflowStorage>,
}

impl FileReferenceResolver {
    fn get(&self, activity_key: &str, output_name: &str) -> Result<String> {
        // This is called during template resolution
        // Return file reference that activity can use
        self.storage.get_file_reference(self.workflow_id, activity_key, output_name).await
    }
}
```

**Test Cases**:
- ✅ Resolve `{{FILE.fetch_doc.document}}` to file reference
- ✅ Error if output is not a file type
- ✅ Error if output doesn't exist
- ✅ Error if activity hasn't completed

---

### 5. Update Activity Executor for File Handling

**File**: `worker/src/executor/mod.rs`

**File Upload After Activity Completion**:
When an activity completes and declares file outputs, the executor needs to:
1. Read the file from the path the activity wrote to
2. Upload to WorkflowStorage
3. Store file reference in activity result

**File Download Before Activity Execution**:
When an activity parameters reference `{{FILE.*}}`, the executor needs to:
1. Resolve file references in parameters
2. Download files from WorkflowStorage
3. Provide local paths to activity

**Implementation**:
```rust
impl ActivityExecutor {
    async fn execute_activity(&self, activity: QueuedActivity) -> Result<ActivityResult> {
        // 1. Download input files if parameters contain FILE references
        let input_files = self.download_input_files(&activity).await?;

        // 2. Execute activity with local file paths
        let result = self.run_activity_implementation(&activity, &input_files).await?;

        // 3. Upload output files if outputs are type: file
        let file_outputs = self.upload_output_files(&activity, &result).await?;

        // 4. Merge file references into result
        let final_result = self.merge_file_outputs(result, file_outputs);

        Ok(final_result)
    }

    async fn download_input_files(
        &self,
        activity: &QueuedActivity
    ) -> Result<HashMap<String, PathBuf>> {
        let mut files = HashMap::new();

        // Parse parameters for FILE references
        for file_ref in self.extract_file_references(&activity.parameters)? {
            // Download file to temp location
            let temp_path = self.download_file_to_temp(&file_ref).await?;
            files.insert(file_ref.clone(), temp_path);
        }

        Ok(files)
    }

    async fn upload_output_files(
        &self,
        activity: &QueuedActivity,
        result: &ActivityResult
    ) -> Result<Vec<ActivityOutput>> {
        let mut file_outputs = Vec::new();

        for output_def in &activity.output_definitions {
            if output_def.output_type == OutputType::File {
                // Activity should have written file to expected location
                let filename = &output_def.name;
                let file_path = self.get_activity_output_path(activity, filename);

                // Upload to storage
                let metadata = self.storage.upload_file(
                    activity.workflow_id,
                    &activity.activity_key,
                    filename,
                    None, // content_type auto-detected
                    file_stream(file_path)?
                ).await?;

                // Create output with file reference
                file_outputs.push(ActivityOutput {
                    name: filename.clone(),
                    output_type: OutputType::File,
                    value: json!(format!("{}/{}/{}",
                        activity.workflow_id,
                        activity.activity_key,
                        filename
                    )),
                });
            }
        }

        Ok(file_outputs)
    }
}
```

**Test Cases**:
- ✅ Activity with file output uploads file correctly
- ✅ Activity with file input downloads file before execution
- ✅ File paths provided to activity are valid
- ✅ Temp files cleaned up after execution
- ✅ Large files handled via streaming

---

### 6. Update http_request Activity for File Support

**File**: `worker/src/activities/http_request.rs`

**File Download (GET)**:
```yaml
fetch_doc:
  activity: http_request
  parameters:
    method: GET
    url: "https://example.com/document.pdf"
  outputs:
    - name: document
      type: file
```

**File Upload (POST with multipart/form-data)**:
```yaml
process_doc:
  activity: http_request
  parameters:
    method: POST
    url: "https://processing.example.com/api/v1/process"
    files:
      input_doc: "{{FILE.fetch_doc.document}}"
  outputs:
    - name: result
      type: file
```

**Implementation**:
```rust
async fn execute_http_request(params: HttpRequestParams) -> Result<ActivityResult> {
    match params.method.as_str() {
        "GET" => {
            // Download response to file if output is type: file
            if params.output_is_file {
                let response = reqwest::get(&params.url).await?;
                let file_path = params.output_file_path;

                // Stream response to file
                let mut file = tokio::fs::File::create(file_path).await?;
                let mut stream = response.bytes_stream();

                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    file.write_all(&chunk).await?;
                }

                Ok(ActivityResult::success(json!({})))
            } else {
                // Normal JSON response
                let response = reqwest::get(&params.url).await?;
                let json = response.json().await?;
                Ok(ActivityResult::success(json))
            }
        }

        "POST" => {
            let client = reqwest::Client::new();
            let mut request = client.post(&params.url);

            // If files are provided, use multipart/form-data
            if let Some(files) = params.files {
                let mut form = reqwest::multipart::Form::new();

                for (field_name, file_path) in files {
                    let file = tokio::fs::File::open(file_path).await?;
                    let stream = tokio_util::io::ReaderStream::new(file);
                    let part = reqwest::multipart::Part::stream(Body::wrap_stream(stream));
                    form = form.part(field_name, part);
                }

                request = request.multipart(form);
            } else {
                // Regular JSON body
                request = request.json(&params.body);
            }

            let response = request.send().await?;

            // Handle file or JSON response
            if params.output_is_file {
                // Stream to file
                todo!("Stream response to file")
            } else {
                let json = response.json().await?;
                Ok(ActivityResult::success(json))
            }
        }

        _ => Err(ActivityError::UnsupportedMethod(params.method))
    }
}
```

**Test Cases**:
- ✅ GET request downloads file
- ✅ POST request uploads file via multipart/form-data
- ✅ POST request uploads multiple files
- ✅ Large file upload/download streams correctly
- ✅ File content matches original after download

---

### 7. Workflow Lifecycle and Cleanup

**File**: `core/src/workflow/lifecycle.rs` (new or in existing workflow service)

**Automatic Cleanup**:
- When workflow is deleted or retention period expires, delete all files
- Configurable retention period (default: 30 days)

**Implementation**:
```rust
pub struct WorkflowLifecycleManager {
    storage: Arc<dyn WorkflowStorage>,
    retention_days: i64,
}

impl WorkflowLifecycleManager {
    pub async fn cleanup_expired_workflows(&self) -> Result<()> {
        // Find workflows older than retention period
        let expired_workflows = self.get_expired_workflows().await?;

        for workflow_id in expired_workflows {
            // Delete all files for workflow
            self.storage.delete_workflow_files(workflow_id).await?;

            // Delete workflow metadata
            self.delete_workflow(workflow_id).await?;
        }

        Ok(())
    }

    pub async fn delete_workflow(&self, workflow_id: Uuid) -> Result<()> {
        // Delete files first
        self.storage.delete_workflow_files(workflow_id).await?;

        // Then delete workflow record
        self.delete_workflow_record(workflow_id).await?;

        Ok(())
    }
}
```

**Test Cases**:
- ✅ Cleanup deletes all files for expired workflow
- ✅ Cleanup doesn't affect active workflows
- ✅ Manual delete removes files immediately

---

## Files to Create

### New Modules
- `core/src/storage/mod.rs` - Storage interface and exports
- `core/src/storage/error.rs` - Storage error types
- `core/src/storage/postgres_storage.rs` - PostgreSQL Large Objects implementation
- `core/src/storage/models.rs` - FileMetadata, FileReference models

### Migrations
- `migrations/20251116000001_workflow_files.up.sql` - Create workflow_files table
- `migrations/20251116000001_workflow_files.down.sql` - Drop workflow_files table

### Modified Files
- `core/src/lib.rs` - Export storage module
- `core/src/activity/models.rs` - Add OutputType enum and ActivityOutputDefinition
- `core/src/orchestrator/template_resolver.rs` - Add FILE reference resolution
- `worker/src/executor/mod.rs` - Add file upload/download handling
- `worker/src/activities/http_request.rs` - Add file download/upload support
- `api/src/main.rs` - Initialize WorkflowStorage service
- `orchestrator/src/main.rs` - Initialize WorkflowStorage service
- `worker/src/main.rs` - Initialize WorkflowStorage service

### Test Files
- `core/tests/storage_tests.rs` - Unit tests for PostgresStorage
- `core/tests/file_output_tests.rs` - Unit tests for file output definitions
- `worker/tests/file_activity_tests.rs` - Integration tests for file activities
- `api/tests/file_workflow_e2e_tests.rs` - End-to-end tests

---

## Database Migrations

### Migration Up

**File**: `migrations/20251116000001_workflow_files.up.sql`

```sql
-- File metadata table for workflow storage
CREATE TABLE workflow_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    filename TEXT NOT NULL,
    oid OID NOT NULL,  -- PostgreSQL Large Object OID
    size BIGINT NOT NULL,
    content_type TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(workflow_id, activity_key, filename)
);

-- Index for workflow file lookups (hot path)
CREATE INDEX idx_workflow_files_workflow_id
    ON workflow_files(workflow_id);

-- Index for activity file lookups
CREATE INDEX idx_workflow_files_activity
    ON workflow_files(workflow_id, activity_key);

-- Index for cleanup queries
CREATE INDEX idx_workflow_files_created
    ON workflow_files(created_at);

-- Cleanup function for old files
CREATE OR REPLACE FUNCTION cleanup_workflow_files(retention_days INTEGER DEFAULT 30)
RETURNS INTEGER AS $$
DECLARE
    deleted_count INTEGER;
    file_record RECORD;
BEGIN
    deleted_count := 0;

    -- Find and delete Large Objects for old files
    FOR file_record IN
        SELECT wf.oid
        FROM workflow_files wf
        WHERE wf.created_at < NOW() - (retention_days || ' days')::INTERVAL
    LOOP
        -- Delete the Large Object
        PERFORM lo_unlink(file_record.oid);
        deleted_count := deleted_count + 1;
    END LOOP;

    -- Delete metadata for old files
    DELETE FROM workflow_files
    WHERE created_at < NOW() - (retention_days || ' days')::INTERVAL;

    RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;
```

### Migration Down

**File**: `migrations/20251116000001_workflow_files.down.sql`

```sql
-- Drop cleanup function
DROP FUNCTION IF EXISTS cleanup_workflow_files(INTEGER);

-- Drop table (this will also drop all indexes)
-- Note: This does NOT automatically clean up Large Objects
-- Large Objects must be unlinked before dropping the table to avoid orphans
DO $$
DECLARE
    file_record RECORD;
BEGIN
    -- Delete all Large Objects before dropping table
    FOR file_record IN SELECT oid FROM workflow_files
    LOOP
        PERFORM lo_unlink(file_record.oid);
    END LOOP;
END $$;

DROP TABLE IF EXISTS workflow_files;
```

**Notes**:
- Migration uses `uuidv7()` for id generation (requires PostgreSQL 18+)
- Large Objects are stored separately in `pg_largeobject` system catalog
- Down migration properly cleans up Large Objects to prevent orphaned data
- No foreign key constraint to workflows table (allows independent lifecycle for MVP)

---

## Testing Strategy

### Unit Tests

**Storage Interface**:
- Upload/download file
- List files
- Delete file
- Get metadata
- Streaming large files

**Output Types**:
- Parse file output definitions from YAML
- Validate output types
- Serialize/deserialize

**Template Resolution**:
- Resolve FILE references
- Error handling for invalid references

### Integration Tests

**File Activity Workflow**:
- Workflow with file download activity
- Workflow with file upload activity
- Workflow with file pass-through (download → process → upload)

### End-to-End Tests

**Example 3 Workflow**:
- Multi-document processing pipeline
- Parallel file downloads
- Parallel file processing
- File aggregation

---

## Success Criteria

- ✅ WorkflowStorage interface defined and PostgresStorage implemented
- ✅ Files uploaded/downloaded via streaming (no full memory load)
- ✅ File outputs declared in YAML with `type: file`
- ✅ Template expressions `{{FILE.activity.output}}` resolve correctly
- ✅ http_request activity supports file download (GET) and upload (POST)
- ✅ Large files (>100MB) handled efficiently
- ✅ File cleanup on workflow deletion
- ✅ All tests pass
- ✅ Example 3 workflow runs end-to-end

---

## Non-Goals (Post-MVP)

- ❌ S3/GCS/Azure storage backends (MVP: PostgreSQL only)
- ❌ Folder outputs (`type: folder`)
- ❌ File versioning
- ❌ File compression
- ❌ File encryption at rest
- ❌ Pre-signed URLs for direct client upload
- ❌ CDN integration for file delivery

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| PostgreSQL Large Objects performance | Medium | Use streaming, benchmark with large files |
| Large Object OID cleanup | High | Transactional delete, cleanup function |
| File reference resolution complexity | Medium | Comprehensive template tests |
| Activity file path conventions | Medium | Clear documentation, standard patterns |

---

## Dependencies

**Upstream**:
- ✅ US-3.1: Sequential Workflows

**Downstream**:
- 🔲 Example 3: Multi-Document Processing Pipeline (requires both US-3.3 and US-5.4)

**Parallel Work**:
- 🔲 US-3.3: Parallel Execution (can be developed in parallel)

---

## Implementation Phases

### Phase 1: Storage Interface and PostgreSQL Backend (Days 1-2)
1. Define WorkflowStorage interface
2. Implement PostgresStorage with Large Objects
3. Database migrations
4. Unit tests for storage operations

### Phase 2: File Output Definitions and Template Resolution (Day 2)
1. Add OutputType enum
2. Update YAML parsing for `type: file`
3. Implement FILE reference resolution in templates
4. Unit tests

### Phase 3: Activity Executor File Handling (Day 2-3)
1. File upload after activity completion
2. File download before activity execution
3. Update http_request activity for files
4. Integration tests

### Phase 4: Integration and Example 3 (Day 3)
1. End-to-end file workflow tests
2. Example 3 implementation (with US-3.3)
3. Performance testing with large files
4. Documentation

---

## Completion Checklist

- [ ] WorkflowStorage interface defined
- [ ] PostgresStorage implemented with Large Objects
- [ ] Database schema created and migrated
- [ ] OutputType enum added
- [ ] YAML parsing supports `type: file`
- [ ] Template resolver supports {{FILE.*}} expressions
- [ ] Activity executor handles file upload/download
- [ ] http_request activity supports file operations
- [ ] Streaming works for large files (>100MB)
- [ ] File cleanup on workflow deletion
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] End-to-end tests pass (Example 3)
- [ ] Documentation updated
- [ ] Code review complete
