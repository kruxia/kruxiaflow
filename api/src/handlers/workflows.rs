use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{Extension, Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use streamflow_core::workflow::{WorkflowService, WorkflowServiceError};
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
pub async fn submit_workflow(
    service: WorkflowService,
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

    tracing::info!(
        workflow_id = %workflow.id,
        definition_name = %workflow.definition_name,
        definition_version = %workflow.definition_version,
        "Workflow submitted successfully"
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
