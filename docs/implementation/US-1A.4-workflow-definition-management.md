# Implementation Plan: US-1A.4 Workflow Definition Management API

**Epic**: 1A - API Server
**User Story**: US-1A.4
**Status**: 📝 Planning
**Priority**: P0 (Must Have for MVP)

---

## User Story

**As** an AI startup engineer
**I want** to deploy and manage workflow definitions separately from execution
**So that** I can version and reuse workflow templates

### Acceptance Criteria

- `POST /api/v1/workflow_definitions` - Deploy workflow definition with name (version auto-generated)
- `GET /api/v1/workflow_definitions` - List all deployed definitions (later: Search)
- `GET /api/v1/workflow_definitions/{name}` - Get latest version of definition
- `GET /api/v1/workflow_definitions/{name}?version={version}` - Get specific version
- Versioning: Timestamp-based (auto-generated at deployment: `YYYYMMDDHHmmss`)
- Validation on deployment: Syntax and semantic checks before storage

---

## Rationale

This user story establishes the workflow definition management system that enables:

1. **Separation of Concerns**: Workflow definitions deployed independently from execution
2. **Versioning**: Multiple versions of the same workflow template can coexist
3. **Reusability**: Deploy once, execute many times with different inputs
4. **Validation**: Syntax and semantic checks prevent invalid workflows from being stored
5. **Auditing**: Track what workflow definitions are available and when they were deployed

**Why This Story is Critical**:
- Foundation for workflow execution (US-1A.5 depends on this)
- Enables GitOps workflows (definitions in version control → deployed via API)
- Allows rollback to previous versions if issues discovered
- Supports A/B testing (deploy two versions, route traffic appropriately)
- Essential for multi-tenant deployments (each tenant can have their own definitions)

**Why Auto-Generated Timestamps**:
- ✅ **Simplicity** - Users don't need to manage version numbers
- ✅ **No conflicts** - Timestamp uniqueness prevents duplicate version errors
- ✅ **Natural ordering** - Sortable by version gives chronological order
- ✅ **Audit trail** - Version timestamp shows exact deployment time

**Why Validation is Essential**:
- ✅ **Fail fast** - Catch errors at deployment time, not execution time
- ✅ **Cost savings** - Invalid workflows rejected before consuming resources
- ✅ **User experience** - Clear error messages with line numbers and specific issues
- ✅ **Security** - Prevent malicious workflow definitions
- ✅ **Consistency** - Ensure all deployed workflows are valid

---

## Architecture Reference

Per `docs/architecture.md` (Data Architecture - Workflow Definitions):
- `workflow_definitions` table stores definition name, version, and YAML/JSON content
- Unique constraint on (name, version) prevents duplicate deployments
- Definitions stored as JSONB for efficient querying and validation

Per `docs/mvp-requirements.md` (Epic 1A, US-1A.4):
- Versioning is timestamp-based and auto-generated at deployment (format: `YYYYMMDDHHmmss`)
- Validation includes syntax checking (valid YAML) and semantic checking (valid activity references)
- List endpoint returns all definitions (search/filtering is post-MVP)

**Database Schema** (from architecture.md):
```sql
CREATE TABLE workflow_definitions (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    definition JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(name, version)
);
```

**Workflow Definition Structure**:
Per architecture.md, workflows are defined as directed graphs using `preceding` and `following` relationships:

```yaml
name: payment_processing
description: "Process payment with validation and authorization"

activities:
  - key: validate_payment
    namespace: payments
    name: validate_card
    parameters:
      card_token: "{{ARG.card_token}}"
    following:
      - activity_key: authorize_card
        conditions:
          - "{{validate_payment.valid}} == true"

  - key: authorize_card
    namespace: payments
    name: authorize
    parameters:
      amount: "{{ARG.amount}}"
    following:
      - activity_key: capture_payment

  - key: capture_payment
    namespace: payments
    name: capture
    parameters:
      authorization_id: "{{authorize_card.authorization_id}}"
```

---

## Implementation Components

### Component 1: Workflow Definition Data Model

**Location**: `core/src/workflow/definition.rs`

**Responsibilities**:
1. Define WorkflowDefinition struct matching YAML structure
2. Provide serialization/deserialization to/from JSONB
3. Validate definition structure
4. Generate timestamp version on deployment

**Implementation**:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Workflow definition (user-provided, without version)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Workflow name (unique per version)
    pub name: String,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Activities in the workflow
    pub activities: Vec<ActivityDefinition>,

    /// Workflow-level settings (timeouts, retries, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<WorkflowSettings>,
}

impl WorkflowDefinition {
    /// Generate timestamp-based version
    ///
    /// Format: YYYYMMDDHHmmss (e.g., "20250105143022")
    /// Uses UTC time for consistency across timezones.
    pub fn generate_version() -> String {
        use chrono::Utc;
        Utc::now().format("%Y%m%d%H%M%S").to_string()
    }
}

/// Activity definition within a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityDefinition {
    /// Unique key for this activity within the workflow
    pub key: String,

    /// Activity namespace (e.g., "payments", "llm")
    pub namespace: String,

    /// Activity name within namespace (e.g., "authorize", "complete")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Activity parameters (can include template expressions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, serde_json::Value>>,

    /// Activities that must complete before this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preceding: Option<Vec<ActivityRelationship>>,

    /// Activities that should run after this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following: Option<Vec<ActivityRelationship>>,

    /// Activity-level settings (timeout, retry, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActivitySettings>,
}

/// Relationship between activities (edge in the directed graph)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityRelationship {
    /// Key of the related activity
    pub activity_key: String,

    /// Optional conditions that must be satisfied for this edge to activate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<String>>,
}

/// Workflow-level settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSettings {
    /// Maximum workflow execution time in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Maximum retry attempts for transient failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

