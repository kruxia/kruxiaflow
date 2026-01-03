# Implementation Plan: US-1A.6 Workflow Status and Query API

**Epic**: 1A - API Server
**User Story**: US-1A.6
**Status**: ✅ Complete
**Priority**: P0 (Must Have for MVP)
**Last Updated**: 2025-11-06
**Implemented**: 2025-11-06

---

## User Story

**As** a platform engineering lead
**I want** to query workflow status and results via API
**So that** I can monitor execution and retrieve outputs

### Acceptance Criteria

- `GET /api/v1/workflows/{workflow_id}` - Get workflow status and state
- Response includes: `{id, status, definition_name, created_at, updated_at, activities: Vec<ActivityState>, state_data}`
- Activities returned as structured objects with typed status enums (not raw JSON)
- `GET /api/v1/workflows?status=running&limit=100` - List workflows with filters
- Pagination: `limit` (default 100) and `offset` (default 0) parameters
- Filter parameters: `status`, `definition_name`, `created_after`, `created_before`

---

## Rationale

This user story enables monitoring and observability for workflow executions, completing the submission → execution → monitoring cycle. It provides read-only query access to workflow state and activity progress.

**Why This Story is Critical**:
- Enables workflow monitoring (check if workflow completed/failed)
- Provides activity-level visibility (which activities running/completed) via structured types
- Supports dashboard and UI development
- Essential for debugging workflow issues
- Completes the workflow lifecycle API (submit via US-1A.5, query via US-1A.6)
- Type safety for activity status values (enums instead of strings)

**Key Design Decisions**:
1. **Separate Activities Column**: Activities stored in dedicated `workflows.activities` JSONB column (as of US-1A.5)
   - Fast queries for activity states (O(1) access)
   - Separate from custom workflow state_data
   - No need to reconstruct from event log
   - Matches orchestrator's materialized state pattern
2. **Single Workflow Endpoint**: GET /workflows/{id} returns both workflow metadata and structured activities
   - Eliminated redundant /workflows/{id}/activities endpoint
   - Activities returned as typed Vec<ActivityState> (not raw JSON)
   - Status fields use enums (WorkflowStatus, WorkflowActivityStatus) for type safety
   - All activity information in one query
   - No JOIN to activity_queue table needed
   - Consistent view of workflow state
3. **List Workflows with Filters**: Support common query patterns
   - Filter by status (running, completed, failed)
   - Filter by definition_name (definition name)
   - Filter by time range (created_after, created_before)
   - Pagination for large result sets
4. **Read-Only Endpoints**: All GET requests, no mutations
   - Safe to call repeatedly
   - Cacheable responses (future optimization)
   - No transaction overhead

---

## Architecture Reference

Per `docs/architecture.md` (Workflow State Management):
- Activity states stored in `workflows.activities` JSONB column (materialized, as of US-1A.5)
- Custom workflow state stored in separate `workflows.state_data` JSONB column
- O(1) access time regardless of workflow complexity
- Events preserved in `workflow_events` for audit trail

Per `docs/mvp-requirements.md` (Epic 1A, US-1A.6):
- Get individual workflow status
- List activities for a workflow
- List workflows with filters and pagination
- Authentication required for all endpoints

**Database Schema**:
```sql
-- Workflows table (from US-1A.5)
CREATE TABLE workflows (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    definition_name TEXT NOT NULL,  -- Workflow type (for filtering)
    workflow_definition_id UUID NOT NULL REFERENCES workflow_definitions(id),
    input JSONB NOT NULL,  -- Original input parameters
    unique_key TEXT UNIQUE,  -- Idempotency key
    status workflow_status NOT NULL DEFAULT 'created',
    activities JSONB NOT NULL,  -- Activity states map (separate from state_data)
    state_data JSONB NOT NULL,  -- Custom workflow state
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_workflows_type_status ON workflows(definition_name, status, created_at DESC);
CREATE INDEX idx_workflows_status ON workflows(status, updated_at DESC);
```

**Activities Column Format** (stored in `activities` column):
```json
{
  "validate_payment": {
    "status": "Completed",
    "outputs": {"valid": true},
    "started_at": "2025-11-06T10:00:00Z",
    "completed_at": "2025-11-06T10:00:01Z"
  },
  "authorize_card": {
    "status": "Running",
    "outputs": null,
    "started_at": "2025-11-06T10:00:01Z",
    "completed_at": null
  },
  "capture_payment": {
    "status": "NotScheduled",
    "outputs": null,
    "started_at": null,
    "completed_at": null
  }
}
```

**State Data Column Format** (stored in `state_data` column):
```json
{
  "custom_field": "value",
  "workflow_specific_state": {}
}
```

---

## Implementation Components

### Component 1: Workflow Query Request/Response Types

**Location**: `api/src/handlers/workflows.rs` (extend existing file from US-1A.5)

**Responsibilities**:
1. Define request/response types for workflow queries
2. Support filtering and pagination
3. Provide OpenAPI documentation

**Implementation Notes**:
Following US-1A.5 patterns:
- Request/response types defined in handlers module
- Use `ToSchema` for OpenAPI generation
- Validation via `ValidationErrors` pattern
- Return types use `ApiResult<Json<T>>` pattern

**Implementation**:

