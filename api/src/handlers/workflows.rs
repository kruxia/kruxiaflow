use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use kruxiaflow_core::workflow::{
    WorkflowFilters, WorkflowQueryError, WorkflowQueryService, WorkflowService,
    WorkflowServiceError,
};
use kruxiaflow_core::{WorkflowActivityStatus, WorkflowStatus};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

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
    #[schema(value_type = Object, example = json!({"amount": 100.00, "card_token": "tok_123"}))]
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
#[tracing::instrument(
    skip(service, claims, request),
    fields(
        definition_name = %request.definition_name,
        version = ?request.version,
        unique_key = ?request.unique_key,
        user = %claims.subject()
    )
)]
pub async fn submit_workflow(
    service: WorkflowService,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<SubmitWorkflowRequest>,
) -> ApiResult<(StatusCode, Json<SubmitWorkflowResponse>)> {
    // Validate request structure
    request.validate().map_err(AppError::ValidationError)?;

    // Submit workflow via service
    let workflow = service
        .submit_workflow(
            &request.definition_name,
            request.version.as_deref(),
            request.input,
            request.unique_key,
        )
        .await
        .map_err(|e| match e {
            WorkflowServiceError::DefinitionNotFound { name, version } => {
                tracing::warn!(
                    "Workflow definition not found: {} version {}",
                    name,
                    version
                );
                AppError::NotFound(format!(
                    "Workflow definition '{}' version '{}' not found",
                    name, version
                ))
            }
            WorkflowServiceError::DefinitionNotFoundLatest { name } => {
                tracing::warn!(
                    "Workflow definition not found: {} (no version specified)",
                    name
                );
                AppError::NotFound(format!(
                    "Workflow definition '{}' not found. No versions deployed.",
                    name
                ))
            }
            WorkflowServiceError::DuplicateSubmission(key) => {
                tracing::warn!("Duplicate workflow submission: unique_key '{}'", key);
                AppError::Conflict(format!("Workflow with unique_key '{}' already exists", key))
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

    tracing::debug!(
        event = "WorkflowSubmitted",
        workflow_id = %workflow.id,
        definition_name = %workflow.definition_name,
        definition_version = %workflow.definition_version,
    );

    Ok((
        StatusCode::CREATED,
        Json(SubmitWorkflowResponse {
            workflow_id: workflow.id,
            definition_name: workflow.definition_name,
            definition_version: workflow.definition_version,
            status: workflow.status,
            created_at: workflow.created_at,
        }),
    ))
}

// ============================================================================
// Workflow Query API
// ============================================================================

/// Get workflow by ID response
///
/// Returns the current state of a workflow, including status,
/// activity states, and custom state data.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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

    /// Activity states
    pub activities: Vec<ActivityState>,

    /// Custom workflow state data (from workflows.state_data column)
    #[schema(example = json!({
        "custom_field": "value"
    }))]
    pub state_data: serde_json::Value,
}

/// Activity state in a workflow
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
                errors.add(
                    "created_after",
                    "created_after must be before created_before",
                );
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowSummary {
    /// Unique workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub id: Uuid,

    /// Workflow status: created, running, completed, failed, or paused
    #[schema(value_type = String, example = "running")]
    pub status: WorkflowStatus,

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
#[tracing::instrument(
    skip(service, claims),
    fields(
        workflow_id = %workflow_id,
        user = %claims.subject()
    )
)]
pub async fn get_workflow(
    service: WorkflowQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Path(workflow_id): Path<Uuid>,
) -> ApiResult<Json<GetWorkflowResponse>> {
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

    // Parse activities from the workflow's activities JSONB column
    let activities = parse_activities(&workflow.activities).map_err(|e| {
        tracing::error!("Failed to parse activities: {:?}", e);
        AppError::InternalError(anyhow::anyhow!("Failed to parse activities: {}", e))
    })?;

    tracing::debug!(
        workflow_id = %workflow.id,
        status = %workflow.status,
        activity_count = activities.len(),
        "Workflow retrieved"
    );

    Ok(Json(GetWorkflowResponse {
        id: workflow.id,
        status: workflow.status,
        definition_name: workflow.definition_name,
        created_at: workflow.created_at,
        updated_at: workflow.updated_at,
        activities,
        state_data: workflow.state_data,
    }))
}

