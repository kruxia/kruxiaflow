# Implementation Plan: US-1A.5 Workflow Submission API

**Epic**: 1A - API Server
**User Story**: US-1A.5
**Status**: 📋 Planned (Updated to align with US-1A.4 patterns)
**Priority**: P0 (Must Have for MVP)
**Last Updated**: 2025-11-05

**⚠️ IMPORTANT**: This plan has been updated to align with architectural decisions from US-1A.4:
- WorkflowService follows repository pattern (cloneable, holds PgPool)
- Version format: `YYYYmmdd.HHMMSS.uuuuuu` (e.g., "20251105.143022.123456")
- No separate EventPublisher trait - events inserted directly via sqlx
- Service added directly to AppState (no Arc wrapping)
- Handlers extract service via State<AppState>

---

## User Story

**As** an AI startup engineer
**I want** to submit workflows via HTTP API
**So that** my applications can trigger workflows programmatically

### Acceptance Criteria

- `POST /api/v1/workflows` - Submit workflow with definition and input parameters
- Request body: `{definition_name, version, input, unique_key}` (JSON, optional version, optional unique_key)
- Response: `{workflow_id, status, created_at}` with 201 Created
- Workflow definition not found: 404 Not Found
- Validation: Reject invalid body or invalid input for the given workflow definition with 422 Unprocessable Entity
- Idempotency: Optional `unique_key` body parameter to prevent duplicate submissions: 409 Conflict
- Async execution: API creates workflow and workflow event then returns immediately, workflow runs in background

**Example**: `POST /api/v1/workflows` with `{"definition_name": "payment", "input": {"amount": 100.00}}`

---

## Rationale

This user story completes the core workflow submission flow, enabling clients to start workflow executions via the API. It connects the workflow definition management (US-1A.4) with the orchestration engine (US-1.2).

**Why This Story is Critical**:
- Enables programmatic workflow execution (core product functionality)
- Bridges workflow definitions with orchestration engine
- Provides idempotency for reliable workflow submission
- Validates inputs before execution (fail fast)
- Returns immediately (async execution pattern)

**Key Design Decisions**:
1. **Async Execution**: API creates workflow record and publishes event, then returns immediately
   - Orchestrator picks up event and evaluates workflow
   - Client polls workflow status via US-1A.6 (future)
   - Prevents API request timeout for long-running workflows
2. **Input Validation**: Validates JSON structure but not semantic validation (activities validate their own inputs)
3. **Idempotency**: Optional `unique_key` prevents duplicate submissions
   - Same `unique_key` twice returns 409 Conflict with original workflow_id
   - Use case: Retry on network failure without creating duplicate workflows
4. **Version Resolution**: If version not specified, uses latest version
   - Same resolution logic as US-1A.4 GET endpoint
   - Client can pin to specific version for reproducibility

---

## Migration Updates Required

**IMPORTANT**: This story requires updates to the existing `20251029000001_workflow_events.up.sql` migration to add missing columns needed for workflow submission.

### Required Migration Changes

1. **Update `workflow_status` enum** to include `'created'` state:
```sql
CREATE TYPE workflow_status AS ENUM (
    'created',    -- ADD THIS: For newly submitted workflows before orchestrator picks them up
    'running',
    'completed',
    'failed',
    'paused'
);
```

2. **Add columns to `workflows` table**:
```sql
CREATE TABLE workflows (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_type TEXT NOT NULL,  -- Keep for efficient queries
    workflow_definition_id UUID NOT NULL REFERENCES workflow_definitions(id),
    input JSONB NOT NULL,  -- ADD THIS: Original input parameters (immutable)
    unique_key TEXT UNIQUE,  -- ADD THIS: Optional idempotency key with unique constraint
    status workflow_status NOT NULL DEFAULT 'running',  -- Change default to 'created' for new workflows
    state_data JSONB NOT NULL,  -- Keep for runtime state (materialized by orchestrator)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Column Purposes

- **`input`**: Original input parameters provided at submission (immutable, for auditing/replay)
- **`state_data`**: Current runtime state including activity statuses (mutable, updated by orchestrator)
- **`workflow_type`**: Workflow definition name for efficient queries without JOIN
- **`workflow_definition_id`**: FK to the specific version used
- **`unique_key`**: Idempotency key to prevent duplicate submissions

### Code Updates Required

After migration update, these files need updates:

1. **`core/src/orchestrator/workflow_state.rs`**: Update state initialization to populate both `input` and initial `state_data`
2. **`core/tests/orchestrator_integration_tests.rs`**: Update test INSERT queries to include `input` column
3. **`core/src/events.rs`**: Add `'created'` to `WorkflowStatus` enum if using Rust enum

---

## Architecture Reference

Per `docs/architecture.md` (Workflow Submission Flow):
- Client submits workflow via POST /api/v1/workflows
- API validates definition exists and input structure
- API creates workflow record in `workflows` table with `status='created'` and initial `state_data`
- API publishes `WorkflowCreated` event to event stream
- API returns workflow_id immediately (async execution)
- Orchestrator picks up event, transitions to `status='running'`, and evaluates workflow
- Orchestrator schedules ready activities to activity queue

Per `docs/mvp-requirements.md` (Epic 1A, US-1A.5):
- Definition reference by name and optional version
- Input validation before workflow creation
- Idempotency via unique_key
- 201 Created on success
- 404 Not Found if definition doesn't exist
- 422 Unprocessable Entity for validation errors
- 409 Conflict for duplicate unique_key

**Database Schema** (updated for US-1A.5):
```sql
-- Workflows table (migration: 20251029000001_workflow_events.up.sql)
CREATE TABLE workflows (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_type TEXT NOT NULL,  -- Workflow definition name (for queries without JOIN)
    workflow_definition_id UUID NOT NULL REFERENCES workflow_definitions(id),  -- FK to specific version
    input JSONB NOT NULL,  -- Original input parameters (immutable)
    unique_key TEXT UNIQUE,  -- Optional idempotency key with unique constraint
    status workflow_status NOT NULL DEFAULT 'created',  -- 'created' | 'running' | 'completed' | 'failed' | 'paused'
    state_data JSONB NOT NULL,  -- Runtime state (updated by orchestrator)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_workflows_type_status ON workflows(workflow_type, status, created_at DESC);
CREATE INDEX idx_workflows_status ON workflows(status, updated_at DESC);
CREATE INDEX idx_workflows_definition_id ON workflows(workflow_definition_id);
```

**Event Schema** (already defined in migration):
```sql
CREATE TABLE workflow_events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    event_type workflow_event_type NOT NULL,
    activity_key TEXT,
    payload JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(workflow_id, event_type, activity_key)
);
```

---

## Implementation Components

### Component 1: Workflow Submission Request/Response Types

**Location**: `api/src/handlers/workflows.rs` (new file)

**Responsibilities**:
1. Define request/response types for workflow submission
2. Provide OpenAPI documentation
3. Request validation

**Implementation Notes**:
Following the patterns established in US-1A.4:
- Request/response types defined directly in handlers module (not DTOs, since workflow types use core DTOs)
- Use `ToSchema` for OpenAPI generation
- Validation follows the `ValidationErrors` pattern from `api/src/error.rs`
- Return types use `ApiResult<(StatusCode, Json<T>)>` pattern

**Implementation**:

```rust
use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{Extension, Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Submit workflow request
///
/// Creates a new workflow instance from a deployed workflow definition.
/// The workflow executes asynchronously; this endpoint returns immediately
/// with the workflow ID for status tracking.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SubmitWorkflowRequest {
    /// Workflow definition name (required)
    #[schema(example = "payment_processing")]
    pub definition_name: String,

    /// Workflow definition version (optional)
    /// If not provided, uses the latest deployed version.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "20251105.143022.123456")]
    pub version: Option<String>,

    /// Workflow input parameters (JSON object)
    /// Structure must match the workflow definition's expected inputs.
    #[schema(example = json!({"amount": 100.00, "card_token": "tok_123"}))]
    pub input: serde_json::Value,

    /// Unique idempotency key (optional)
    /// If provided, prevents duplicate workflow submissions.
    /// Submitting with the same unique_key twice returns 409 Conflict.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "order_12345_payment")]
    pub unique_key: Option<String>,
}