```rust
use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{Extension, Json, extract::{Path, Query}};
use serde::{Deserialize, Serialize};
use utoipa::{ToSchema, IntoParams};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Get workflow by ID response
///
/// Returns the current state of a workflow, including status,
/// activity states, and custom state data.
#[derive(Debug, Serialize, ToSchema)]
pub struct GetWorkflowResponse {
    /// Unique workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub id: Uuid,

    /// Workflow status
    #[schema(example = "running")]
    pub status: String,

    /// Workflow type (definition name)
    #[schema(example = "payment_processing")]
    pub definition_name: String,

    /// When the workflow was created
    #[schema(example = "2025-11-06T10:00:00Z")]
    pub created_at: DateTime<Utc>,

    /// When the workflow was last updated
    #[schema(example = "2025-11-06T10:00:05Z")]
    pub updated_at: DateTime<Utc>,

    /// Activity states (structured objects with typed status enums)
    pub activities: Vec<ActivityState>,

    /// Custom workflow state data (from workflows.state_data column)
    #[schema(example = json!({
        "custom_field": "value"
    }))]
    pub state_data: serde_json::Value,
}

/// Activity state in a workflow
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityState {
    /// Activity key (unique within workflow)
    #[schema(example = "validate_payment")]
    pub activity_key: String,

    /// Activity status: not_scheduled, pending, running, completed, or failed
    #[schema(value_type = String, example = "completed")]
    pub status: WorkflowActivityStatus,

    /// Activity outputs (null if not completed)
    #[schema(example = json!({"valid": true}))]
    pub outputs: Option<serde_json::Value>,

    /// When the activity started (null if not started)
    #[schema(example = "2025-11-06T10:00:00Z")]
    pub started_at: Option<DateTime<Utc>>,

    /// When the activity completed (null if not completed)
    #[schema(example = "2025-11-06T10:00:01Z")]
    pub completed_at: Option<DateTime<Utc>>,
}

/// List workflows query parameters
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListWorkflowsQuery {
    /// Filter by workflow status
    #[serde(skip_serializing_if = "Option::is_none")]
    #[param(example = "running")]
    pub status: Option<String>,

    /// Filter by workflow type (definition name)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[param(example = "payment_processing")]
    pub definition_name: Option<String>,

    /// Filter workflows created after this time
    #[serde(skip_serializing_if = "Option::is_none")]
    #[param(example = "2025-11-06T00:00:00Z")]
    pub created_after: Option<DateTime<Utc>>,

    /// Filter workflows created before this time
    #[serde(skip_serializing_if = "Option::is_none")]
    #[param(example = "2025-11-06T23:59:59Z")]
    pub created_before: Option<DateTime<Utc>>,

    /// Maximum number of results (default 100, max 1000)
    #[serde(default = "default_limit")]
    #[param(example = 100)]
    pub limit: i64,

    /// Offset for pagination (default 0)
    #[serde(default)]
    #[param(example = 0)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

impl ListWorkflowsQuery {
    /// Validate query parameters
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.limit < 1 {
            errors.add("limit", "Limit must be at least 1");
        }
        if self.limit > 1000 {
            errors.add("limit", "Limit must be at most 1000");
        }
        if self.offset < 0 {
            errors.add("offset", "Offset must be non-negative");
        }

        // Validate time range
        if let (Some(after), Some(before)) = (self.created_after, self.created_before) {
            if after >= before {
                errors.add("created_after", "created_after must be before created_before");
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// List workflows response
#[derive(Debug, Serialize, ToSchema)]
pub struct ListWorkflowsResponse {
    /// List of workflows matching filter criteria
    pub workflows: Vec<WorkflowSummary>,

    /// Total count of matching workflows (for pagination)
    pub total: i64,

    /// Number of results returned
    pub count: i64,

    /// Query limit
    pub limit: i64,

    /// Query offset
    pub offset: i64,
}

/// Workflow summary (for list view)
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowSummary {
    /// Unique workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub id: Uuid,

    /// Workflow status
    #[schema(example = "running")]
    pub status: String,

    /// Workflow type (definition name)
    #[schema(example = "payment_processing")]
    pub definition_name: String,

    /// When the workflow was created
    #[schema(example = "2025-11-06T10:00:00Z")]
    pub created_at: DateTime<Utc>,

    /// When the workflow was last updated
    #[schema(example = "2025-11-06T10:00:05Z")]
    pub updated_at: DateTime<Utc>,
}
```

**Key Features**:
- Clear request/response structure
- Filter and pagination support
- Activity state extraction from materialized state
- OpenAPI documentation via utoipa
- Validation with field-level errors

---

### Component 2: Workflow Query Service Layer

**Location**: `core/src/workflow/query_service.rs` (new file)

**Responsibilities**:
1. Query individual workflow by ID
2. Extract activity states from materialized state
3. List workflows with filters and pagination
4. Handle not found errors

**Implementation Notes**:
Following US-1A.5 repository pattern:
- Service struct holds PgPool (cloneable)
- Service implements Clone (required for FromRequestParts extractor)
- NOT added to AppState (extracted on-demand in handlers)
- Methods return `Result<T, WorkflowQueryError>`
- Error type uses thiserror

**Implementation**:

```rust
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Workflow query service error
#[derive(Debug, Error)]
pub enum WorkflowQueryError {
    #[error("Workflow not found: {0}")]
    WorkflowNotFound(Uuid),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("State deserialization error: {0}")]
    DeserializationError(#[from] serde_json::Error),
}

pub type WorkflowQueryResult<T> = Result<T, WorkflowQueryError>;

/// Workflow record (full detail)
#[derive(Debug, Clone)]
pub struct WorkflowRecord {
    pub id: Uuid,
    pub definition_name: String,
    pub status: String,
    pub activities: Value,
    pub state_data: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Activity state extracted from workflow state_data
#[derive(Debug, Clone)]
pub struct ActivityRecord {
    pub activity_key: String,
    pub status: String,
    pub outputs: Option<Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Workflow summary for list view
#[derive(Debug, Clone)]
pub struct WorkflowSummaryRecord {
    pub id: Uuid,
    pub definition_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Workflow query filters
#[derive(Debug, Clone, Default)]
pub struct WorkflowFilters {
    pub status: Option<String>,
    pub definition_name: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}

/// Workflow query service
///
/// Provides read-only access to workflow state and activity information.
/// Follows the repository pattern - holds PgPool and is cloneable.
#[derive(Clone)]
pub struct WorkflowQueryService {
    pool: PgPool,
}

impl WorkflowQueryService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get workflow by ID
    ///
    /// Returns workflow record with activities and state_data.
    pub async fn get_workflow(&self, workflow_id: Uuid) -> WorkflowQueryResult<WorkflowRecord> {
        let row = sqlx::query!(
            r#"
            SELECT id, definition_name, status AS "status: String",
                   activities, state_data, created_at, updated_at
            FROM workflows
            WHERE id = $1
            "#,
            workflow_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(WorkflowQueryError::WorkflowNotFound(workflow_id))?;

        Ok(WorkflowRecord {
            id: row.id,
            definition_name: row.definition_name,
            status: row.status,
            activities: row.activities,
            state_data: row.state_data,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Get activities for a workflow
    ///
    /// Reads activity states from workflow.activities column.
    pub async fn get_workflow_activities(
        &self,
        workflow_id: Uuid,
    ) -> WorkflowQueryResult<Vec<ActivityRecord>> {
        // Get workflow activities from dedicated column
        let workflow = self.get_workflow(workflow_id).await?;

        // Parse activities from the activities column
        let activities_map = workflow
            .activities
            .as_object()
            .ok_or_else(|| {
                WorkflowQueryError::DeserializationError(serde_json::Error::custom(
                    "activities column is not an object",
                ))
            })?;

        let mut activities = Vec::new();
        for (activity_key, activity_state) in activities_map {
            let status = activity_state
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let outputs = activity_state.get("outputs").cloned();

            let started_at = activity_state
                .get("started_at")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            let completed_at = activity_state
                .get("completed_at")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            activities.push(ActivityRecord {
                activity_key: activity_key.clone(),
                status,
                outputs,
                started_at,
                completed_at,
            });
        }

        Ok(activities)
    }

    /// List workflows with filters and pagination
    ///
    /// Returns workflows matching filter criteria with pagination support.
    pub async fn list_workflows(
        &self,
        filters: WorkflowFilters,
        limit: i64,
        offset: i64,
    ) -> WorkflowQueryResult<(Vec<WorkflowSummaryRecord>, i64)> {
        // Build query with filters
        let mut query = String::from(
            r#"
            SELECT id, definition_name, status, created_at, updated_at
            FROM workflows
            WHERE 1=1
            "#,
        );

        let mut param_num = 1;
        let mut bind_values: Vec<Box<dyn sqlx::Encode<'_, sqlx::Postgres> + Send>> = Vec::new();

        // Add filters
        if let Some(ref status) = filters.status {
            query.push_str(&format!(" AND status = ${}", param_num));
            bind_values.push(Box::new(status.clone()));
            param_num += 1;
        }

        if let Some(ref definition_name) = filters.definition_name {
            query.push_str(&format!(" AND definition_name = ${}", param_num));
            bind_values.push(Box::new(definition_name.clone()));
            param_num += 1;
        }

        if let Some(created_after) = filters.created_after {
            query.push_str(&format!(" AND created_at >= ${}", param_num));
            bind_values.push(Box::new(created_after));
            param_num += 1;
        }

        if let Some(created_before) = filters.created_before {
            query.push_str(&format!(" AND created_at < ${}", param_num));
            bind_values.push(Box::new(created_before));
            param_num += 1;
        }

        // Order by created_at DESC (most recent first)
        query.push_str(" ORDER BY created_at DESC");

        // Pagination
        query.push_str(&format!(" LIMIT ${} OFFSET ${}", param_num, param_num + 1));

        // Note: Due to sqlx limitations with dynamic queries, we'll use a different 
        // approach: For MVP, we'll use optional filters directly in query.
        let rows = sqlx::query!(
            r#"
            SELECT id, definition_name, status AS "status: String",
                   created_at, updated_at
            FROM workflows
            WHERE ($1::TEXT IS NULL OR status = $1::workflow_status::TEXT)
              AND ($2::TEXT IS NULL OR definition_name = $2)
              AND ($3::TIMESTAMPTZ IS NULL OR created_at >= $3)
              AND ($4::TIMESTAMPTZ IS NULL OR created_at < $4)
            ORDER BY created_at DESC
            LIMIT $5 OFFSET $6
            "#,
            filters.status.as_deref(),
            filters.definition_name.as_deref(),
            filters.created_after,
            filters.created_before,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;

        // Get total count (for pagination)
        let total = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*)
            FROM workflows
            WHERE ($1::TEXT IS NULL OR status = $1::workflow_status::TEXT)
              AND ($2::TEXT IS NULL OR definition_name = $2)
              AND ($3::TIMESTAMPTZ IS NULL OR created_at >= $3)
              AND ($4::TIMESTAMPTZ IS NULL OR created_at < $4)
            "#,
            filters.status.as_deref(),
            filters.definition_name.as_deref(),
            filters.created_after,
            filters.created_before
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0);

        let workflows = rows
            .into_iter()
            .map(|row| WorkflowSummaryRecord {
                id: row.id,
                definition_name: row.definition_name,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();

        Ok((workflows, total))
    }
}
```

**Key Features**:
- Get individual workflow with full state_data
- Extract activity states from materialized state
- List workflows with flexible filters
- Pagination support with total count
- Clear error types