/// Activity-level settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySettings {
    /// Activity timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Retry configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetrySettings>,
}

/// Retry configuration for activities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySettings {
    /// Maximum retry attempts
    pub max_attempts: u32,

    /// Backoff strategy
    #[serde(default = "default_backoff")]
    pub backoff: BackoffStrategy,
}

fn default_backoff() -> BackoffStrategy {
    BackoffStrategy::Exponential
}

/// Backoff strategy for retries
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Exponential backoff (delay doubles each retry)
    Exponential,
}

impl WorkflowDefinition {
    /// Validate workflow definition structure
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut errors = ValidationErrors::new();

        // Validate workflow name
        if self.name.is_empty() {
            errors.add("name", "Workflow name cannot be empty");
        }
        if !is_valid_identifier(&self.name) {
            errors.add("name", "Workflow name must be a valid identifier (alphanumeric, hyphens, underscores)");
        }

        // Validate activities
        if self.activities.is_empty() {
            errors.add("activities", "Workflow must have at least one activity");
        }

        // Check for duplicate activity keys
        let mut activity_keys = std::collections::HashSet::new();
        for (idx, activity) in self.activities.iter().enumerate() {
            if !activity_keys.insert(&activity.key) {
                errors.add(
                    &format!("activities[{}].key", idx),
                    &format!("Duplicate activity key: {}", activity.key),
                );
            }

            // Validate activity structure
            if let Err(e) = self.validate_activity(activity, idx) {
                errors.merge(e);
            }
        }