/// Parse activities from JSONB value into structured ActivityState objects
fn parse_activities(activities_json: &serde_json::Value) -> Result<Vec<ActivityState>, String> {
    let activities_map = activities_json
        .as_object()
        .ok_or_else(|| "activities is not an object".to_string())?;

    let mut activities = Vec::new();
    for (activity_key, activity_state) in activities_map {
        let status = activity_state
            .get("status")
            .ok_or_else(|| format!("Activity '{}' missing status field", activity_key))?;

        let status: WorkflowActivityStatus = serde_json::from_value(status.clone())
            .map_err(|e| format!("Invalid status for activity '{}': {}", activity_key, e))?;

        let outputs = activity_state.get("outputs").cloned();

        let started_at = activity_state
            .get("started_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let completed_at = activity_state
            .get("completed_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        activities.push(ActivityState {
            activity_key: activity_key.clone(),
            status,
            outputs,
            started_at,
            completed_at,
        });
    }

    Ok(activities)
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
#[tracing::instrument(
    skip(service, claims, query),
    fields(
        status = ?query.status,
        definition_name = ?query.definition_name,
        limit = query.limit,
        offset = query.offset,
        user = %claims.subject()
    )
)]
pub async fn list_workflows(
    service: WorkflowQueryService,
    Extension(claims): Extension<ValidatedClaims>,
    Query(query): Query<ListWorkflowsQuery>,
) -> ApiResult<Json<ListWorkflowsResponse>> {
    // Validate query parameters
    query.validate().map_err(AppError::ValidationError)?;

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

    let count = workflows.len() as i64;

    tracing::debug!(count = count, total = total, "Workflows retrieved");

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
        count,
        limit: query.limit,
        offset: query.offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    // =========================================================================
    // SubmitWorkflowRequest validation tests
    // =========================================================================

    #[test]
    fn test_submit_workflow_request_valid() {
        let request = SubmitWorkflowRequest {
            definition_name: "payment_processing".to_string(),
            version: None,
            input: json!({"amount": 100.00}),
            unique_key: None,
        };

        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_submit_workflow_request_valid_with_all_fields() {
        let request = SubmitWorkflowRequest {
            definition_name: "payment_processing".to_string(),
            version: Some("20251105.143022.123456".to_string()),
            input: json!({"amount": 100.00, "card_token": "tok_123"}),
            unique_key: Some("order_12345_payment".to_string()),
        };

        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_submit_workflow_request_empty_definition_name() {
        let request = SubmitWorkflowRequest {
            definition_name: "".to_string(),
            version: None,
            input: json!({}),
            unique_key: None,
        };

        let result = request.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("definition_name"));
    }

    #[test]
    fn test_submit_workflow_request_input_must_be_object() {
        // Array input
        let request = SubmitWorkflowRequest {
            definition_name: "test".to_string(),
            version: None,
            input: json!(["array", "input"]),
            unique_key: None,
        };

        let result = request.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("input"));
    }

    #[test]
    fn test_submit_workflow_request_input_primitive_rejected() {
        // String input
        let request = SubmitWorkflowRequest {
            definition_name: "test".to_string(),
            version: None,
            input: json!("just a string"),
            unique_key: None,
        };

        let result = request.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("input"));
    }

    #[test]
    fn test_submit_workflow_request_empty_unique_key() {
        let request = SubmitWorkflowRequest {
            definition_name: "test".to_string(),
            version: None,
            input: json!({}),
            unique_key: Some("".to_string()),
        };

        let result = request.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("unique_key"));
    }

    #[test]
    fn test_submit_workflow_request_unique_key_too_long() {
        let long_key = "x".repeat(256);
        let request = SubmitWorkflowRequest {
            definition_name: "test".to_string(),
            version: None,
            input: json!({}),
            unique_key: Some(long_key),
        };

        let result = request.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("unique_key"));
    }

    #[test]
    fn test_submit_workflow_request_unique_key_max_length() {
        let max_key = "x".repeat(255);
        let request = SubmitWorkflowRequest {
            definition_name: "test".to_string(),
            version: None,
            input: json!({}),
            unique_key: Some(max_key),
        };

        assert!(request.validate().is_ok());
    }

    // =========================================================================
    // ListWorkflowsQuery validation tests
    // =========================================================================

    #[test]
    fn test_list_workflows_query_valid_defaults() {
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: None,
            created_before: None,
            limit: 100,
            offset: 0,
        };

        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_list_workflows_query_valid_with_filters() {
        let query = ListWorkflowsQuery {
            status: Some("running".to_string()),
            definition_name: Some("payment_processing".to_string()),
            created_after: Some(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()),
            created_before: Some(Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap()),
            limit: 50,
            offset: 10,
        };

        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_list_workflows_query_limit_zero() {
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: None,
            created_before: None,
            limit: 0,
            offset: 0,
        };

        let result = query.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("limit"));
    }

    #[test]
    fn test_list_workflows_query_limit_exceeds_max() {
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: None,
            created_before: None,
            limit: 1001,
            offset: 0,
        };

        let result = query.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("limit"));
    }

    #[test]
    fn test_list_workflows_query_limit_max_allowed() {
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: None,
            created_before: None,
            limit: 1000,
            offset: 0,
        };

        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_list_workflows_query_negative_offset() {
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: None,
            created_before: None,
            limit: 100,
            offset: -1,
        };

        let result = query.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("offset"));
    }

    #[test]
    fn test_list_workflows_query_invalid_time_range() {
        // created_after >= created_before
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: Some(Utc.with_ymd_and_hms(2025, 12, 31, 0, 0, 0).unwrap()),
            created_before: Some(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()),
            limit: 100,
            offset: 0,
        };

        let result = query.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.has_error("created_after"));
    }

    #[test]
    fn test_list_workflows_query_same_time_range() {
        // created_after == created_before is invalid
        let same_time = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
        let query = ListWorkflowsQuery {
            status: None,
            definition_name: None,
            created_after: Some(same_time),
            created_before: Some(same_time),
            limit: 100,
            offset: 0,
        };

        let result = query.validate();
        assert!(result.is_err());
    }

    // =========================================================================
    // parse_activities tests
    // =========================================================================

    #[test]
    fn test_parse_activities_valid() {
        let activities_json = json!({
            "step1": {
                "status": "completed",
                "outputs": {"result": "success"},
                "started_at": "2025-11-06T10:00:00Z",
                "completed_at": "2025-11-06T10:00:05Z"
            },
            "step2": {
                "status": "running",
                "outputs": null,
                "started_at": "2025-11-06T10:00:05Z"
            }
        });

        let result = parse_activities(&activities_json);
        assert!(result.is_ok());

        let activities = result.unwrap();
        assert_eq!(activities.len(), 2);
    }

    #[test]
    fn test_parse_activities_empty() {
        let activities_json = json!({});

        let result = parse_activities(&activities_json);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_parse_activities_not_object() {
        let activities_json = json!(["array", "not", "object"]);

        let result = parse_activities(&activities_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not an object"));
    }

    #[test]
    fn test_parse_activities_missing_status() {
        let activities_json = json!({
            "step1": {
                "outputs": null
            }
        });

        let result = parse_activities(&activities_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing status"));
    }

    #[test]
    fn test_parse_activities_invalid_status() {
        let activities_json = json!({
            "step1": {
                "status": "invalid_status"
            }
        });

        let result = parse_activities(&activities_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid status"));
    }

    #[test]
    fn test_parse_activities_all_statuses() {
        let activities_json = json!({
            "not_scheduled": {"status": "not_scheduled"},
            "pending": {"status": "pending"},
            "running": {"status": "running"},
            "completed": {"status": "completed"},
            "failed": {"status": "failed"},
            "skipped": {"status": "skipped"}
        });

        let result = parse_activities(&activities_json);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 6);
    }

    #[test]
    fn test_parse_activities_with_outputs() {
        let activities_json = json!({
            "step1": {
                "status": "completed",
                "outputs": [
                    {"name": "result", "value": "success", "type": "value"}
                ]
            }
        });

        let result = parse_activities(&activities_json);
        assert!(result.is_ok());

        let activities = result.unwrap();
        assert!(activities[0].outputs.is_some());
    }

    // =========================================================================
    // Response serialization tests
    // =========================================================================

    #[test]
    fn test_submit_workflow_response_serialization() {
        let response = SubmitWorkflowResponse {
            workflow_id: Uuid::nil(),
            definition_name: "test_workflow".to_string(),
            definition_version: "20251105.143022.123456".to_string(),
            status: "created".to_string(),
            created_at: Utc.with_ymd_and_hms(2025, 11, 5, 14, 30, 22).unwrap(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("workflow_id"));
        assert!(json.contains("definition_name"));
        assert!(json.contains("status"));
    }

    #[test]
    fn test_get_workflow_response_serialization() {
        let response = GetWorkflowResponse {
            id: Uuid::nil(),
            status: "running".to_string(),
            definition_name: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            activities: vec![ActivityState {
                activity_key: "step1".to_string(),
                status: WorkflowActivityStatus::Completed,
                outputs: Some(json!({"result": "success"})),
                started_at: Some(Utc::now()),
                completed_at: Some(Utc::now()),
            }],
            state_data: json!({}),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("activities"));
        assert!(json.contains("step1"));
    }

    #[test]
    fn test_list_workflows_response_serialization() {
        let response = ListWorkflowsResponse {
            workflows: vec![WorkflowSummary {
                id: Uuid::nil(),
                status: WorkflowStatus::Running,
                definition_name: "test".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            total: 1,
            count: 1,
            limit: 100,
            offset: 0,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("workflows"));
        assert!(json.contains("total"));
        assert!(json.contains("count"));
    }
}
