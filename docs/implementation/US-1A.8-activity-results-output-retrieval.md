# US-1A.8: Activity Results and Output Retrieval

## User Story

**As** an AI researcher
**I want** to retrieve activity outputs and workflow results via API
**So that** I can access computation results for downstream processing

## Status

✅ **Implemented** - 2025-12-05

## Background

Currently, activity outputs are accessible through the workflow status endpoint (`GET /api/v1/workflows/{workflow_id}`), which returns the full workflow state including all activity outputs in the `activities` array. However, this approach has limitations:

1. **Large responses**: For workflows with many activities, returning all activity data is inefficient
2. **No direct access**: Clients must parse the full workflow response to find specific activity outputs
3. **No file streaming**: Large file outputs stored in `workflow_files` have no direct download endpoint
4. **No cost information**: Activity cost information is stored separately in `activity_costs` table but not exposed

This feature adds dedicated endpoints for retrieving individual activity outputs and final workflow results.

## Acceptance Criteria

- `GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output` - Get activity output
  - Response includes: `{activity_key, output, cost_usd, completed_at}`
  - Large outputs: Return reference to artifact storage with signed URL
  - 404 if activity not completed or doesn't exist
- `GET /api/v1/workflows/{workflow_id}/output` - Get final workflow output
  - Output format: JSON with activity outputs accessible by key

## Technical Design

### 1. New API Endpoints

#### 1.1 Get Activity Output

```
GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output
```

**Response (200 OK)**:
```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "activity_key": "analyze_document",
  "status": "completed",
  "output": {
    "summary": "Document analysis complete",
    "categories": ["finance", "legal"],
    "confidence": 0.95
  },
  "cost_usd": "0.0023",
  "completed_at": "2025-11-27T10:30:00Z",
  "files": [
    {
      "filename": "analysis_report.pdf",
      "size": 102400,
      "content_type": "application/pdf",
      "download_url": "/api/v1/workflows/{workflow_id}/activities/{activity_key}/files/analysis_report.pdf"
    }
  ]
}
```

**Error Responses**:
- `404 Not Found`: Workflow or activity doesn't exist
- `400 Bad Request`: Activity not yet completed (status != completed)

#### 1.2 Get Workflow Output (Final Results)

```
GET /api/v1/workflows/{workflow_id}/output
```

Returns aggregated outputs from all completed activities, with terminal activities (those with no dependents) highlighted as "final outputs".

**Response (200 OK)**:
```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "completed",
  "total_cost_usd": "0.0145",
  "completed_at": "2025-11-27T10:35:00Z",
  "outputs": {
    "validate_input": {
      "status": "completed",
      "output": {"valid": true},
      "cost_usd": "0.0000",
      "completed_at": "2025-11-27T10:30:00Z"
    },
    "analyze_document": {
      "status": "completed",
      "output": {"summary": "..."},
      "cost_usd": "0.0023",
      "completed_at": "2025-11-27T10:32:00Z"
    },
    "generate_report": {
      "status": "completed",
      "output": {"report_url": "..."},
      "cost_usd": "0.0122",
      "completed_at": "2025-11-27T10:35:00Z",
      "is_terminal": true
    }
  },
  "terminal_outputs": ["generate_report"]
}
```

**Error Responses**:
- `404 Not Found`: Workflow doesn't exist
- `400 Bad Request`: Workflow not yet completed

#### 1.3 Download Activity File

```
GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
```

Streams the file content directly for download.

**Response (200 OK)**:
- `Content-Type`: File's content type
- `Content-Disposition`: `attachment; filename="{filename}"`
- Body: Binary file content (streamed)

**Error Responses**:
- `404 Not Found`: File doesn't exist

### 2. Database Queries

#### 2.1 Get Activity Output Query

```sql
-- Get activity output from workflow state and cost from activity_costs
SELECT
    w.id AS workflow_id,
    w.status AS workflow_status,
    w.activities->>$2 AS activity_data,
    COALESCE(
        (SELECT SUM(cost_usd)
         FROM activity_costs
         WHERE workflow_id = $1 AND activity_key = $2),
        0.0
    ) AS cost_usd
FROM workflows w
WHERE w.id = $1
```

Then parse `activity_data` JSON to extract:
- `status`
- `outputs`
- `completed_at`

#### 2.2 Get Activity Files Query

```sql
SELECT
    filename,
    size,
    content_type
FROM workflow_files
WHERE workflow_id = $1
  AND activity_key = $2
```

### 3. Implementation Components

#### 3.1 New Service: `OutputQueryService`

Located in `core/src/workflow/output_query_service.rs`:

```rust
pub struct ActivityOutput {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub status: WorkflowActivityStatus,
    pub output: Option<serde_json::Value>,
    pub cost_usd: Decimal,
    pub completed_at: Option<DateTime<Utc>>,
    pub files: Vec<FileInfo>,
}

pub struct WorkflowOutput {
    pub workflow_id: Uuid,
    pub status: WorkflowStatus,
    pub total_cost_usd: Decimal,
    pub completed_at: Option<DateTime<Utc>>,
    pub outputs: HashMap<String, ActivityOutputSummary>,
    pub terminal_outputs: Vec<String>,
}

pub struct OutputQueryService {
    pool: PgPool,
}

impl OutputQueryService {
    pub async fn get_activity_output(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
    ) -> Result<ActivityOutput, OutputQueryError>;

    pub async fn get_workflow_output(
        &self,
        workflow_id: Uuid,
    ) -> Result<WorkflowOutput, OutputQueryError>;
}
```

#### 3.2 New Handler Functions

Located in `api/src/handlers/outputs.rs`:

```rust
/// GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output
pub async fn get_activity_output(
    service: OutputQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Path((workflow_id, activity_key)): Path<(Uuid, String)>,
) -> ApiResult<Json<GetActivityOutputResponse>>;

/// GET /api/v1/workflows/{workflow_id}/output
pub async fn get_workflow_output(
    service: OutputQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Path(workflow_id): Path<Uuid>,
) -> ApiResult<Json<GetWorkflowOutputResponse>>;

/// GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
pub async fn download_activity_file(
    storage: Arc<dyn WorkflowStorage>,
    Extension(claims): Extension<ValidatedClaims>,
    Path((workflow_id, activity_key, filename)): Path<(Uuid, String, String)>,
) -> ApiResult<impl IntoResponse>;
```

#### 3.3 New Routes

Add to `api/src/routes.rs`:

```rust
// Activity Output APIs
.route(
    "/api/v1/workflows/:workflow_id/activities/:activity_key/output",
    get(handlers::get_activity_output),
)
.route(
    "/api/v1/workflows/:workflow_id/output",
    get(handlers::get_workflow_output),
)
.route(
    "/api/v1/workflows/:workflow_id/activities/:activity_key/files/:filename",
    get(handlers::download_activity_file),
)
```

### 4. Terminal Activity Detection

To identify terminal activities (those whose outputs are "final workflow outputs"), we need to analyze the workflow definition:

```rust
/// Determine which activities are terminal (have no dependents)
fn find_terminal_activities(definition: &WorkflowDefinition) -> Vec<String> {
    let mut has_dependents: HashSet<String> = HashSet::new();

    // Collect all activities that are depended upon
    for activity in &definition.activities {
        for dep in &activity.depends_on {
            has_dependents.insert(dep.activity_key.clone());
        }
    }

    // Activities with no dependents are terminal
    definition.activities
        .iter()
        .filter(|a| !has_dependents.contains(&a.key))
        .map(|a| a.key.clone())
        .collect()
}
```

### 5. File Streaming Implementation

For file downloads, use streaming to avoid loading entire files into memory:

```rust
pub async fn download_activity_file(
    storage: Arc<dyn WorkflowStorage>,
    Extension(claims): Extension<ValidatedClaims>,
    Path((workflow_id, activity_key, filename)): Path<(Uuid, String, String)>,
) -> ApiResult<impl IntoResponse> {
    // Get file metadata first
    let metadata = storage
        .get_file_metadata(workflow_id, &activity_key, &filename)
        .await
        .map_err(|e| match e {
            StorageError::FileNotFound(_) => AppError::NotFound("File not found".to_string()),
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    // Stream file content
    let stream = storage
        .download_file(workflow_id, &activity_key, &filename)
        .await
        .map_err(|e| AppError::InternalError(anyhow::anyhow!(e)))?;

    // Build response with appropriate headers
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        metadata.content_type
            .unwrap_or("application/octet-stream".to_string())
            .parse()
            .unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", filename).parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        metadata.size.to_string().parse().unwrap(),
    );

    Ok((headers, body))
}
```

## Implementation Tasks

### Phase 1: Core Service

1. Create `OutputQueryError` enum in `core/src/workflow/output_query_error.rs`
2. Create `OutputQueryService` in `core/src/workflow/output_query_service.rs`
3. Add `get_activity_output()` method with:
   - Workflow lookup
   - Activity state parsing from `workflows.activities` JSONB
   - Cost aggregation from `activity_costs` table
   - File listing from `workflow_files` table
4. Add `get_workflow_output()` method with:
   - Full workflow state retrieval
   - Terminal activity detection
   - Cost aggregation per activity
5. Export new types in `core/src/workflow/mod.rs`

### Phase 2: API Handlers

6. Create DTO types in `api/src/dto/output.rs`:
   - `GetActivityOutputResponse`
   - `GetWorkflowOutputResponse`
   - `FileInfo`
   - `ActivityOutputSummary`