        // Validate graph structure (no cycles, valid references)
        if let Err(e) = self.validate_graph(&activity_keys) {
            errors.merge(e);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationError::MultipleErrors(errors))
        }
    }

    /// Validate individual activity
    fn validate_activity(
        &self,
        activity: &ActivityDefinition,
        idx: usize,
    ) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // Validate activity key
        if activity.key.is_empty() {
            errors.add(&format!("activities[{}].key", idx), "Activity key cannot be empty");
        }
        if !is_valid_identifier(&activity.key) {
            errors.add(
                &format!("activities[{}].key", idx),
                "Activity key must be a valid identifier",
            );
        }

        // Validate namespace
        if activity.namespace.is_empty() {
            errors.add(
                &format!("activities[{}].namespace", idx),
                "Activity namespace cannot be empty",
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate directed graph structure
    fn validate_graph(
        &self,
        activity_keys: &std::collections::HashSet<&String>,
    ) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // Build adjacency list for cycle detection
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
        for activity in &self.activities {
            graph.entry(activity.key.as_str()).or_insert_with(Vec::new);
        }

        // Validate all activity references
        for activity in &self.activities {
            // Validate preceding references
            if let Some(preceding) = &activity.preceding {
                for rel in preceding {
                    if !activity_keys.contains(&rel.activity_key) {
                        errors.add(
                            &format!("activity.{}.preceding", activity.key),
                            &format!("Referenced activity not found: {}", rel.activity_key),
                        );
                    } else {
                        // Add edge: preceding -> current
                        graph
                            .get_mut(rel.activity_key.as_str())
                            .unwrap()
                            .push(activity.key.as_str());
                    }
                }
            }

            // Validate following references
            if let Some(following) = &activity.following {
                for rel in following {
                    if !activity_keys.contains(&rel.activity_key) {
                        errors.add(
                            &format!("activity.{}.following", activity.key),
                            &format!("Referenced activity not found: {}", rel.activity_key),
                        );
                    } else {
                        // Add edge: current -> following
                        graph
                            .get_mut(activity.key.as_str())
                            .unwrap()
                            .push(rel.activity_key.as_str());
                    }
                }
            }
        }

        // Detect cycles using DFS
        if let Some(cycle) = detect_cycle(&graph) {
            errors.add(
                "activities",
                &format!("Workflow contains a cycle: {}", cycle.join(" -> ")),
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Check if string is valid identifier (alphanumeric, hyphens, underscores)
fn is_valid_identifier(s: &str) -> bool {
    let re = regex::Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
    re.is_match(s)
}

/// Detect cycles in directed graph using DFS
fn detect_cycle<'a>(graph: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<String>> {
    let mut visited = std::collections::HashSet::new();
    let mut rec_stack = std::collections::HashSet::new();
    let mut path = Vec::new();

    for node in graph.keys() {
        if !visited.contains(node) {
            if let Some(cycle) = dfs_cycle(node, graph, &mut visited, &mut rec_stack, &mut path) {
                return Some(cycle);
            }
        }
    }

    None
}

/// DFS helper for cycle detection
fn dfs_cycle<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut std::collections::HashSet<&'a str>,
    rec_stack: &mut std::collections::HashSet<&'a str>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    visited.insert(node);
    rec_stack.insert(node);
    path.push(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for &neighbor in neighbors {
            if !visited.contains(neighbor) {
                if let Some(cycle) = dfs_cycle(neighbor, graph, visited, rec_stack, path) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(neighbor) {
                // Found cycle - return path from neighbor to current node
                let cycle_start = path.iter().position(|n| n == neighbor).unwrap();
                return Some(path[cycle_start..].to_vec());
            }
        }
    }

    rec_stack.remove(node);
    path.pop();
    None
}

/// Validation errors
#[derive(Debug, Clone)]
pub struct ValidationErrors {
    errors: HashMap<String, Vec<String>>,
}

impl ValidationErrors {
    pub fn new() -> Self {
        Self {
            errors: HashMap::new(),
        }
    }

    pub fn add(&mut self, field: &str, message: &str) {
        self.errors
            .entry(field.to_string())
            .or_insert_with(Vec::new)
            .push(message.to_string());
    }

    pub fn merge(&mut self, other: ValidationErrors) {
        for (field, messages) in other.errors {
            self.errors
                .entry(field)
                .or_insert_with(Vec::new)
                .extend(messages);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "field_errors": self.errors
        })
    }
}

/// Validation error
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Validation failed: {0}")]
    SingleError(String),

    #[error("Multiple validation errors")]
    MultipleErrors(ValidationErrors),
}
```

**Key Features**:
- Matches YAML structure from architecture.md (directed graph with preceding/following)
- Validates version format (semantic or timestamp)
- Validates graph structure (no cycles, valid references)
- Validates activity keys are unique
- Returns detailed field-level validation errors
- Supports both simple and complex validation rules

---

### Component 2: Workflow Definition Repository

**Location**: `core/src/workflow/repository.rs`

**Responsibilities**:
1. Store workflow definitions in PostgreSQL
2. Retrieve definitions by name and version
3. List all definitions
4. Enforce unique constraint on (name, version)

**Implementation**:

```rust
use crate::workflow::definition::{WorkflowDefinition, ValidationError};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Stored workflow definition record
#[derive(Debug, Clone)]
pub struct StoredWorkflowDefinition {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub definition: WorkflowDefinition,
    pub created_at: DateTime<Utc>,
}

/// Workflow definition repository
pub struct WorkflowDefinitionRepository {
    pool: PgPool,
}

impl WorkflowDefinitionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Store workflow definition
    ///
    /// Auto-generates timestamp-based version at deployment time.
    /// Returns error if definition with same (name, version) already exists (highly unlikely).
    pub async fn store(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<StoredWorkflowDefinition, RepositoryError> {
        // Validate definition before storing
        definition.validate()
            .map_err(RepositoryError::ValidationError)?;

        let id = Uuid::now_v7();
        let version = WorkflowDefinition::generate_version();
        let definition_json = serde_json::to_value(&definition)?;

        let row = sqlx::query!(
            r#"
            INSERT INTO workflow_definitions (id, name, version, definition, created_at)
            VALUES ($1, $2, $3, $4, NOW())
            RETURNING id, name, version, definition, created_at
            "#,
            id,
            definition.name,
            version,
            definition_json
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // Check for unique constraint violation (extremely rare with timestamp versions)
            if let Some(db_err) = e.as_database_error() {
                if db_err.is_unique_violation() {
                    return RepositoryError::DuplicateVersion {
                        name: definition.name.clone(),
                        version: version.clone(),
                    };
                }
            }
            RepositoryError::DatabaseError(e)
        })?;

        Ok(StoredWorkflowDefinition {
            id: row.id,
            name: row.name,
            version: row.version,
            definition: serde_json::from_value(row.definition)?,
            created_at: row.created_at,
        })
    }

    /// Get workflow definition by name and version
    pub async fn get(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<StoredWorkflowDefinition>, RepositoryError> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, version, definition, created_at
            FROM workflow_definitions
            WHERE name = $1 AND version = $2
            "#,
            name,
            version
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StoredWorkflowDefinition {
            id: r.id,
            name: r.name,
            version: r.version,
            definition: serde_json::from_value(r.definition).unwrap(),
            created_at: r.created_at,
        }))
    }

    /// Get latest version of workflow definition by name
    pub async fn get_latest(
        &self,
        name: &str,
    ) -> Result<Option<StoredWorkflowDefinition>, RepositoryError> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, version, definition, created_at
            FROM workflow_definitions
            WHERE name = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StoredWorkflowDefinition {
            id: r.id,
            name: r.name,
            version: r.version,
            definition: serde_json::from_value(r.definition).unwrap(),
            created_at: r.created_at,
        }))
    }

    /// List all workflow definitions
    ///
    /// Returns all versions of all workflows.
    /// Post-MVP: Add filtering, pagination, search.
    pub async fn list(&self) -> Result<Vec<StoredWorkflowDefinition>, RepositoryError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name, version, definition, created_at
            FROM workflow_definitions
            ORDER BY name ASC, created_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| StoredWorkflowDefinition {
                id: r.id,
                name: r.name,
                version: r.version,
                definition: serde_json::from_value(r.definition).unwrap(),
                created_at: r.created_at,
            })
            .collect())
    }

    /// List all versions of a specific workflow
    pub async fn list_versions(
        &self,
        name: &str,
    ) -> Result<Vec<StoredWorkflowDefinition>, RepositoryError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name, version, definition, created_at
            FROM workflow_definitions
            WHERE name = $1
            ORDER BY created_at DESC
            "#,
            name
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| StoredWorkflowDefinition {
                id: r.id,
                name: r.name,
                version: r.version,
                definition: serde_json::from_value(r.definition).unwrap(),
                created_at: r.created_at,
            })
            .collect())
    }
}

/// Repository errors
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Validation error: {0}")]
    ValidationError(#[from] ValidationError),

    #[error("Workflow definition already exists: {name} version {version}")]
    DuplicateVersion { name: String, version: String },

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
```

**Key Features**:
- Validates definitions before storing
- Enforces unique constraint on (name, version)
- Provides clear error messages for duplicate deployments
- Efficient queries with proper indexing
- Separates storage logic from API handlers

---

### Component 3: API Handlers

**Location**: `api/src/handlers/workflow_definitions.rs`

**Responsibilities**:
1. Handle HTTP requests for workflow definition management
2. Parse and validate request payloads
3. Return appropriate HTTP status codes and responses
4. Apply authentication middleware

**Implementation**:

```rust
use crate::error::{AppError, ApiResult};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use core::workflow::definition::WorkflowDefinition;
use core::workflow::repository::RepositoryError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Deploy workflow definition request
///
/// Note: Version is auto-generated at deployment time (timestamp-based).
/// Users only provide the workflow name and definition.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeployWorkflowDefinitionRequest {
    /// Workflow definition (version auto-generated)
    #[serde(flatten)]
    pub definition: WorkflowDefinition,
}