impl SubmitWorkflowRequest {
    /// Validate request structure
    ///
    /// Follows the ValidationErrors pattern from api/src/error.rs
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.definition_name.is_empty() {
            errors.add("definition_name", "Definition name cannot be empty");
        }

        // Validate input is an object (not array or primitive)
        if !self.input.is_object() {
            errors.add("input", "Input must be a JSON object");
        }

        // Validate unique_key format if provided
        if let Some(ref key) = self.unique_key {
            if key.is_empty() {
                errors.add("unique_key", "Unique key cannot be empty if provided");
            }
            if key.len() > 255 {
                errors.add("unique_key", "Unique key must be 255 characters or less");
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Submit workflow response
///
/// Returns immediately with workflow ID and initial status.
/// The workflow executes asynchronously in the background.
#[derive(Debug, Serialize, ToSchema)]
pub struct SubmitWorkflowResponse {
    /// Unique workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub workflow_id: uuid::Uuid,

    /// Workflow definition name
    #[schema(example = "payment_processing")]
    pub definition_name: String,

    /// Workflow definition version used
    #[schema(example = "20251105.143022.123456")]
    pub definition_version: String,

    /// Initial workflow status (always "created")
    #[schema(example = "created")]
    pub status: String,

    /// When the workflow was created
    #[schema(example = "2025-11-05T14:30:22.123456Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

**Key Features**:
- Clear request/response structure
- Optional version (defaults to latest) in format `YYYYmmdd.HHMMSS.uuuuuu`
- Optional unique_key for idempotency
- Input validation with field-level error messages
- OpenAPI documentation via utoipa
- Follows existing error handling patterns

---

### Component 2: Workflow Service Layer

**Location**: `core/src/workflow/service.rs` (new file)

**Responsibilities**:
1. Orchestrate workflow creation (validate definition exists, create workflow record, publish event)
2. Handle idempotency (unique_key collision detection)
3. Version resolution (latest if not specified)
4. Transaction management (workflow + event creation atomic)

**Implementation Notes**:
Following the repository pattern established in US-1A.4:
- Service struct holds PgPool (cloneable, cheap to clone)
- Service implements Clone for use directly in AppState
- Methods return `Result<T, WorkflowServiceError>`
- Error type uses thiserror with clear error variants
- No separate EventPublisher trait needed - events inserted directly via sqlx

**Implementation**:

```rust
use crate::workflow::repository::{WorkflowDefinitionRepository, RepositoryError};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Workflow service error
#[derive(Debug, Error)]
pub enum WorkflowServiceError {
    #[error("Workflow definition not found: {name} version {version}")]
    DefinitionNotFound { name: String, version: String },

    #[error("Workflow definition not found: {name} (no version specified)")]
    DefinitionNotFoundLatest { name: String },

    #[error("Duplicate workflow submission: unique_key '{0}' already exists")]
    DuplicateSubmission(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Repository error: {0}")]
    RepositoryError(#[from] RepositoryError),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type WorkflowServiceResult<T> = Result<T, WorkflowServiceError>;

/// Created workflow record
#[derive(Debug, Clone)]
pub struct CreatedWorkflow {
    pub id: Uuid,
    pub workflow_type: String,  // Workflow definition name
    pub workflow_definition_id: Uuid,  // FK to workflow_definitions
    pub definition_version: String,  // Formatted as YYYYmmdd.HHMMSS.uuuuuu
    pub input: Value,
    pub unique_key: Option<String>,
    pub status: String,  // Will be 'created'
    pub created_at: DateTime<Utc>,
}

/// Workflow service
///
/// Orchestrates workflow creation, validation, and event publishing.
/// Following the repository pattern - holds PgPool and is cloneable.
#[derive(Clone)]
pub struct WorkflowService {
    pool: PgPool,
}

impl WorkflowService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Submit a new workflow
    ///
    /// This method:
    /// 1. Resolves workflow definition (by name and optional version)
    /// 2. Validates input structure (basic JSON validation)
    /// 3. Checks for duplicate unique_key (idempotency)
    /// 4. Creates workflow record in database
    /// 5. Publishes WorkflowCreated event
    /// 6. Returns workflow ID immediately (async execution)
    ///
    /// All operations are atomic (transaction).
    pub async fn submit_workflow(
        &self,
        definition_name: &str,
        version: Option<&str>,
        input: Value,
        unique_key: Option<String>,
    ) -> WorkflowServiceResult<CreatedWorkflow> {
        // Create repository for definition lookup
        let repo = WorkflowDefinitionRepository::new(self.pool.clone());

        // Resolve workflow definition (latest if version not specified)
        let definition = if let Some(v) = version {
            repo.get(definition_name, v)
                .await
                .map_err(WorkflowServiceError::RepositoryError)?
                .ok_or_else(|| WorkflowServiceError::DefinitionNotFound {
                    name: definition_name.to_string(),
                    version: v.to_string(),
                })?
        } else {
            repo.get_latest(definition_name)
                .await
                .map_err(WorkflowServiceError::RepositoryError)?
                .ok_or_else(|| WorkflowServiceError::DefinitionNotFoundLatest {
                    name: definition_name.to_string(),
                })?
        };

        // Basic input validation (activities will validate semantically)
        Self::validate_input(&input)?;

        // Start transaction for atomic workflow + event creation
        let mut tx = self.pool.begin().await?;

        // Check for duplicate unique_key (idempotency)
        if let Some(ref key) = unique_key {
            let existing = sqlx::query!(
                r#"
                SELECT id, workflow_type, created_at
                FROM workflows
                WHERE unique_key = $1
                "#,
                key
            )
            .fetch_optional(&mut *tx)
            .await?;

            if existing.is_some() {
                return Err(WorkflowServiceError::DuplicateSubmission(key.clone()));
            }
        }

        // Create workflow record
        let workflow_id = Uuid::now_v7();
        let status = "created";

        // Initialize state_data with empty workflow state
        // Orchestrator will populate this when processing WorkflowCreated event
        let initial_state = serde_json::json!({
            "workflow_id": workflow_id,
            "workflow_type": definition.name,
            "status": status,
            "activities": {},
            "state_data": {}
        });

        let row = sqlx::query!(
            r#"
            INSERT INTO workflows (
                id, workflow_type, workflow_definition_id,
                input, unique_key, status, state_data,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6::workflow_status, $7, NOW(), NOW())
            RETURNING id, workflow_type, workflow_definition_id,
                      input, unique_key, status AS "status: String", created_at
            "#,
            workflow_id,
            definition.name,  // workflow_type
            definition.id,    // workflow_definition_id
            input,
            unique_key,
            status,
            initial_state
        )
        .fetch_one(&mut *tx)
        .await?;

        // Publish WorkflowCreated event
        let event_id = Uuid::now_v7();
        let event_type = "WorkflowCreated";
        let event_payload = serde_json::json!({
            "definition_name": definition.name,
            "definition_version": definition.version,
            "input": input
        });

        sqlx::query!(
            r#"
            INSERT INTO workflow_events (id, workflow_id, event_type, activity_key, payload, timestamp)
            VALUES ($1, $2, $3, $4, $5, NOW())
            "#,
            event_id,
            workflow_id,
            event_type,
            None::<String>,  // activity_key is None for WorkflowCreated events
            event_payload
        )
        .execute(&mut *tx)
        .await?;

        // Commit transaction
        tx.commit().await?;

        tracing::info!(
            workflow_id = %workflow_id,
            workflow_type = %definition.name,
            definition_version = %definition.version,
            "Workflow submitted successfully"
        );

        Ok(CreatedWorkflow {
            id: row.id,
            workflow_type: row.workflow_type,
            workflow_definition_id: row.workflow_definition_id,
            definition_version: definition.version,  // From resolved definition
            input: row.input,
            unique_key: row.unique_key,
            status: row.status,
            created_at: row.created_at,
        })
    }

    /// Validate input structure
    ///
    /// For MVP, this only validates that input is a JSON object.
    /// Activities will validate specific parameter requirements at execution time.
    fn validate_input(input: &Value) -> WorkflowServiceResult<()> {
        if !input.is_object() {
            return Err(WorkflowServiceError::InvalidInput(
                "Input must be a JSON object".to_string(),
            ));
        }

        Ok(())
    }
}
```

**Key Features**:
- Version resolution (latest if not specified)
- Idempotency check (unique_key collision detection)
- Atomic transaction (workflow + event creation)
- Clear error types with context
- Input validation (basic structure check)
- Event publishing integrated
- Logging for debugging

**Design Note**: Input validation is intentionally basic (JSON object check only). Activities validate their own parameter requirements at execution time, allowing workflows to be flexible about which parameters they provide.

---

### Component 3: API Handler

**Location**: `api/src/handlers/workflows.rs` (continued)

**Responsibilities**:
1. Handle HTTP POST /api/v1/workflows
2. Validate request body
3. Call workflow service
4. Map errors to HTTP status codes
5. Return response

**Implementation Notes**:
Following US-1A.4 handler patterns:
- Extract `WorkflowService` from `State<AppState>` (same as WorkflowDefinitionRepository)
- Use `Extension<ValidatedClaims>` for authentication
- Error mapping with match expressions following US-1A.4 patterns
- Return `ApiResult<(StatusCode, Json<Response>)>`
- Structured logging with field names

**Implementation**:

```rust
/// Submit workflow
///
/// Endpoint: POST /api/v1/workflows
///
/// Creates a new workflow instance from a deployed workflow definition.
/// The workflow executes asynchronously in the background; this endpoint
/// returns immediately with the workflow ID for status tracking.
///
/// Version Resolution:
/// - If `version` is provided, uses that specific version
/// - If `version` is omitted, uses the latest deployed version
///
/// Idempotency:
/// - If `unique_key` is provided, prevents duplicate submissions
/// - Submitting with the same `unique_key` twice returns 409 Conflict
///
/// Input Validation:
/// - Basic structure validation (must be JSON object)
/// - Activities validate their own parameter requirements at execution time
#[utoipa::path(
    post,
    path = "/api/v1/workflows",
    tag = "Workflows",
    request_body = SubmitWorkflowRequest,
    responses(
        (status = 201, description = "Workflow submitted successfully", body = SubmitWorkflowResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow definition not found"),
        (status = 409, description = "Duplicate submission (unique_key conflict)"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn submit_workflow(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<SubmitWorkflowRequest>,
) -> ApiResult<(StatusCode, Json<SubmitWorkflowResponse>)> {
    // Validate request structure
    request.validate().map_err(AppError::ValidationError)?;

    tracing::info!(
        definition_name = %request.definition_name,
        version = ?request.version,
        unique_key = ?request.unique_key,
        user = %claims.subject(),
        "Submitting workflow"
    );

    // Submit workflow via service
    let workflow = state
        .workflow_service
        .submit_workflow(
            &request.definition_name,
            request.version.as_deref(),
            request.input,
            request.unique_key,
        )
        .await
        .map_err(|e| match e {
            WorkflowServiceError::DefinitionNotFound { name, version } => {
                tracing::warn!("Workflow definition not found: {} version {}", name, version);
                AppError::NotFound(format!(
                    "Workflow definition '{}' version '{}' not found",
                    name, version
                ))
            }
            WorkflowServiceError::DefinitionNotFoundLatest { name } => {
                tracing::warn!("Workflow definition not found: {} (no version specified)", name);
                AppError::NotFound(format!(
                    "Workflow definition '{}' not found. No versions deployed.",
                    name
                ))
            }
            WorkflowServiceError::DuplicateSubmission(key) => {
                tracing::warn!("Duplicate workflow submission: unique_key '{}'", key);
                AppError::Conflict(format!(
                    "Workflow with unique_key '{}' already exists",
                    key
                ))
            }
            WorkflowServiceError::InvalidInput(msg) => {
                tracing::warn!("Invalid workflow input: {}", msg);
                let mut errors = ValidationErrors::new();
                errors.add("input", msg);
                AppError::ValidationError(errors)
            }
            WorkflowServiceError::DatabaseError(e) => {
                tracing::error!("Database error submitting workflow: {:?}", e);
                AppError::DatabaseError(e)
            }
            WorkflowServiceError::RepositoryError(e) => {
                tracing::error!("Repository error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
            WorkflowServiceError::SerializationError(e) => {
                tracing::error!("Serialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    tracing::info!(
        workflow_id = %workflow.id,
        workflow_type = %workflow.workflow_type,
        definition_version = %workflow.definition_version,
        "Workflow submitted successfully"
    );

    Ok((
        StatusCode::CREATED,
        Json(SubmitWorkflowResponse {
            workflow_id: workflow.id,
            definition_name: workflow.workflow_type,  // workflow_type is the definition name
            definition_version: workflow.definition_version,
            status: workflow.status,
            created_at: workflow.created_at,
        }),
    ))
}
```

**Key Features**:
- Extracts WorkflowService from AppState (following US-1A.4 repository pattern)
- Clear OpenAPI documentation
- Request validation before service call
- Error mapping following US-1A.4 patterns (AppError variants)
- Structured logging with field names
- Authentication via Extension<ValidatedClaims>

---

### Component 4: Update Application State

**Location**: Update `api/src/state.rs`

**Responsibilities**:
1. Add WorkflowService to application state
2. Initialize service with PgPool
3. Provide to handlers

**Implementation Notes**:
Following US-1A.4 pattern:
- WorkflowService is cloneable (holds PgPool)
- Add directly to AppState (no Arc needed)
- Handlers extract via State<AppState>

**Implementation**:

```rust
use core::workflow::repository::WorkflowDefinitionRepository;
use core::workflow::service::WorkflowService;
use oauth::{AuthenticationService, PostgresAuthService, AuthConfig};
use sqlx::PgPool;
use std::sync::Arc;

/// Application state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    pub db_pool: PgPool,
    pub auth_service: Arc<dyn AuthenticationService>,
    pub workflow_definition_repo: WorkflowDefinitionRepository,
    pub workflow_service: WorkflowService,
}

impl AppState {
    /// Create new application state
    pub async fn new(db_pool: PgPool, auth_config: AuthConfig) -> Self {
        // Initialize authentication service
        let auth_service = PostgresAuthService::new(db_pool.clone(), auth_config);

        // Initialize workflow definition repository
        let workflow_definition_repo = WorkflowDefinitionRepository::new(db_pool.clone());

        // Initialize workflow service
        let workflow_service = WorkflowService::new(db_pool.clone());

        Self {
            db_pool,
            auth_service: Arc::new(auth_service),
            workflow_definition_repo,
            workflow_service,
        }
    }
}
```

**Key Features**:
- WorkflowService initialized with PgPool only
- No Arc wrapping needed (WorkflowService is already cloneable)
- Follows the same pattern as WorkflowDefinitionRepository from US-1A.4

---

### Component 5: Update Routes

**Location**: Update `api/src/routes.rs`

**Responsibilities**:
1. Add workflow submission route to protected routes
2. Ensure authentication middleware applied

**Implementation**:

```rust
use crate::handlers;

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
            post(handlers::workflows::submit_workflow),
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
1. Add workflow submission endpoint to OpenAPI spec
2. Document request/response schemas

**Implementation**:

```rust
use crate::handlers::workflows::{SubmitWorkflowRequest, SubmitWorkflowResponse};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "StreamFlow API",
        version = "0.2.0",
        description = "High-performance workflow orchestration platform for AI-native workloads",
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server")
    ),
    paths(
        // ... existing paths ...

        // Workflow submission
        crate::handlers::workflows::submit_workflow,
    ),
    components(
        schemas(
            // ... existing schemas ...

            // Workflow submission schemas
            SubmitWorkflowRequest,
            SubmitWorkflowResponse,
        )
    ),
    tags(
        // ... existing tags ...
        (name = "Workflows", description = "Workflow submission and management"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;
```

---

### Component 7: Update Handlers Module

**Location**: Update `api/src/handlers/mod.rs`

**Responsibilities**:
1. Export workflow submission handlers
2. Make available to routes

**Implementation**:

```rust
pub mod health;
pub mod oauth;
pub mod workflow_definitions;
pub mod workflows;  // New module

// Re-export for convenience
pub use workflows::{submit_workflow};
```

---

### Component 8: Integration with Orchestrator

**Note**: The orchestrator integration is already implemented in US-1.2. When a WorkflowCreated event is published:

1. Orchestrator polls events from `workflow_events` table
2. Finds WorkflowCreated event
3. Loads workflow definition from `workflow_definitions` table
4. Evaluates workflow state (determines ready activities)
5. Schedules ready activities to activity queue
6. Workers poll activities and execute them

**No additional work needed** for US-1A.5 - the workflow submission endpoint publishes the event, and the orchestrator picks it up automatically.

---

## Testing Requirements

### Unit Tests

**File**: `core/src/workflow/service_test.rs`

**Test Scenarios**:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[tokio::test]
    async fn test_submit_workflow_latest_version() {
        let pool = test_db_pool().await;
        let service = test_workflow_service(pool.clone()).await;

        // Create test definition
        let def_name = "test_workflow";
        deploy_test_definition(&pool, def_name, "1.0").await;

        // Submit workflow (no version specified)
        let result = service
            .submit_workflow(
                def_name,
                None,
                serde_json::json!({"key": "value"}),
                None,
            )
            .await;

        assert!(result.is_ok());
        let workflow = result.unwrap();
        assert_eq!(workflow.definition_name, def_name);
        assert_eq!(workflow.definition_version, "1.0");
        assert_eq!(workflow.status, "created");
    }

    #[tokio::test]
    async fn test_submit_workflow_specific_version() {
        let pool = test_db_pool().await;
        let service = test_workflow_service(pool.clone()).await;

        // Create multiple versions
        let def_name = "test_workflow";
        deploy_test_definition(&pool, def_name, "1.0").await;
        deploy_test_definition(&pool, def_name, "2.0").await;

        // Submit workflow with version 1.0
        let result = service
            .submit_workflow(
                def_name,
                Some("1.0"),
                serde_json::json!({"key": "value"}),
                None,
            )
            .await;

        assert!(result.is_ok());
        let workflow = result.unwrap();
        assert_eq!(workflow.definition_version, "1.0");
    }

    #[tokio::test]
    async fn test_submit_workflow_definition_not_found() {
        let pool = test_db_pool().await;
        let service = test_workflow_service(pool.clone()).await;

        let result = service
            .submit_workflow(
                "nonexistent",
                None,
                serde_json::json!({"key": "value"}),
                None,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkflowServiceError::DefinitionNotFoundLatest { .. }
        ));
    }

    #[tokio::test]
    async fn test_submit_workflow_idempotency() {
        let pool = test_db_pool().await;
        let service = test_workflow_service(pool.clone()).await;

        let def_name = "test_workflow";
        deploy_test_definition(&pool, def_name, "1.0").await;

        let unique_key = "test_unique_key";

        // First submission
        let result1 = service
            .submit_workflow(
                def_name,
                None,
                serde_json::json!({"key": "value"}),
                Some(unique_key.to_string()),
            )
            .await;

        assert!(result1.is_ok());

        // Second submission with same unique_key
        let result2 = service
            .submit_workflow(
                def_name,
                None,
                serde_json::json!({"key": "value"}),
                Some(unique_key.to_string()),
            )
            .await;

        assert!(result2.is_err());
        assert!(matches!(
            result2.unwrap_err(),
            WorkflowServiceError::DuplicateSubmission(_)
        ));
    }

    #[tokio::test]
    async fn test_submit_workflow_invalid_input() {
        let pool = test_db_pool().await;
        let service = test_workflow_service(pool.clone()).await;

        let def_name = "test_workflow";
        deploy_test_definition(&pool, def_name, "1.0").await;

        // Submit with array instead of object
        let result = service
            .submit_workflow(
                def_name,
                None,
                serde_json::json!(["invalid"]),
                None,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkflowServiceError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_submit_workflow_publishes_event() {
        let pool = test_db_pool().await;
        let service = test_workflow_service(pool.clone()).await;

        let def_name = "test_workflow";
        deploy_test_definition(&pool, def_name, "1.0").await;

        let result = service
            .submit_workflow(
                def_name,
                None,
                serde_json::json!({"key": "value"}),
                None,
            )
            .await;

        assert!(result.is_ok());
        let workflow = result.unwrap();

        // Verify event was published
        let event = sqlx::query!(
            r#"
            SELECT event_type, payload
            FROM workflow_events
            WHERE workflow_id = $1 AND event_type = 'WorkflowCreated'
            "#,
            workflow.id
        )
        .fetch_one(&pool)
        .await
        .expect("Event should exist");

        assert_eq!(event.event_type, "WorkflowCreated");
        assert!(event.payload["definition_name"].is_string());
    }
}
```

---

### Integration Tests

**File**: `api/tests/workflow_submission_test.rs`

**Test Scenarios**:

```rust
#[tokio::test]
async fn test_submit_workflow_success() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Deploy workflow definition first
    let def_name = "test_workflow";
    deploy_test_definition(&app, def_name, "1.0").await;

    // Submit workflow
    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": def_name,
            "input": {"amount": 100.00, "card_token": "tok_123"}
        }))
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);

    let body: SubmitWorkflowResponse = response.json().await;
    assert_eq!(body.definition_name, def_name);
    // definition_version is in format YYYYmmdd.HHMMSS.uuuuuu
    assert!(!body.definition_version.is_empty());
    assert_eq!(body.status, "created");
    assert!(!body.workflow_id.to_string().is_empty());
}

#[tokio::test]
async fn test_submit_workflow_specific_version() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let def_name = "test_workflow";
    deploy_test_definition(&app, def_name, "1.0").await;
    deploy_test_definition(&app, def_name, "2.0").await;

    // Submit with version 1.0
    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": def_name,
            "version": "1.0",
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);

    let body: SubmitWorkflowResponse = response.json().await;
    // definition_version should match the version from the workflow definition
    assert!(!body.definition_version.is_empty());
}

#[tokio::test]
async fn test_submit_workflow_definition_not_found() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": "nonexistent",
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body: ApiErrorResponse = response.json().await;
    assert_eq!(body.error.code, ErrorCode::NotFound);
    assert!(body.error.message.contains("not found"));
}

#[tokio::test]
async fn test_submit_workflow_invalid_input() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let def_name = "test_workflow";
    deploy_test_definition(&app, def_name, "1.0").await;

    // Submit with array instead of object
    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": def_name,
            "input": ["invalid", "array"]
        }))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: ApiErrorResponse = response.json().await;
    assert_eq!(body.error.code, ErrorCode::ValidationError);
}

#[tokio::test]
async fn test_submit_workflow_idempotency() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let def_name = "test_workflow";
    deploy_test_definition(&app, def_name, "1.0").await;

    let unique_key = "test_unique_key_123";

    // First submission
    let response1 = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"},
            "unique_key": unique_key
        }))
        .await;

    assert_eq!(response1.status(), StatusCode::CREATED);
    let body1: SubmitWorkflowResponse = response1.json().await;

    // Second submission with same unique_key
    let response2 = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"},
            "unique_key": unique_key
        }))
        .await;

    assert_eq!(response2.status(), StatusCode::CONFLICT);

    let body2: ApiErrorResponse = response2.json().await;
    assert_eq!(body2.error.code, ErrorCode::Conflict);
    assert!(body2.error.message.contains(unique_key));
}

#[tokio::test]
async fn test_submit_workflow_requires_authentication() {
    let app = test_app().await;

    let response = app
        .post("/api/v1/workflows")
        .json(&json!({
            "definition_name": "test",
            "input": {}
        }))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_submit_workflow_missing_definition_name() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "input": {"key": "value"}
            // Missing definition_name
        }))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_submit_workflow_empty_definition_name() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": "",
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_submit_workflow_event_published() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let def_name = "test_workflow";
    deploy_test_definition(&app, def_name, "1.0").await;

    let response = app
        .post("/api/v1/workflows")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"}
        }))
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: SubmitWorkflowResponse = response.json().await;

    // Verify event was published
    let event = sqlx::query!(
        r#"
        SELECT event_type, workflow_id
        FROM workflow_events
        WHERE workflow_id = $1 AND event_type = 'WorkflowCreated'
        "#,
        body.workflow_id
    )
    .fetch_one(&app.db_pool)
    .await
    .expect("WorkflowCreated event should exist");

    assert_eq!(event.event_type, "WorkflowCreated");
}
```

---

## Dependencies

### Existing Dependencies

All required dependencies should already be available:
- `sqlx` - Database access
- `serde` / `serde_json` - Serialization
- `uuid` - UUID generation
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
## Workflow Submission

Submit a workflow for asynchronous execution.

### Submit Workflow

**Endpoint**: `POST /api/v1/workflows`

**Authentication**: Required (Bearer token)

**Request Body**:
\`\`\`json
{
  "definition_name": "payment_processing",
  "version": "20251105143022",  // Optional, uses latest if not provided
  "input": {
    "amount": 100.00,
    "card_token": "tok_123"
  },
  "unique_key": "order_12345_payment"  // Optional idempotency key
}
\`\`\`

**Response** (201 Created):
\`\`\`json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "definition_name": "payment_processing",
  "definition_version": "20251105143022",
  "status": "created",
  "created_at": "2025-11-05T14:30:22.123456Z",
  "message": "Workflow 'payment_processing' version '20251105143022' submitted successfully"
}
\`\`\`

**Version Resolution**:
- If `version` is provided, uses that specific version
- If `version` is omitted, uses the latest deployed version

**Idempotency**:
- If `unique_key` is provided, prevents duplicate submissions
- Submitting with the same `unique_key` twice returns 409 Conflict

**Error Responses**:
- `404 Not Found` - Workflow definition not found
- `409 Conflict` - Duplicate submission (unique_key conflict)
- `422 Unprocessable Entity` - Validation error

**Async Execution**:
The workflow executes asynchronously in the background. This endpoint returns
immediately with the workflow ID. Use the workflow status API (US-1A.6) to
track execution progress.

**Example**:
\`\`\`bash
curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9..." \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "payment_processing",
    "input": {
      "amount": 100.00,
      "card_token": "tok_123"
    }
  }'
\`\`\`
```

---

## Success Criteria

### Functional Requirements

- ✅ `POST /api/v1/workflows` submits workflow with definition and input
- ✅ Returns 201 Created with workflow_id, status, created_at
- ✅ Returns 404 Not Found if definition doesn't exist
- ✅ Returns 422 Unprocessable Entity for validation errors
- ✅ Returns 409 Conflict for duplicate unique_key
- ✅ Async execution (returns immediately, orchestrator picks up event)
- ✅ Version resolution (uses latest if not specified)
- ✅ Idempotency support via unique_key
- ✅ Authentication required (Bearer token)
- ✅ WorkflowCreated event published to event stream

### Non-Functional Requirements

- ✅ Atomic transaction (workflow + event creation)
- ✅ Clear error messages with field-level details
- ✅ OpenAPI documentation for endpoint
- ✅ Structured logging for debugging
- ✅ Input validation before workflow creation
- ✅ Efficient database queries (<5ms workflow creation)

---

## Implementation Phases

### Phase 1: Data Models and Service Layer (P0)
- Implement SubmitWorkflowRequest/Response types
- Implement WorkflowService with submit_workflow method
- Events published directly via sqlx (no separate EventPublisher)
- Unit tests for service layer
- **Estimated Time**: 3 hours

### Phase 2: API Handler (P0)
- Implement POST /api/v1/workflows handler
- Error mapping to HTTP status codes
- Request validation
- Integration with workflow service
- **Estimated Time**: 2 hours

### Phase 3: Application State and Routes (P0)
- Add WorkflowService to AppState
- Add workflow submission route to protected routes
- Update OpenAPI documentation
- **Estimated Time**: 1 hour

### Phase 4: Integration Tests (P0)
- Test successful workflow submission
- Test version resolution (latest and specific)
- Test definition not found
- Test invalid input validation
- Test idempotency (unique_key)
- Test authentication required
- Test event publishing
- **Estimated Time**: 3 hours

### Phase 5: End-to-End Testing (P0)
- Test workflow submission → orchestrator pickup → activity scheduling
- Verify events flow correctly
- Manual testing with curl/Postman
- Update documentation
- **Estimated Time**: 2 hours

**Total Estimated Time**: 11 hours

---

## Risks and Mitigations

### Risk 1: Event Publishing Failure After Workflow Creation

**Probability**: Low
**Impact**: High (workflow created but orchestrator doesn't see it)

**Mitigation**:
- Use database transaction for atomic workflow + event creation
- If event publishing fails, transaction rolls back (no orphaned workflow)
- Clear error message to client (can retry safely)
- Consider idempotency (unique_key) for retry safety

### Risk 2: Duplicate Event Publishing

**Probability**: Low
**Impact**: Medium (orchestrator processes workflow twice)

**Mitigation**:
- Orchestrator idempotency (already implemented in US-1.2)
- Event ID (UUID v7) prevents duplicate processing
- Activity queue UNIQUE constraint prevents duplicate scheduling

### Risk 3: Input Validation Complexity

**Probability**: Medium
**Impact**: Low (validation errors at execution time instead of submission)

**Mitigation**:
- MVP: Basic input validation (JSON object check)
- Activities validate their own parameter requirements
- Clear error messages when activities fail validation
- Post-MVP: Schema-based input validation (JSON Schema)

### Risk 4: Version Resolution Confusion

**Probability**: Medium
**Impact**: Low (user submits with wrong version)

**Mitigation**:
- Clear documentation on version resolution behavior
- Response includes actual version used
- Logging shows resolved version
- Recommend explicit version for production workflows

---

## Future Enhancements (Post-MVP)

### Schema-Based Input Validation
- Define input schema in workflow definition
- Validate input structure at submission time
- Clear validation error messages with field paths
- Prevents execution of workflows with invalid inputs

### Workflow Templates
- Pre-fill common input parameters
- Simplify workflow submission
- Reduce client complexity

### Batch Workflow Submission
- Submit multiple workflows in single request
- Atomic batch creation (all or nothing)
- Efficient for bulk operations

### Workflow Tags/Labels
- Add metadata to workflows at submission time
- Filter/search by tags in workflow query API
- Organize workflows by project, environment, etc.

### Workflow Input Size Limits
- Prevent very large input payloads (>1MB)
- Suggest artifact storage for large data
- Configurable limit per deployment

---

## Related User Stories

- **US-1A.3**: Authentication (provides auth middleware for endpoint)
- **US-1A.4**: Workflow Definition Management API (defines workflow definitions)
- **US-1.2**: Event-Driven Dynamic Scheduling (orchestrator picks up WorkflowCreated events)
- **US-1A.6**: Workflow Status and Query API (query submitted workflow status)

---

## References

- Architecture: `docs/architecture.md` (Workflow Submission Flow)
- Requirements: `docs/mvp-requirements.md` (Epic 1A, US-1A.5)
- US-1.2 Implementation: `docs/implementation/US-1.2-event-driven-scheduling.md`
- US-1A.4 Implementation: `docs/implementation/US-1A.4-workflow-definition-management.md`

---

## Implementation Notes

**⚠️ Schema Correction Summary**:

This implementation plan includes critical corrections to the `workflows` table schema discovered during planning:

1. **Column Names**: Uses `workflow_definition_id` (not `definition_id`) to match existing code
2. **Dual Purpose Columns**:
   - `input`: Stores original submission parameters (immutable, for audit/replay)
   - `state_data`: Stores runtime state updated by orchestrator (mutable)
3. **Workflow Type**: Adds `workflow_type` column (definition name) for efficient queries without JOINs
4. **Idempotency**: Adds `unique_key` column with unique constraint for duplicate prevention
5. **Status Enum**: Adds `'created'` state to `workflow_status` enum for newly submitted workflows
6. **Default State**: Sets default status to `'created'` (not `'running'`)

These changes require updating the migration file and several existing code files. See [Migration Updates Required](#migration-updates-required) for details.

**Implementation Order**:
1. **Update migration file** (20251029000001_workflow_events.up.sql) with schema corrections (if needed)
2. **Update existing code** to handle new columns (core/src/orchestrator/workflow_state.rs, tests) if needed
3. Data models (request/response types, service errors)
4. WorkflowService with submit_workflow method (events inserted directly via sqlx)
5. API handler with error mapping
6. Application state and routes
7. Integration tests
8. End-to-end testing and documentation

**Key Design Decisions**:
1. **Follows US-1A.4 Patterns**: WorkflowService mirrors WorkflowDefinitionRepository design
   - Cloneable struct holding PgPool
   - Added directly to AppState (no Arc)
   - Handlers extract via State<AppState>
   - Version format: YYYYmmdd.HHMMSS.uuuuuu
2. **Async Execution**: API returns immediately, orchestrator processes asynchronously
   - Prevents API timeout for long-running workflows
   - Enables horizontal scaling (multiple orchestrators)
   - Client polls status via US-1A.6 (future story)
3. **Basic Input Validation**: Only validates JSON structure, not semantics
   - Activities validate their own parameter requirements
   - Allows flexible workflow definitions
   - Clear error messages at execution time
4. **Atomic Transaction**: Workflow + event creation in single transaction
   - Prevents orphaned workflows
   - Safe retry on failure
   - Consistent state
5. **Idempotency**: Optional unique_key prevents duplicate submissions
   - Safe retry on network failure
   - 409 Conflict with original workflow_id
   - Client can decide whether to use idempotency
6. **Version Resolution**: Latest if not specified, explicit if provided
   - Simplifies client code (no version management for dev/test)
   - Production can pin to specific version
   - Response includes actual version used
7. **Separation of Input and State**: Keeps original input separate from runtime state
   - `input`: Immutable original parameters (for audit trail, replay, debugging)
   - `state_data`: Mutable runtime state (optimized for orchestrator O(1) access)
   - Both stored in JSONB for flexibility
8. **No EventPublisher Abstraction**: Events inserted directly via sqlx
   - Simpler implementation for MVP
   - Can extract to trait later if needed for alternative event stores

**Post-Implementation**:
- US-1A.6 will provide workflow status query
- US-1A.7 will enable workers to poll activities
- US-1A.8 will enable retrieving activity outputs
- US-1A.9 will provide real-time workflow updates via WebSocket

---

## Testing Plan

### Phase 1: Unit Tests (Service Layer)
- Workflow submission with latest version
- Workflow submission with specific version
- Definition not found errors
- Invalid input validation
- Idempotency (duplicate unique_key)
- Event publishing

### Phase 2: Integration Tests (API Endpoints)
- HTTP request/response validation
- Authentication required
- Error HTTP status codes (404, 409, 422)
- OpenAPI documentation generation

### Phase 3: End-to-End Tests
- Submit workflow → Orchestrator pickup → Activity scheduling
- Multiple workflows in parallel
- Idempotency across retries
- Event stream flow

### Phase 4: Manual Testing
- Test with curl/Postman
- Test with different workflow definitions
- Test error scenarios
- Test with real orchestrator running

---

## Pending Requirements from Previous Stories

### Resolved in This Story

1. **Workflow Creation**: US-1.2 defined workflows table but didn't provide API to create workflows
   - ✅ **Resolution**: US-1A.5 provides POST /api/v1/workflows endpoint

2. **Event Publishing API**: US-1.2 defined event stream but workflows could only be created manually
   - ✅ **Resolution**: WorkflowService publishes WorkflowCreated event automatically

3. **Input Validation**: Workflow definitions exist but no validation of input structure
   - ✅ **Resolution**: Basic input validation (JSON object) at submission time
   - ⏳ **Post-MVP**: Schema-based validation using JSON Schema (see Future Enhancements)

### Still Pending (Future Stories)

1. **Workflow Status Query**: Can submit workflow but can't query status
   - 📅 **US-1A.6**: Workflow Status and Query API will provide GET /api/v1/workflows/{id}

2. **Activity Output Retrieval**: Can't retrieve workflow results
   - 📅 **US-1A.8**: Activity Results and Output Retrieval will provide output access

3. **Real-Time Updates**: No way to watch workflow progress
   - 📅 **US-1A.9**: WebSocket Streaming will provide real-time workflow events

4. **Workflow Cancellation**: No way to stop running workflows
   - 📅 **Post-MVP** (Story 6.6 in post-mvp.md): Workflow cancellation API

---

## Definition of Done

- [ ] WorkflowService implemented with submit_workflow method
- [ ] Events published directly via sqlx (no separate EventPublisher trait)
- [ ] POST /api/v1/workflows handler implemented
- [ ] Request/response validation implemented
- [ ] Error mapping to HTTP status codes complete
- [ ] Application state includes WorkflowService
- [ ] Routes include workflow submission endpoint
- [ ] OpenAPI documentation updated
- [ ] Unit tests passing (service layer)
- [ ] Integration tests passing (API endpoints)
- [ ] End-to-end tests passing (workflow submission → orchestrator)
- [ ] Manual testing complete
- [ ] Documentation updated (API reference)
- [ ] Code reviewed and approved
- [ ] All acceptance criteria met
- [ ] Zero cargo warnings
- [ ] Test coverage >85%
- [ ] Aligns with US-1A.4 architectural patterns

---

**Last Updated**: 2025-11-05
**Next Review**: After implementation complete