7. Create handler functions in `api/src/handlers/outputs.rs`:
   - `get_activity_output`
   - `get_workflow_output`
   - `download_activity_file`
8. Export handlers in `api/src/handlers/mod.rs`

### Phase 3: Routes and Documentation

9. Add routes to `api/src/routes.rs`
10. Add OpenAPI documentation to `api/src/openapi.rs`
11. Update `docs/architecture.md` with new endpoints

### Phase 4: Integration Testing

12. Create integration test `api/tests/output_retrieval_test.rs`:
    - Test activity output retrieval for completed activity
    - Test 404 for non-existent activity
    - Test 400 for incomplete activity
    - Test workflow output retrieval
    - Test file download streaming
    - Test cost aggregation accuracy

## Files to Create/Modify

### New Files
- `core/src/workflow/output_query_service.rs` - Core query service
- `api/src/dto/output.rs` - Response DTOs
- `api/src/handlers/outputs.rs` - Handler functions
- `api/tests/output_retrieval_test.rs` - Integration tests

### Modified Files
- `core/src/workflow/mod.rs` - Export new service
- `api/src/handlers/mod.rs` - Export new handlers
- `api/src/routes.rs` - Add new routes
- `api/src/openapi.rs` - Add OpenAPI docs
- `docs/architecture.md` - Document new endpoints

## Dependencies

- Uses existing `WorkflowStorage` interface for file operations
- Uses existing `activity_costs` table for cost data
- Uses existing `workflow_files` table for file metadata

## Testing Strategy

1. **Unit Tests**: Test parsing of activity state from JSONB
2. **Integration Tests**: End-to-end tests with database
3. **File Streaming Tests**: Verify large file handling without memory issues

## Estimated Effort

- Core Service: 2-3 hours
- API Handlers: 2-3 hours
- Routes & Documentation: 1 hour
- Integration Tests: 2-3 hours
- **Total: 7-10 hours**

## Implementation Notes

### Files Created

- `core/src/workflow/output_query_service.rs` - Core query service with:
  - `OutputQueryService` - Main service class
  - `ActivityOutputResult` - Activity output with cost and file info
  - `WorkflowOutputResult` - Full workflow output with all activities
  - `FileInfo` - File metadata with download URL
  - `OutputQueryError` - Error types for output retrieval
  - Terminal activity detection via dependency analysis
  - Per-activity cost aggregation from `activity_costs` table

- `api/src/dto/output.rs` - API response types with OpenAPI schemas:
  - `GetActivityOutputResponse`
  - `GetWorkflowOutputResponse`
  - `ActivityOutputSummary`
  - `FileInfo`

- `api/src/handlers/outputs.rs` - Handler functions:
  - `get_activity_output` - GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output
  - `get_workflow_output` - GET /api/v1/workflows/{workflow_id}/output
  - `download_activity_file` - GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}

### Files Modified

- `core/src/workflow/mod.rs` - Added exports for new types
- `api/src/dto/mod.rs` - Added output module export
- `api/src/handlers/mod.rs` - Added output handlers export
- `api/src/routes.rs` - Added new routes to protected routes
- `api/src/openapi.rs` - Added OpenAPI documentation and schemas

### Key Implementation Decisions

1. **Terminal Activity Detection**: Terminal activities (those whose outputs are "final workflow outputs") are detected by analyzing which activities have no dependents in the workflow graph.

2. **File Streaming**: Uses `WorkflowStorage::download_file` which returns an async stream for memory-efficient large file handling.

3. **Cost Aggregation**: Activity costs are fetched from the `activity_costs` table and aggregated per activity, supporting multiple cost records per activity (e.g., retries).

4. **Error Handling**: Returns 400 Bad Request when activity/workflow is not completed (not 404), since the resource exists but isn't ready yet.

### Tests

**Unit Tests** (`core/src/workflow/output_query_service.rs`):
- `test_find_terminal_activities_single_activity`
- `test_find_terminal_activities_linear_chain`
- `test_find_terminal_activities_fan_out`
- `test_find_terminal_activities_fan_in`
- `test_find_terminal_activities_diamond_pattern`
- `test_find_terminal_activities_multiple_independent_chains`
- `test_find_terminal_activities_empty_depends_on`
- `test_find_terminal_activities_invalid_json`
- `test_file_info_from_metadata`

**Integration Tests** (`api/tests/output_retrieval_tests.rs`):
- `test_get_activity_output_success`
- `test_get_activity_output_not_completed`
- `test_get_activity_output_workflow_not_found`
- `test_get_activity_output_activity_not_found`
- `test_get_activity_output_requires_authentication`
- `test_get_workflow_output_success`
- `test_get_workflow_output_not_completed`
- `test_get_workflow_output_not_found`
- `test_get_workflow_output_requires_authentication`
- `test_download_file_not_found`
- `test_download_file_requires_authentication`
- `test_output_retrieval_end_to_end`