/// Deploy workflow definition response
#[derive(Debug, Serialize, ToSchema)]
pub struct DeployWorkflowDefinitionResponse {
    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// When the definition was deployed
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Message
    pub message: String,
}

/// List workflow definitions response
#[derive(Debug, Serialize, ToSchema)]
pub struct ListWorkflowDefinitionsResponse {
    /// All workflow definitions
    pub definitions: Vec<WorkflowDefinitionSummary>,

    /// Total count
    pub total: usize,
}

/// Workflow definition summary (without full definition body)
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowDefinitionSummary {
    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Number of activities
    pub activity_count: usize,

    /// When deployed
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Get workflow definition response
#[derive(Debug, Serialize, ToSchema)]
pub struct GetWorkflowDefinitionResponse {
    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// Full workflow definition
    pub definition: WorkflowDefinition,

    /// When deployed
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Query parameters for getting workflow definition
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetWorkflowDefinitionQuery {
    /// Specific version to retrieve (if not provided, returns latest)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Deploy workflow definition
///
/// Endpoint: POST /api/v1/workflow_definitions
///
/// Validates and stores a workflow definition. The version is automatically
/// generated as a timestamp (format: YYYYMMDDHHmmss) at deployment time.
///
/// Validation includes:
/// - Syntax validation (valid YAML structure)
/// - Semantic validation (activity references, no cycles)
/// - Graph structure validation (no cycles)
///
/// Returns 409 Conflict if a definition with the same name and version already exists
/// (extremely rare due to timestamp precision).
#[utoipa::path(
    post,
    path = "/api/v1/workflow_definitions",
    tag = "Workflow Definitions",
    request_body = DeployWorkflowDefinitionRequest,
    responses(
        (status = 201, description = "Workflow definition deployed successfully", body = DeployWorkflowDefinitionResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 409, description = "Workflow definition already exists", body = ApiErrorResponse),
        (status = 422, description = "Validation error", body = ApiErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn deploy_workflow_definition(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<DeployWorkflowDefinitionRequest>,
) -> ApiResult<(StatusCode, Json<DeployWorkflowDefinitionResponse>)> {
    tracing::info!(
        workflow_name = %request.definition.name,
        user = %claims.subject(),
        "Deploying workflow definition (version will be auto-generated)"
    );

    // Store definition (includes validation)
    let stored = state
        .workflow_definition_repo
        .store(request.definition)
        .await
        .map_err(|e| match e {
            RepositoryError::ValidationError(ve) => {
                tracing::warn!("Workflow definition validation failed: {:?}", ve);
                AppError::ValidationError(crate::error::ValidationErrors::from_workflow_validation(ve))
            }
            RepositoryError::DuplicateVersion { name, version } => {
                tracing::warn!("Duplicate workflow definition: {} version {}", name, version);
                AppError::Conflict(format!(
                    "Workflow definition '{}' version '{}' already exists",
                    name, version
                ))
            }
            RepositoryError::DatabaseError(e) => {
                tracing::error!("Database error storing workflow definition: {:?}", e);
                AppError::InternalServerError("Failed to store workflow definition".to_string())
            }
            RepositoryError::SerializationError(e) => {
                tracing::error!("Serialization error: {:?}", e);
                AppError::InternalServerError("Failed to serialize workflow definition".to_string())
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(DeployWorkflowDefinitionResponse {
            name: stored.name.clone(),
            version: stored.version.clone(),
            created_at: stored.created_at,
            message: format!(
                "Workflow definition '{}' version '{}' deployed successfully",
                stored.name, stored.version
            ),
        }),
    ))
}

/// List workflow definitions
///
/// Endpoint: GET /api/v1/workflow_definitions
///
/// Returns all deployed workflow definitions.
/// Post-MVP: Add filtering, pagination, and search.
#[utoipa::path(
    get,
    path = "/api/v1/workflow_definitions",
    tag = "Workflow Definitions",
    responses(
        (status = 200, description = "List of workflow definitions", body = ListWorkflowDefinitionsResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_workflow_definitions(
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
) -> ApiResult<Json<ListWorkflowDefinitionsResponse>> {
    let definitions = state.workflow_definition_repo.list().await.map_err(|e| {
        tracing::error!("Failed to list workflow definitions: {:?}", e);
        AppError::InternalServerError("Failed to list workflow definitions".to_string())
    })?;

    let summaries: Vec<WorkflowDefinitionSummary> = definitions
        .into_iter()
        .map(|d| WorkflowDefinitionSummary {
            name: d.name,
            version: d.version,
            description: d.definition.description,
            activity_count: d.definition.activities.len(),
            created_at: d.created_at,
        })
        .collect();

    let total = summaries.len();

    Ok(Json(ListWorkflowDefinitionsResponse {
        definitions: summaries,
        total,
    }))
}

/// Get workflow definition
///
/// Endpoint: GET /api/v1/workflow_definitions/{name}
///
/// Returns a specific workflow definition by name.
/// - If `version` query parameter is provided, returns that specific version
/// - If `version` is not provided, returns the latest version (most recently deployed)
#[utoipa::path(
    get,
    path = "/api/v1/workflow_definitions/{name}",
    tag = "Workflow Definitions",
    params(
        ("name" = String, Path, description = "Workflow name"),
        ("version" = Option<String>, Query, description = "Specific version (optional, returns latest if not provided)")
    ),
    responses(
        (status = 200, description = "Workflow definition", body = GetWorkflowDefinitionResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Workflow definition not found", body = ApiErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_workflow_definition(
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(name): Path<String>,
    Query(query): Query<GetWorkflowDefinitionQuery>,
) -> ApiResult<Json<GetWorkflowDefinitionResponse>> {
    let stored = if let Some(version) = query.version {
        // Get specific version
        state
            .workflow_definition_repo
            .get(&name, &version)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get workflow definition: {:?}", e);
                AppError::InternalServerError("Failed to retrieve workflow definition".to_string())
            })?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "Workflow definition '{}' version '{}' not found",
                    name, version
                ))
            })?
    } else {
        // Get latest version
        state
            .workflow_definition_repo
            .get_latest(&name)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get latest workflow definition: {:?}", e);
                AppError::InternalServerError("Failed to retrieve workflow definition".to_string())
            })?
            .ok_or_else(|| AppError::NotFound(format!("Workflow definition '{}' not found", name)))?
    };

    Ok(Json(GetWorkflowDefinitionResponse {
        name: stored.name,
        version: stored.version,
        definition: stored.definition,
        created_at: stored.created_at,
    }))
}
```

**Key Features**:
- Clear separation of request/response DTOs from domain models
- Detailed validation error messages with field-level details
- HTTP status codes match acceptance criteria (201, 404, 409, 422)
- Authentication required via middleware (ValidatedClaims)
- OpenAPI documentation via utoipa
- Structured logging for debugging
- Latest version retrieval when version not specified

---

### Component 4: Route Configuration

**Location**: Update `api/src/routes.rs`

**Responsibilities**:
1. Add workflow definition routes to protected route group
2. Ensure authentication middleware is applied

**Implementation**:

```rust
// Add to protected_routes() function in api/src/routes.rs

use crate::handlers::workflow_definitions;

pub fn protected_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/info", get(handlers::health::service_info_handler))

        // Workflow Definition Management
        .route(
            "/api/v1/workflow_definitions",
            post(workflow_definitions::deploy_workflow_definition)
                .get(workflow_definitions::list_workflow_definitions),
        )
        .route(
            "/api/v1/workflow_definitions/:name",
            get(workflow_definitions::get_workflow_definition),
        )

        // Future routes...

        .layer(axum_middleware::from_fn_with_state(
            middleware::auth_middleware,
        ))
}
```

---

### Component 5: Application State Updates

**Location**: Update `api/src/state.rs`

**Responsibilities**:
1. Add WorkflowDefinitionRepository to application state
2. Initialize repository during server startup

**Implementation**:

```rust
use core::workflow::repository::WorkflowDefinitionRepository;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: PgPool,
    pub auth_service: Arc<dyn AuthenticationService>,
    pub workflow_definition_repo: WorkflowDefinitionRepository,
}