**Design Note**: Activity states are read directly from the `workflows.activities` column (as of US-1A.5) rather than querying the `activity_queue` table. This provides a consistent view of workflow state as seen by the orchestrator and avoids JOIN queries.

---

### Component 3: API Handlers

**Location**: `api/src/handlers/workflows.rs` (extend existing file)

**Responsibilities**:
1. Handle GET /api/v1/workflows/{workflow_id}
2. Handle GET /api/v1/workflows/{workflow_id}/activities
3. Handle GET /api/v1/workflows (list with filters)
4. Map errors to HTTP status codes
5. Return responses

**Implementation Notes**:
Following US-1A.5 handler patterns with extractor pattern:
- Extract `WorkflowQueryService` directly in handler signature (via FromRequestParts extractor)
- Use `Extension<ValidatedClaims>` for authentication
- Error mapping with match expressions
- Return `ApiResult<Json<Response>>`
- Structured logging

**Implementation**:

```rust
/// Get workflow by ID
///
/// Endpoint: GET /api/v1/workflows/{workflow_id}
///
/// Returns the current state of a workflow, including status, activity states,
/// and custom state data.
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}",
    tag = "Workflows",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Workflow found", body = GetWorkflowResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_workflow(
    service: WorkflowQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Path(workflow_id): Path<Uuid>,
) -> ApiResult<Json<GetWorkflowResponse>> {
    tracing::info!(
        workflow_id = %workflow_id,
        user = %claims.subject(),
        "Getting workflow"
    );

    let workflow = service
        .get_workflow(workflow_id)
        .await
        .map_err(|e| match e {
            WorkflowQueryError::WorkflowNotFound(id) => {
                tracing::warn!("Workflow not found: {}", id);
                AppError::NotFound(format!("Workflow '{}' not found", id))
            }
            WorkflowQueryError::DatabaseError(e) => {
                tracing::error!("Database error getting workflow: {:?}", e);
                AppError::DatabaseError(e)
            }
            WorkflowQueryError::DeserializationError(e) => {
                tracing::error!("Deserialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    tracing::debug!(
        workflow_id = %workflow.id,
        status = %workflow.status,
        "Workflow retrieved"
    );

    Ok(Json(GetWorkflowResponse {
        id: workflow.id,
        status: workflow.status,
        definition_name: workflow.definition_name,
        created_at: workflow.created_at,
        updated_at: workflow.updated_at,
        activities: workflow.activities,
        state_data: workflow.state_data,
    }))
}

/// Get workflow activities
///
/// Endpoint: GET /api/v1/workflows/{workflow_id}/activities
///
/// Returns a list of all activities in the workflow with their current states.
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/activities",
    tag = "Workflows",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Activities list", body = ListActivitiesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_workflow_activities(
    service: WorkflowQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Path(workflow_id): Path<Uuid>,
) -> ApiResult<Json<ListActivitiesResponse>> {
    tracing::info!(
        workflow_id = %workflow_id,
        user = %claims.subject(),
        "Getting workflow activities"
    );

    let activities = service
        .get_workflow_activities(workflow_id)
        .await
        .map_err(|e| match e {
            WorkflowQueryError::WorkflowNotFound(id) => {
                tracing::warn!("Workflow not found: {}", id);
                AppError::NotFound(format!("Workflow '{}' not found", id))
            }
            WorkflowQueryError::DatabaseError(e) => {
                tracing::error!("Database error getting activities: {:?}", e);
                AppError::DatabaseError(e)
            }
            WorkflowQueryError::DeserializationError(e) => {
                tracing::error!("Deserialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    tracing::debug!(
        workflow_id = %workflow_id,
        activity_count = activities.len(),
        "Activities retrieved"
    );

    Ok(Json(ListActivitiesResponse {
        workflow_id,
        activities: activities
            .into_iter()
            .map(|a| ActivityState {
                activity_key: a.activity_key,
                status: a.status,
                outputs: a.outputs,
                started_at: a.started_at,
                completed_at: a.completed_at,
            })
            .collect(),
    }))
}

/// List workflows
///
/// Endpoint: GET /api/v1/workflows
///
/// Returns a paginated list of workflows matching filter criteria.
#[utoipa::path(
    get,
    path = "/api/v1/workflows",
    tag = "Workflows",
    params(
        ListWorkflowsQuery
    ),
    responses(
        (status = 200, description = "Workflows list", body = ListWorkflowsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_workflows(
    service: WorkflowQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Query(query): Query<ListWorkflowsQuery>,
) -> ApiResult<Json<ListWorkflowsResponse>> {
    // Validate query parameters
    query.validate().map_err(AppError::ValidationError)?;

    tracing::info!(
        status = ?query.status,
        definition_name = ?query.definition_name,
        limit = query.limit,
        offset = query.offset,
        user = %claims.subject(),
        "Listing workflows"
    );

    let filters = WorkflowFilters {
        status: query.status.clone(),
        definition_name: query.definition_name.clone(),
        created_after: query.created_after,
        created_before: query.created_before,
    };

    let (workflows, total) = service
        .list_workflows(filters, query.limit, query.offset)
        .await
        .map_err(|e| match e {
            WorkflowQueryError::DatabaseError(e) => {
                tracing::error!("Database error listing workflows: {:?}", e);
                AppError::DatabaseError(e)
            }
            WorkflowQueryError::DeserializationError(e) => {
                tracing::error!("Deserialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
            WorkflowQueryError::WorkflowNotFound(_) => {
                // This shouldn't happen in list_workflows
                AppError::InternalError(anyhow::anyhow!("Unexpected error"))
            }
        })?;

    tracing::debug!(
        count = workflows.len(),
        total = total,
        "Workflows retrieved"
    );

    Ok(Json(ListWorkflowsResponse {
        workflows: workflows
            .into_iter()
            .map(|w| WorkflowSummary {
                id: w.id,
                status: w.status,
                definition_name: w.definition_name,
                created_at: w.created_at,
                updated_at: w.updated_at,
            })
            .collect(),
        total,
        count: workflows.len() as i64,
        limit: query.limit,
        offset: query.offset,
    }))
}
```

