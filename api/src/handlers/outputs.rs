//! Output Retrieval Handlers
//!
//! Handlers for activity output retrieval and file download endpoints.

use crate::dto::{GetActivityOutputResponse, GetWorkflowOutputResponse};
use crate::error::{ApiResult, AppError};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use streamflow_core::storage::StorageError;
use streamflow_core::workflow::{OutputQueryError, OutputQueryService};
use uuid::Uuid;

/// Get activity output
///
/// Endpoint: GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output
///
/// Returns the output, cost, and files for a completed activity.
/// Returns 404 if the workflow or activity doesn't exist.
/// Returns 400 if the activity is not yet completed.
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/activities/{activity_key}/output",
    tag = "Outputs",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID"),
        ("activity_key" = String, Path, description = "Activity key")
    ),
    responses(
        (status = 200, description = "Activity output retrieved", body = GetActivityOutputResponse),
        (status = 400, description = "Activity not completed"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow or activity not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(
    skip(state, claims),
    fields(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        user = %claims.subject()
    )
)]
pub async fn get_activity_output(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path((workflow_id, activity_key)): Path<(Uuid, String)>,
) -> ApiResult<Json<GetActivityOutputResponse>> {
    let service = OutputQueryService::new(state.db_pool.clone());

    let result = service
        .get_activity_output(workflow_id, &activity_key, state.workflow_storage.as_ref())
        .await
        .map_err(|e| match e {
            OutputQueryError::WorkflowNotFound(id) => {
                tracing::warn!("Workflow not found: {}", id);
                AppError::NotFound(format!("Workflow '{}' not found", id))
            }
            OutputQueryError::ActivityNotFound(key) => {
                tracing::warn!("Activity not found: {}", key);
                AppError::NotFound(format!("Activity '{}' not found", key))
            }
            OutputQueryError::ActivityNotCompleted(key) => {
                tracing::warn!("Activity not completed: {}", key);
                AppError::BadRequest(format!(
                    "Activity '{}' is not completed. Output is only available for completed activities.",
                    key
                ))
            }
            OutputQueryError::DatabaseError(e) => {
                tracing::error!("Database error: {:?}", e);
                AppError::DatabaseError(e)
            }
            OutputQueryError::StorageError(e) => {
                tracing::error!("Storage error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
            OutputQueryError::DeserializationError(e) => {
                tracing::error!("Deserialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
            _ => {
                tracing::error!("Unexpected error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    tracing::debug!(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        cost_usd = %result.cost_usd,
        file_count = result.files.len(),
        "Activity output retrieved"
    );

    Ok(Json(GetActivityOutputResponse::from(result)))
}

/// Get workflow output
///
/// Endpoint: GET /api/v1/workflows/{workflow_id}/output
///
/// Returns aggregated outputs from all completed activities in a workflow,
/// with terminal activities (final outputs) marked.
/// Returns 404 if the workflow doesn't exist.
/// Returns 400 if the workflow is not yet completed.
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/output",
    tag = "Outputs",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Workflow output retrieved", body = GetWorkflowOutputResponse),
        (status = 400, description = "Workflow not completed"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(
    skip(state, claims),
    fields(
        workflow_id = %workflow_id,
        user = %claims.subject()
    )
)]
pub async fn get_workflow_output(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path(workflow_id): Path<Uuid>,
) -> ApiResult<Json<GetWorkflowOutputResponse>> {
    let service = OutputQueryService::new(state.db_pool.clone());

    let result = service
        .get_workflow_output(workflow_id)
        .await
        .map_err(|e| match e {
            OutputQueryError::WorkflowNotFound(id) => {
                tracing::warn!("Workflow not found: {}", id);
                AppError::NotFound(format!("Workflow '{}' not found", id))
            }
            OutputQueryError::WorkflowNotCompleted => {
                tracing::warn!("Workflow not completed: {}", workflow_id);
                AppError::BadRequest(
                    "Workflow is not completed. Output is only available for completed workflows."
                        .to_string(),
                )
            }
            OutputQueryError::DatabaseError(e) => {
                tracing::error!("Database error: {:?}", e);
                AppError::DatabaseError(e)
            }
            OutputQueryError::DeserializationError(e) => {
                tracing::error!("Deserialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
            _ => {
                tracing::error!("Unexpected error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    tracing::debug!(
        workflow_id = %workflow_id,
        total_cost_usd = %result.total_cost_usd,
        activity_count = result.outputs.len(),
        terminal_count = result.terminal_outputs.len(),
        "Workflow output retrieved"
    );

    Ok(Json(GetWorkflowOutputResponse::from(result)))
}

/// Download activity file
///
/// Endpoint: GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
///
/// Streams the file content directly from workflow storage in chunks to the client response.
/// Returns 404 if the file doesn't exist.
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}",
    tag = "Outputs",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID"),
        ("activity_key" = String, Path, description = "Activity key"),
        ("filename" = String, Path, description = "Filename")
    ),
    responses(
        (status = 200, description = "File content", content_type = "application/octet-stream"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "File not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(
    skip(state, claims),
    fields(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        filename = %filename,
        user = %claims.subject()
    )
)]
pub async fn download_activity_file(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path((workflow_id, activity_key, filename)): Path<(Uuid, String, String)>,
) -> Result<Response, AppError> {
    // Get file metadata first
    let metadata = state
        .workflow_storage
        .get_file_metadata(workflow_id, &activity_key, &filename)
        .await
        .map_err(|e| match e {
            StorageError::FileNotFound(_) => {
                tracing::warn!("File not found: {}", filename);
                AppError::NotFound(format!("File '{}' not found", filename))
            }
            _ => {
                tracing::error!("Storage error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    // Stream file content
    let stream = state
        .workflow_storage
        .download_file(workflow_id, &activity_key, &filename)
        .await
        .map_err(|e| {
            tracing::error!("Storage error streaming file: {:?}", e);
            AppError::InternalError(anyhow::anyhow!(e))
        })?;

    // Convert stream to axum Body
    let body = Body::from_stream(stream);

    // Determine content type
    let content_type = metadata
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    tracing::debug!(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        filename = %filename,
        size = metadata.size,
        content_type = %content_type,
        "File download started"
    );

    // Build response with appropriate headers
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .header(header::CONTENT_LENGTH, metadata.size.to_string())
        .body(body)
        .unwrap()
        .into_response())
}