impl AppState {
    pub async fn new(db_pool: PgPool, auth_config: AuthConfig) -> Self {
        let auth_service = PostgresAuthService::new(db_pool.clone(), auth_config);
        let workflow_definition_repo = WorkflowDefinitionRepository::new(db_pool.clone());

        Self {
            db_pool,
            auth_service: Arc::new(auth_service),
            workflow_definition_repo,
        }
    }
}
```

---

### Component 6: Database Migration

**Location**: `migrations/YYYYMMDD_workflow_definitions.sql`

**Note**: Schema already defined in architecture.md, this migration creates the table.

**Implementation**:

```sql
-- Migration: Create workflow_definitions table
-- Date: 2025-11-05

-- Workflow definitions table
CREATE TABLE IF NOT EXISTS workflow_definitions (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    definition JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(name, version)
);

-- Index for listing all definitions
CREATE INDEX idx_workflow_definitions_name ON workflow_definitions(name);

-- Index for listing by created_at (for "latest" queries)
CREATE INDEX idx_workflow_definitions_created_at ON workflow_definitions(created_at DESC);

-- Index for searching within definition JSONB (post-MVP)
CREATE INDEX idx_workflow_definitions_definition ON workflow_definitions USING gin(definition);
```

---

### Component 7: Error Handling Updates

**Location**: Update `api/src/error.rs`

**Responsibilities**:
1. Add conversion from ValidationErrors to ApiError
2. Provide helpful error messages for validation failures

**Implementation**:

```rust
// Add to api/src/error.rs