**Key Features**:
- Three query endpoints (get workflow, list activities, list workflows)
- Service extracted directly in handler signature (via FromRequestParts extractor)
- Clear OpenAPI documentation
- Error mapping to HTTP status codes
- Structured logging
- Authentication via ValidatedClaims

---

### Component 4: Create WorkflowQueryService Extractor

**Location**: Update `api/src/extractors.rs`

**Responsibilities**:
1. Implement FromRequestParts<AppState> for WorkflowQueryService
2. Extract service on-demand in handlers
3. Create service instances using db_pool from AppState

**Implementation Notes**:
Following the extractor pattern used for WorkflowDefinitionRepository and WorkflowService:
- Services are NOT added to AppState
- Instead, implement `FromRequestParts<AppState>` to allow direct extraction in handlers
- Service instances created on-demand from AppState's db_pool
- Cleaner handler signatures, no need to access state.workflow_query_service

**Implementation**:

```rust
/// Axum extractor for WorkflowQueryService
///
/// Allows WorkflowQueryService to be extracted directly in handler signatures.
/// Automatically creates service from AppState's db_pool.
///
/// # Example
/// ```rust,ignore
/// async fn handler(service: WorkflowQueryService) -> impl IntoResponse {
///     // Use service directly
/// }
/// ```
#[async_trait]
impl FromRequestParts<AppState> for WorkflowQueryService {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(WorkflowQueryService::new(state.db_pool.clone()))
    }
}
```

**Key Features**:
- WorkflowQueryService created on-demand from db_pool
- No need to modify AppState
- Consistent with other service extractors
- Handler signatures are cleaner and more ergonomic

---

### Component 5: Update Routes

**Location**: Update `api/src/routes.rs`

**Responsibilities**:
1. Add workflow query routes to protected routes
2. Ensure authentication middleware applied

**Implementation Notes**:
- Routes simply map paths to handlers
- Handlers use WorkflowQueryService extractor (not from AppState)
- No changes needed to AppState

**Implementation**:

```rust
/// Protected API routes (require authentication)
pub fn protected_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/info", get(handlers::health::service_info_handler))

        // Workflow Definition Management
        .route(
            "/api/v1/workflow_definitions",
            post(handlers::workflow_definitions::deploy_workflow_definition)
                .get(handlers::workflow_definitions::list_workflow_definitions),
        )
        .route(
            "/api/v1/workflow_definitions/:name",
            get(handlers::workflow_definitions::get_workflow_definition),
        )

        // Workflow Submission
        .route(
            "/api/v1/workflows",
            post(handlers::workflows::submit_workflow)
                .get(handlers::workflows::list_workflows),  // List workflows
        )
        // Workflow Query
        .route(
            "/api/v1/workflows/:workflow_id",
            get(handlers::workflows::get_workflow),  // Get specific workflow
        )
        .route(
            "/api/v1/workflows/:workflow_id/activities",
            get(handlers::workflows::get_workflow_activities),  // Get workflow activities
        )

        // Apply authentication middleware to all routes
        .layer(axum_middleware::from_fn_with_state(
            middleware::auth_middleware,
        ))
}
```

---

### Component 6: Update OpenAPI Documentation

**Location**: Update `api/src/openapi.rs`

**Responsibilities**:
1. Add workflow query endpoints to OpenAPI spec
2. Document request/response schemas

**Implementation**:

```rust
use crate::handlers::workflows::{
    SubmitWorkflowRequest, SubmitWorkflowResponse,
    GetWorkflowResponse, ListActivitiesResponse, ActivityState,
    ListWorkflowsQuery, ListWorkflowsResponse, WorkflowSummary,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Kruxia Flow API",
        version = "0.2.0",
        description = "High-performance workflow orchestration platform for AI-native workloads",
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server")
    ),
    paths(
        // ... existing paths ...

        // Workflow submission and query
        crate::handlers::workflows::submit_workflow,
        crate::handlers::workflows::get_workflow,
        crate::handlers::workflows::get_workflow_activities,
        crate::handlers::workflows::list_workflows,
    ),
    components(
        schemas(
            // ... existing schemas ...

            // Workflow query schemas
            GetWorkflowResponse,
            ListActivitiesResponse,
            ActivityState,
            ListWorkflowsQuery,
            ListWorkflowsResponse,
            WorkflowSummary,
        )
    ),
    tags(
        // ... existing tags ...
        (name = "Workflows", description = "Workflow submission, query, and management"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;
```

---

## Testing Requirements

### Unit Tests

**File**: `core/src/workflow/query_service_test.rs`

**Test Scenarios**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_workflow() {
        let pool = test_db_pool().await;
        let service = WorkflowQueryService::new(pool.clone());

        // Create test workflow
        let workflow_id = create_test_workflow(&pool).await;

        // Get workflow
        let result = service.get_workflow(workflow_id).await;

        assert!(result.is_ok());
        let workflow = result.unwrap();
        assert_eq!(workflow.id, workflow_id);
        assert!(!workflow.status.is_empty());
    }

    #[tokio::test]
    async fn test_get_workflow_not_found() {
        let pool = test_db_pool().await;
        let service = WorkflowQueryService::new(pool.clone());

        let nonexistent_id = Uuid::now_v7();
        let result = service.get_workflow(nonexistent_id).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkflowQueryError::WorkflowNotFound(_)
        ));
    }

    #[tokio::test]
    async fn test_get_workflow_activities() {
        let pool = test_db_pool().await;
        let service = WorkflowQueryService::new(pool.clone());

        // Create test workflow with activities
        let workflow_id = create_test_workflow_with_activities(&pool).await;

        // Get activities
        let result = service.get_workflow_activities(workflow_id).await;

        assert!(result.is_ok());
        let activities = result.unwrap();
        assert!(!activities.is_empty());
        assert!(activities.iter().any(|a| a.status == "Completed"));
    }

    #[tokio::test]
    async fn test_list_workflows_no_filters() {
        let pool = test_db_pool().await;
        let service = WorkflowQueryService::new(pool.clone());

        // Create multiple test workflows
        create_test_workflows(&pool, 5).await;

        // List all workflows
        let filters = WorkflowFilters::default();
        let result = service.list_workflows(filters, 100, 0).await;

        assert!(result.is_ok());
        let (workflows, total) = result.unwrap();
        assert_eq!(workflows.len(), 5);
        assert_eq!(total, 5);
    }

    #[tokio::test]
    async fn test_list_workflows_filter_by_status() {
        let pool = test_db_pool().await;
        let service = WorkflowQueryService::new(pool.clone());

        // Create workflows with different statuses
        create_workflow_with_status(&pool, "running").await;
        create_workflow_with_status(&pool, "running").await;
        create_workflow_with_status(&pool, "completed").await;

        // Filter by status=running
        let filters = WorkflowFilters {
            status: Some("running".to_string()),
            ..Default::default()
        };
        let result = service.list_workflows(filters, 100, 0).await;

        assert!(result.is_ok());
        let (workflows, total) = result.unwrap();
        assert_eq!(workflows.len(), 2);
        assert_eq!(total, 2);
        assert!(workflows.iter().all(|w| w.status == "running"));
    }

    #[tokio::test]
    async fn test_list_workflows_pagination() {
        let pool = test_db_pool().await;
        let service = WorkflowQueryService::new(pool.clone());

        // Create 10 test workflows
        create_test_workflows(&pool, 10).await;

        // Get first page (5 workflows)
        let filters = WorkflowFilters::default();
        let result = service.list_workflows(filters.clone(), 5, 0).await;

        assert!(result.is_ok());
        let (page1, total) = result.unwrap();
        assert_eq!(page1.len(), 5);
        assert_eq!(total, 10);

        // Get second page (5 workflows)
        let result = service.list_workflows(filters, 5, 5).await;

        assert!(result.is_ok());
        let (page2, total) = result.unwrap();
        assert_eq!(page2.len(), 5);
        assert_eq!(total, 10);

        // Ensure different workflows
        assert_ne!(page1[0].id, page2[0].id);
    }
}
```

---

### Integration Tests

**File**: `api/tests/workflow_query_test.rs`

**Test Scenarios**:

```rust
#[tokio::test]
async fn test_get_workflow_success() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Create test workflow
    let workflow_id = submit_test_workflow(&app, &token).await;

    // Get workflow
    let response = app
        .get(&format!("/api/v1/workflows/{}", workflow_id))
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: GetWorkflowResponse = response.json().await;
    assert_eq!(body.id, workflow_id);
    assert!(!body.status.is_empty());
    assert!(!body.definition_name.is_empty());
}

#[tokio::test]
async fn test_get_workflow_not_found() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let nonexistent_id = Uuid::now_v7();

    let response = app
        .get(&format!("/api/v1/workflows/{}", nonexistent_id))
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_workflow_requires_authentication() {
    let app = test_app().await;

    let workflow_id = Uuid::now_v7();

    let response = app
        .get(&format!("/api/v1/workflows/{}", workflow_id))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_workflow_activities() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Create and run test workflow with activities
    let workflow_id = submit_and_run_test_workflow(&app, &token).await;

    let response = app
        .get(&format!("/api/v1/workflows/{}/activities", workflow_id))
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListActivitiesResponse = response.json().await;
    assert_eq!(body.workflow_id, workflow_id);
    assert!(!body.activities.is_empty());
}

#[tokio::test]
async fn test_list_workflows() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Create multiple workflows
    submit_test_workflow(&app, &token).await;
    submit_test_workflow(&app, &token).await;
    submit_test_workflow(&app, &token).await;

    let response = app
        .get("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json().await;
    assert!(body.workflows.len() >= 3);
    assert!(body.total >= 3);
}

#[tokio::test]
async fn test_list_workflows_filter_by_status() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Create workflows with different statuses
    let _running_wf = submit_test_workflow(&app, &token).await;
    complete_workflow(&app, _running_wf).await;

    let response = app
        .get("/api/v1/workflows?status=completed")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json().await;
    assert!(body.workflows.iter().all(|w| w.status == "completed"));
}

#[tokio::test]
async fn test_list_workflows_pagination() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Create 10 workflows
    for _ in 0..10 {
        submit_test_workflow(&app, &token).await;
    }

    // Get first page (limit=5)
    let response = app
        .get("/api/v1/workflows?limit=5&offset=0")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json().await;
    assert_eq!(body.count, 5);
    assert_eq!(body.limit, 5);
    assert_eq!(body.offset, 0);
    assert!(body.total >= 10);
}

#[tokio::test]
async fn test_list_workflows_invalid_limit() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let response = app
        .get("/api/v1/workflows?limit=2000")  // Exceeds max limit
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

---

## Dependencies

### Existing Dependencies

All required dependencies should already be available:
- `sqlx` - Database access
- `serde` / `serde_json` - Serialization
- `uuid` - UUID handling
- `chrono` - Timestamps
- `axum` - HTTP framework
- `utoipa` - OpenAPI generation
- `thiserror` - Error handling
- `tracing` - Logging

### No New Dependencies Required

---

## Configuration

### Environment Variables

No new environment variables required. Uses existing `DATABASE_URL`.

---

## Documentation Updates

### API Documentation

Update `docs/api-reference.md`:

```markdown
## Workflow Query

Query workflow status, activities, and list workflows.

### Get Workflow

**Endpoint**: `GET /api/v1/workflows/{workflow_id}`

**Authentication**: Required (Bearer token)

**Response** (200 OK):
\`\`\`json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "running",
  "definition_name": "payment_processing",
  "created_at": "2025-11-06T10:00:00Z",
  "updated_at": "2025-11-06T10:00:05Z",
  "activities": {
    "validate_payment": {
      "status": "Completed",
      "outputs": {"valid": true}
    },
    "authorize_card": {
      "status": "Running",
      "outputs": null
    }
  },
  "state_data": {
    "custom_field": "value"
  }
}
\`\`\`

### Get Workflow Activities

**Endpoint**: `GET /api/v1/workflows/{workflow_id}/activities`

**Authentication**: Required (Bearer token)

**Response** (200 OK):
\`\`\`json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "activities": [
    {
      "activity_key": "validate_payment",
      "status": "Completed",
      "outputs": {"valid": true},
      "started_at": "2025-11-06T10:00:00Z",
      "completed_at": "2025-11-06T10:00:01Z"
    },
    {
      "activity_key": "authorize_card",
      "status": "Running",
      "outputs": null,
      "started_at": "2025-11-06T10:00:01Z",
      "completed_at": null
    }
  ]
}
\`\`\`

### List Workflows

**Endpoint**: `GET /api/v1/workflows`

**Authentication**: Required (Bearer token)

**Query Parameters**:
- `status` (optional) - Filter by status (e.g., `running`, `completed`, `failed`)
- `definition_name` (optional) - Filter by workflow type (definition name)
- `created_after` (optional) - Filter workflows created after this time (ISO 8601)
- `created_before` (optional) - Filter workflows created before this time (ISO 8601)
- `limit` (optional) - Maximum results (default 100, max 1000)
- `offset` (optional) - Pagination offset (default 0)

**Response** (200 OK):
\`\`\`json
{
  "workflows": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "status": "running",
      "definition_name": "payment_processing",
      "created_at": "2025-11-06T10:00:00Z",
      "updated_at": "2025-11-06T10:00:05Z"
    }
  ],
  "total": 42,
  "count": 1,
  "limit": 100,
  "offset": 0
}
\`\`\`

**Example**:
\`\`\`bash
# Get all running workflows
curl http://localhost:8080/api/v1/workflows?status=running \
  -H "Authorization: Bearer eyJhbGc..."

# Get workflows with pagination
curl http://localhost:8080/api/v1/workflows?limit=10&offset=20 \
  -H "Authorization: Bearer eyJhbGc..."
\`\`\`
```

---

## Success Criteria

### Functional Requirements

- ✅ `GET /api/v1/workflows/{workflow_id}` returns workflow status and state
- ✅ Response includes id, status, definition_name, created_at, updated_at, state_data
- ✅ `GET /api/v1/workflows/{workflow_id}/activities` lists activities with states
- ✅ `GET /api/v1/workflows` lists workflows with filters
- ✅ Pagination support (limit and offset)
- ✅ Filter by status, definition_name, created_after, created_before
- ✅ Returns 404 Not Found for nonexistent workflows
- ✅ Authentication required (Bearer token)

### Non-Functional Requirements

- ✅ Fast queries (<10ms for single workflow)
- ✅ Efficient list queries with indexes
- ✅ Clear error messages
- ✅ OpenAPI documentation for all endpoints
- ✅ Structured logging for debugging

---

## Implementation Phases

### Phase 1: Query Service Layer (P0)
- Implement WorkflowQueryService
- Implement get_workflow, get_workflow_activities, list_workflows methods
- Unit tests for service layer
- **Estimated Time**: 3 hours

### Phase 2: API Handlers (P0)
- Implement GET /api/v1/workflows/{workflow_id}
- Implement GET /api/v1/workflows/{workflow_id}/activities
- Implement GET /api/v1/workflows (list with filters)
- Error mapping to HTTP status codes
- **Estimated Time**: 2 hours

### Phase 3: Extractor and Routes (P0)
- Create WorkflowQueryService extractor (FromRequestParts implementation)
- Add routes for workflow query endpoints
- Update OpenAPI documentation
- **Estimated Time**: 1 hour

### Phase 4: Integration Tests (P0)
- Test get workflow (success and not found)
- Test get workflow activities
- Test list workflows (no filters, with filters, pagination)
- Test authentication required
- **Estimated Time**: 3 hours

### Phase 5: End-to-End Testing (P0)
- Test query after workflow submission
- Test query during workflow execution
- Test query after workflow completion
- Manual testing with curl/Postman
- Update documentation
- **Estimated Time**: 2 hours

**Total Estimated Time**: 11 hours

---

## Risks and Mitigations

### Risk 1: Activity State Deserialization Errors

**Probability**: Medium
**Impact**: Medium (query fails even though workflow exists)

**Mitigation**:
- Validate state_data structure during workflow creation
- Graceful error handling with clear messages
- Fallback to empty activities list if deserialization fails
- Logging for debugging

### Risk 2: Large State Data Performance

**Probability**: Low
**Impact**: Medium (slow queries for workflows with many activities)

**Mitigation**:
- JSONB column indexed for efficient access
- Pagination limits result set size
- Consider activity count limits per workflow
- Post-MVP: Separate activities table if needed

### Risk 3: Filter Query Performance

**Probability**: Low
**Impact**: Medium (slow list queries with many workflows)

**Mitigation**:
- Existing indexes on (definition_name, status, created_at)
- Query optimization with proper index usage
- Pagination prevents unbounded result sets
- Monitor slow queries in production

---

## Future Enhancements (Post-MVP)

### Advanced Filtering
- Filter by workflow input parameters
- Filter by activity status (e.g., "has failed activities")
- Full-text search on workflow type
- Custom metadata filtering

### Query Optimization
- Response caching for completed workflows
- Materialized views for dashboard queries
- Activity summary in list view (counts by status)

### Activity Output Retrieval
- Dedicated endpoint for activity outputs
- Large output handling via artifact storage
- Output streaming for large results

### Workflow History
- Query historical workflow executions
- Trend analysis over time
- Retention policy enforcement

---

## Related User Stories

- **US-1A.5**: Workflow Submission API (creates workflows to query)
- **US-1.2**: Event-Driven Dynamic Scheduling (updates workflow state)
- **US-1A.7**: Worker Activity APIs (workers update activity states)
- **US-1A.9**: WebSocket Streaming (real-time updates complement polling)

---

## References

- Architecture: `docs/architecture.md` (Workflow State Management)
- Requirements: `docs/mvp-requirements.md` (Epic 1A, US-1A.6)
- US-1A.5 Implementation: `docs/implementation/US-1A.5-workflow-submission.md`
- US-1.2 Implementation: `docs/implementation/US-1.2-event-driven-scheduling.md`

---

## Implementation Notes

**Key Design Decisions**:

1. **Separate Activities Column**: Read from `workflows.activities` for O(1) access (as of US-1A.5)
   - Activities stored in dedicated column, not nested in state_data
   - No need to reconstruct from event log
   - Consistent view of workflow state

2. **Direct Activity Column Access**: Read from `workflows.activities` column
   - All activity info in one query
   - No JOIN to activity_queue needed
   - Reflects orchestrator's view of state

3. **List Workflows Filtering**: Use optional parameters in SQL query
   - NULL checks allow flexible filtering
   - Efficient with existing indexes
   - Simple query structure

4. **Pagination**: Standard limit/offset pattern
   - Include total count for pagination UI
   - Default limit of 100, max 1000
   - Offset for page navigation

5. **Read-Only Operations**: All GET requests, no mutations
   - Safe to cache (future optimization)
   - No transaction overhead
   - Idempotent operations

6. **Extractor Pattern**: Use FromRequestParts for service injection
   - WorkflowQueryService NOT added to AppState
   - Service instances created on-demand from db_pool
   - Cleaner handler signatures
   - Consistent with WorkflowDefinitionRepository and WorkflowService patterns

**Implementation Order**:
1. Query service layer (get workflow, activities, list)
2. Create WorkflowQueryService extractor in api/src/extractors.rs
3. API handlers with error mapping (using extractor pattern)
4. Routes configuration
5. Integration tests
6. End-to-end testing and documentation

**Post-Implementation**:
- US-1A.7 will enable workers to update activity states
- US-1A.8 will provide dedicated activity output retrieval
- US-1A.9 will provide real-time updates via WebSocket
- US-10.1 will build dashboard using these query APIs

---

## Definition of Done

- [x] WorkflowQueryService implemented with get_workflow, get_workflow_activities, list_workflows
- [x] WorkflowQueryService extractor (FromRequestParts) implemented in api/src/extractors.rs
- [x] GET /api/v1/workflows/{workflow_id} handler implemented
- [x] GET /api/v1/workflows/{workflow_id}/activities handler implemented
- [x] GET /api/v1/workflows handler implemented (list with filters)
- [x] Request/response validation implemented
- [x] Error mapping to HTTP status codes complete
- [x] Routes include workflow query endpoints
- [x] OpenAPI documentation updated
- [x] Integration tests passing (API endpoints) - 13 tests
- [x] End-to-end tests passing (submit → query workflow)
- [x] All acceptance criteria met
- [x] Zero cargo warnings
- [x] Documentation updated

## Implementation Summary

### Files Created
1. **core/src/workflow/query_service.rs** - WorkflowQueryService with three query methods

### Files Modified
1. **core/src/workflow/mod.rs** - Added query_service module and exports
2. **api/src/handlers/workflows.rs** - Added query request/response types and three handler functions
3. **api/src/handlers/mod.rs** - Exported new handler functions
4. **api/src/extractors.rs** - Added WorkflowQueryService extractor
5. **api/src/routes.rs** - Added three query routes
6. **api/src/openapi.rs** - Added query schemas and paths

### Tests Created
1. **api/tests/workflow_query_tests.rs** - 13 integration tests covering all query endpoints

### Test Results
- All 13 workflow query integration tests pass
- All existing tests continue to pass
- Zero cargo warnings
- Zero cargo errors

### API Endpoints Implemented
1. `GET /api/v1/workflows/{workflow_id}` - Get workflow status and state
2. `GET /api/v1/workflows/{workflow_id}/activities` - List workflow activities
3. `GET /api/v1/workflows` - List workflows with filters and pagination

All endpoints require authentication and are documented in OpenAPI spec.

---

**Last Updated**: 2025-11-06
**Next Review**: After implementation complete