impl ValidationErrors {
    /// Convert workflow validation errors to API validation errors
    pub fn from_workflow_validation(
        ve: core::workflow::definition::ValidationError,
    ) -> Self {
        match ve {
            core::workflow::definition::ValidationError::SingleError(msg) => {
                let mut errors = Self::new();
                errors.add("definition", &msg);
                errors
            }
            core::workflow::definition::ValidationError::MultipleErrors(errs) => {
                let mut errors = Self::new();
                for (field, messages) in errs.errors() {
                    for message in messages {
                        errors.add(field, message);
                    }
                }
                errors
            }
        }
    }
}
```

---

## Testing Requirements

### Unit Tests

**File**: `core/src/workflow/definition_test.rs`

**Test Scenarios**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_workflow() {
        let definition = WorkflowDefinition {
            name: "payment_processing".to_string(),
            description: Some("Payment workflow".to_string()),
            activities: vec![
                ActivityDefinition {
                    key: "validate".to_string(),
                    namespace: "payments".to_string(),
                    name: Some("validate_card".to_string()),
                    parameters: None,
                    preceding: None,
                    following: Some(vec![ActivityRelationship {
                        activity_key: "authorize".to_string(),
                        conditions: None,
                    }]),
                    settings: None,
                },
                ActivityDefinition {
                    key: "authorize".to_string(),
                    namespace: "payments".to_string(),
                    name: Some("authorize_card".to_string()),
                    parameters: None,
                    preceding: None,
                    following: None,
                    settings: None,
                },
            ],
            settings: None,
        };

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn test_validate_duplicate_activity_keys() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            description: None,
            activities: vec![
                ActivityDefinition {
                    key: "step1".to_string(),
                    namespace: "test".to_string(),
                    name: None,
                    parameters: None,
                    preceding: None,
                    following: None,
                    settings: None,
                },
                ActivityDefinition {
                    key: "step1".to_string(), // Duplicate!
                    namespace: "test".to_string(),
                    name: None,
                    parameters: None,
                    preceding: None,
                    following: None,
                    settings: None,
                },
            ],
            settings: None,
        };

        let result = definition.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Duplicate activity key"));
    }

    #[test]
    fn test_validate_invalid_activity_reference() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            description: None,
            activities: vec![ActivityDefinition {
                key: "step1".to_string(),
                namespace: "test".to_string(),
                name: None,
                parameters: None,
                preceding: None,
                following: Some(vec![ActivityRelationship {
                    activity_key: "step2".to_string(), // Doesn't exist!
                    conditions: None,
                }]),
                settings: None,
            }],
            settings: None,
        };

        let result = definition.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_validate_cycle_detection() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            description: None,
            activities: vec![
                ActivityDefinition {
                    key: "step1".to_string(),
                    namespace: "test".to_string(),
                    name: None,
                    parameters: None,
                    preceding: None,
                    following: Some(vec![ActivityRelationship {
                        activity_key: "step2".to_string(),
                        conditions: None,
                    }]),
                    settings: None,
                },
                ActivityDefinition {
                    key: "step2".to_string(),
                    namespace: "test".to_string(),
                    name: None,
                    parameters: None,
                    preceding: None,
                    following: Some(vec![ActivityRelationship {
                        activity_key: "step1".to_string(), // Cycle!
                        conditions: None,
                    }]),
                    settings: None,
                },
            ],
            settings: None,
        };

        let result = definition.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn test_generate_version() {
        let version1 = WorkflowDefinition::generate_version();
        let version2 = WorkflowDefinition::generate_version();

        // Check format: YYYYMMDDHHmmss (14 digits)
        assert_eq!(version1.len(), 14);
        assert!(version1.chars().all(|c| c.is_ascii_digit()));

        // Versions should be equal or sequential if generated close together
        assert!(version1 <= version2);
    }
}
```

### Integration Tests

**File**: `api/tests/workflow_definitions_test.rs`

**Test Scenarios**:

```rust
#[tokio::test]
async fn test_deploy_workflow_definition() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let definition = json!({
        "name": "test_workflow",
        "description": "Test workflow",
        "activities": [
            {
                "key": "step1",
                "namespace": "test",
                "name": "test_activity"
            }
        ]
    });

    let response = app
        .post("/api/v1/workflow_definitions")
        .header("Authorization", format!("Bearer {}", token))
        .json(&definition)
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);

    let body: DeployWorkflowDefinitionResponse = response.json().await;
    assert_eq!(body.name, "test_workflow");
    // Version is auto-generated timestamp (14 digits)
    assert_eq!(body.version.len(), 14);
    assert!(body.version.chars().all(|c| c.is_ascii_digit()));
}

#[tokio::test]
async fn test_deploy_multiple_versions() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let definition = json!({
        "name": "test_workflow",
        "activities": [
            {
                "key": "step1",
                "namespace": "test"
            }
        ]
    });

    // Deploy first version
    let response1 = app
        .post("/api/v1/workflow_definitions")
        .header("Authorization", format!("Bearer {}", token))
        .json(&definition)
        .await;

    assert_eq!(response1.status(), StatusCode::CREATED);
    let body1: DeployWorkflowDefinitionResponse = response1.json().await;

    // Sleep briefly to ensure different timestamp
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Deploy second version - should succeed with different timestamp
    let response2 = app
        .post("/api/v1/workflow_definitions")
        .header("Authorization", format!("Bearer {}", token))
        .json(&definition)
        .await;

    assert_eq!(response2.status(), StatusCode::CREATED);
    let body2: DeployWorkflowDefinitionResponse = response2.json().await;

    // Versions should be different
    assert_ne!(body1.version, body2.version);
    // Second version should be later
    assert!(body2.version > body1.version);
}

#[tokio::test]
async fn test_deploy_invalid_workflow() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let definition = json!({
        "name": "test_workflow",
        "activities": [
            {
                "key": "step1",
                "namespace": "test",
                "following": [
                    {
                        "activity_key": "step2" // Doesn't exist!
                    }
                ]
            }
        ]
    });

    let response = app
        .post("/api/v1/workflow_definitions")
        .header("Authorization", format!("Bearer {}", token))
        .json(&definition)
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: ApiErrorResponse = response.json().await;
    assert_eq!(body.error.code, ErrorCode::ValidationError);
    assert!(body.error.message.contains("not found"));
}

#[tokio::test]
async fn test_list_workflow_definitions() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Deploy a few definitions
    for i in 1..=3 {
        let definition = json!({
            "name": format!("workflow_{}", i),
            "activities": [
                {
                    "key": "step1",
                    "namespace": "test"
                }
            ]
        });

        app.post("/api/v1/workflow_definitions")
            .header("Authorization", format!("Bearer {}", token))
            .json(&definition)
            .await;
    }

    // List all definitions
    let response = app
        .get("/api/v1/workflow_definitions")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ListWorkflowDefinitionsResponse = response.json().await;
    assert!(body.total >= 3);
    assert!(body.definitions.len() >= 3);
}

#[tokio::test]
async fn test_get_workflow_definition_by_version() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Deploy workflow
    let definition = json!({
        "name": "test_workflow",
        "activities": [
            {
                "key": "step1",
                "namespace": "test"
            }
        ]
    });

    let deploy_response = app
        .post("/api/v1/workflow_definitions")
        .header("Authorization", format!("Bearer {}", token))
        .json(&definition)
        .await;

    let deploy_body: DeployWorkflowDefinitionResponse = deploy_response.json().await;
    let version = deploy_body.version;

    // Get specific version
    let response = app
        .get(&format!("/api/v1/workflow_definitions/test_workflow?version={}", version))
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: GetWorkflowDefinitionResponse = response.json().await;
    assert_eq!(body.name, "test_workflow");
    assert_eq!(body.version, version);
    assert_eq!(body.definition.activities.len(), 1);
}

#[tokio::test]
async fn test_get_latest_workflow_definition() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let definition = json!({
        "name": "test_workflow",
        "activities": [
            {
                "key": "step1",
                "namespace": "test"
            }
        ]
    });

    // Deploy multiple versions
    let mut versions = Vec::new();
    for _ in 0..3 {
        let deploy_response = app
            .post("/api/v1/workflow_definitions")
            .header("Authorization", format!("Bearer {}", token))
            .json(&definition)
            .await;

        let deploy_body: DeployWorkflowDefinitionResponse = deploy_response.json().await;
        versions.push(deploy_body.version);

        // Sleep to ensure different timestamps
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // Get latest (no version parameter)
    let response = app
        .get("/api/v1/workflow_definitions/test_workflow")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: GetWorkflowDefinitionResponse = response.json().await;
    assert_eq!(body.name, "test_workflow");
    assert_eq!(body.version, versions[2]); // Latest version (last deployed)
}

#[tokio::test]
async fn test_get_nonexistent_workflow() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    let response = app
        .get("/api/v1/workflow_definitions/nonexistent")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body: ApiErrorResponse = response.json().await;
    assert_eq!(body.error.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn test_deploy_requires_authentication() {
    let app = test_app().await;

    let definition = json!({
        "name": "test_workflow",
        "activities": [
            {
                "key": "step1",
                "namespace": "test"
            }
        ]
    });

    // No Authorization header
    let response = app
        .post("/api/v1/workflow_definitions")
        .json(&definition)
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

---

## Dependencies

### New Dependencies

Add to `core/Cargo.toml`:

```toml
[dependencies]
# Existing dependencies...

# Regex for identifier validation
regex = "1"

# Chrono for timestamp generation
chrono = { version = "0.4", features = ["serde"] }
```

No additional dependencies needed for `api` crate (already has required dependencies).

---

## Configuration

### Environment Variables

No new environment variables required. Uses existing `DATABASE_URL`.

---

## Documentation Updates

### API Documentation

Update `docs/api-reference.md`:

```markdown
## Workflow Definition Management

Workflow definitions are deployed separately from workflow execution. This allows versioning, reusability, and validation before execution.

### Deploy Workflow Definition

**Endpoint**: `POST /api/v1/workflow_definitions`

**Authentication**: Required (Bearer token)

**Request Body**:
\`\`\`json
{
  "name": "payment_processing",
  "description": "Process payment with validation",
  "activities": [
    {
      "key": "validate_payment",
      "namespace": "payments",
      "name": "validate_card",
      "parameters": {
        "card_token": "{{ARG.card_token}}"
      },
      "following": [
        {
          "activity_key": "authorize_card",
          "conditions": ["{{validate_payment.valid}} == true"]
        }
      ]
    },
    {
      "key": "authorize_card",
      "namespace": "payments",
      "name": "authorize",
      "parameters": {
        "amount": "{{ARG.amount}}"
      }
    }
  ]
}
\`\`\`

**Response** (201 Created):
\`\`\`json
{
  "name": "payment_processing",
  "version": "20250105143022",
  "created_at": "2025-11-05T10:30:00Z",
  "message": "Workflow definition 'payment_processing' version '20250105143022' deployed successfully"
}
\`\`\`

**Note**: Version is auto-generated as a timestamp (format: `YYYYMMDDHHmmss`).

**Error Responses**:
- `409 Conflict` - Workflow definition with same name and version already exists (extremely rare)
- `422 Unprocessable Entity` - Validation error (invalid structure, cycles, missing references)

### List Workflow Definitions

**Endpoint**: `GET /api/v1/workflow_definitions`

**Authentication**: Required (Bearer token)

**Response** (200 OK):
\`\`\`json
{
  "definitions": [
    {
      "name": "payment_processing",
      "version": "20250105143022",
      "description": "Process payment with validation",
      "activity_count": 2,
      "created_at": "2025-11-05T10:30:00Z"
    }
  ],
  "total": 1
}
\`\`\`

### Get Workflow Definition

**Endpoint**: `GET /api/v1/workflow_definitions/{name}`

**Authentication**: Required (Bearer token)

**Query Parameters**:
- `version` (optional) - Specific version to retrieve. If not provided, returns latest version.

**Response** (200 OK):
\`\`\`json
{
  "name": "payment_processing",
  "version": "20250105143022",
  "definition": {
    "name": "payment_processing",
    "description": "Process payment with validation",
    "activities": [...]
  },
  "created_at": "2025-11-05T10:30:00Z"
}
\`\`\`

**Error Responses**:
- `404 Not Found` - Workflow definition not found

### Version Format

StreamFlow uses automatic timestamp-based versioning:

**Timestamp Versioning**:
- Format: `YYYYMMDDHHmmss` (e.g., `20250105143022`)
- Auto-generated at deployment time (UTC)
- Sortable by deployment time
- 14 digits: year (4), month (2), day (2), hour (2), minute (2), second (2)
- No user input required - eliminates version conflicts
```

---

## Success Criteria

### Functional Requirements

- ✅ `POST /api/v1/workflow_definitions` deploys workflow definition with validation
- ✅ Version auto-generated as timestamp (format: `YYYYMMDDHHmmss`)
- ✅ Unique constraint enforced on (name, version)
- ✅ 409 Conflict returned for duplicate deployments (extremely rare)
- ✅ Validation includes syntax and semantic checks
- ✅ Validation errors include field-level details with line numbers
- ✅ Cycle detection prevents infinite loops
- ✅ Activity reference validation ensures all edges point to valid activities
- ✅ `GET /api/v1/workflow_definitions` lists all definitions
- ✅ `GET /api/v1/workflow_definitions/{name}` gets latest version
- ✅ `GET /api/v1/workflow_definitions/{name}?version={version}` gets specific version
- ✅ Timestamp versioning ensures natural ordering by deployment time
- ✅ Authentication required for all endpoints

### Non-Functional Requirements

- ✅ Definitions stored as JSONB for efficient querying
- ✅ Indexed queries for fast retrieval
- ✅ Validation happens before storage (fail fast)
- ✅ Clear error messages with actionable feedback
- ✅ OpenAPI documentation for all endpoints
- ✅ Repository pattern separates storage from business logic

---

## Implementation Phases

### Phase 1: Data Model and Validation (P0)
- Implement WorkflowDefinition struct
- Implement validation logic (structure, cycles, references)
- Unit tests for validation
- **Estimated Time**: 4 hours

### Phase 2: Repository Layer (P0)
- Implement WorkflowDefinitionRepository
- Store/retrieve definitions
- Handle unique constraint violations
- Unit tests for repository
- **Estimated Time**: 2 hours

### Phase 3: API Handlers (P0)
- Implement deploy endpoint
- Implement list endpoint
- Implement get endpoint (latest and specific version)
- Integration tests
- **Estimated Time**: 3 hours

### Phase 4: Route Configuration and State (P0)
- Add routes to protected route group
- Update AppState with repository
- Ensure middleware applied
- **Estimated Time**: 1 hour

### Phase 5: Database Migration (P0)
- Create workflow_definitions table
- Create indexes
- Test migration
- **Estimated Time**: 1 hour

### Phase 6: Testing and Documentation (P0)
- End-to-end workflow tests
- Update API documentation
- Update OpenAPI spec
- **Estimated Time**: 2 hours

**Total Estimated Time**: 13 hours

---

## Risks and Mitigations

### Risk 1: Complex Validation Logic

**Probability**: Medium
**Impact**: High (invalid workflows could break execution)

**Mitigation**:
- Comprehensive unit tests for all validation rules
- Fail fast at deployment time (not execution time)
- Clear error messages guide users to fix issues
- Cycle detection algorithm well-tested (DFS-based)
- Validation happens before storing in database

### Risk 2: Timestamp Version Collisions

**Probability**: Very Low
**Impact**: Low (duplicate version error)

**Mitigation**:
- Timestamp precision of 1 second makes collisions extremely rare
- Even in high-volume scenarios (<1 deployment/second per workflow name)
- If collision occurs, 409 Conflict returned with clear error message
- User can simply retry (next second will succeed)
- Database unique constraint prevents corrupt state

### Risk 3: Large Workflow Definitions

**Probability**: Low
**Impact**: Medium (performance issues with large JSONB)

**Mitigation**:
- JSONB is efficient for storage and querying
- Indexes on frequently queried fields
- List endpoint returns summaries (not full definitions)
- Get endpoint returns full definition (acceptable for single query)
- Post-MVP: Add pagination if needed

### Risk 4: Validation Performance

**Probability**: Low
**Impact**: Low (validation takes too long)

**Mitigation**:
- Validation is in-memory operation (fast)
- Most workflows have <100 activities (validation <10ms)
- Cycle detection is O(V+E) - efficient for typical graphs
- Async handler allows concurrent validations
- Benchmark validation for large workflows

---

## Future Enhancements (Post-MVP)

### Search and Filtering
- Filter by name pattern
- Filter by namespace
- Full-text search in descriptions
- Filter by activity types used

### Pagination
- Limit and offset parameters
- Cursor-based pagination
- Total count metadata

### Definition Metadata
- Tags for categorization
- Author information
- Deprecation status
- Usage statistics (how many workflows executed with this definition)

### Version Management
- Delete old versions
- Mark versions as deprecated
- Default version per workflow name
- Version comparison (diff between versions)

### Advanced Validation
- Activity type registry (validate activity.namespace + activity.name exist)
- Parameter schema validation
- Template expression validation
- Resource limit checks (max activities, max depth)

---

## Related User Stories

- **US-1A.3**: Authentication (provides auth middleware for endpoints)
- **US-1A.5**: Workflow Submission API (uses definitions deployed via this story)
- **US-1A.6**: Workflow Status and Query API (references definition name and version)

---

## References

- Architecture: `docs/architecture.md` (Data Architecture - Workflow Definitions)
- Requirements: `docs/mvp-requirements.md` (Epic 1A, US-1A.4)
- Workflow Definition Language: `docs/architecture.md` (Workflow Definition Languages)
- Semantic Versioning: https://semver.org/

---

## Implementation Notes

**Key Design Decisions**:
1. **Directed Graph Structure**: Activities use `preceding`/`following` relationships (never "edges")
2. **JSONB Storage**: Efficient querying and validation without external schema
3. **Auto-Generated Versions**: Timestamp-based versions (YYYYMMDDHHmmss) eliminate user coordination and conflicts
4. **Fail Fast**: Validation happens at deployment, not execution
5. **Latest Version Logic**: Based on `created_at` timestamp (most recent deployment)
6. **Repository Pattern**: Clean separation of storage from business logic
7. **Cycle Detection**: DFS-based algorithm for reliable cycle detection
8. **Field-Level Errors**: Validation errors include specific field paths for debugging

**Implementation Order**:
1. Data model and validation (foundation)
2. Repository layer (storage)
3. API handlers (user interface)
4. Route configuration (integration)
5. Database migration (persistence)
6. Testing and documentation (verification)

**Post-Implementation**:
- US-1A.5 (Workflow Submission) will reference definitions by name and version
- Workflow execution will load definitions from this repository
- Versioning enables A/B testing and rollback strategies
- Validation ensures only valid workflows can be executed
