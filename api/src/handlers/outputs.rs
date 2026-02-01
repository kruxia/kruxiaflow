//! Output Retrieval Handlers
//!
//! Handlers for activity output retrieval, file download, and file upload endpoints.

use crate::dto::{
    GetActivityOutputResponse, GetWorkflowOutputResponse, UploadActivityFileResponse,
};
use crate::error::{ApiResult, AppError};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::body::Bytes;
use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use kruxiaflow_core::storage::StorageError;
use kruxiaflow_core::workflow::{OutputQueryError, OutputQueryService};
use std::pin::Pin;
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

/// Upload activity file
///
/// Endpoint: POST /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
///
/// Streams the request body directly to workflow storage without buffering the entire file in memory.
/// Returns 201 Created with file metadata on success.
///
/// The Content-Type header from the request is preserved as the file's content type.
/// If not provided, defaults to application/octet-stream.
#[utoipa::path(
    post,
    path = "/api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}",
    tag = "Outputs",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID"),
        ("activity_key" = String, Path, description = "Activity key"),
        ("filename" = String, Path, description = "Filename to store")
    ),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "File content as streaming bytes"
    ),
    responses(
        (status = 201, description = "File uploaded successfully", body = UploadActivityFileResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(
    skip(state, claims, headers, body),
    fields(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        filename = %filename,
        user = %claims.subject()
    )
)]
pub async fn upload_activity_file(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path((workflow_id, activity_key, filename)): Path<(Uuid, String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    // Extract content type from request headers
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    tracing::debug!(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        filename = %filename,
        content_type = ?content_type,
        "File upload started"
    );

    // Convert axum Body to a stream compatible with WorkflowStorage::upload_file
    let body_stream = body.into_data_stream();
    let stream = body_stream.map(|result| {
        result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    });

    // Pin the stream for upload_file
    let pinned_stream: Pin<
        Box<dyn futures::Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Unpin>,
    > = Box::pin(stream);

    // Upload the file using streaming storage
    let metadata = state
        .workflow_storage
        .upload_file(
            workflow_id,
            &activity_key,
            &filename,
            content_type.as_deref(),
            pinned_stream,
        )
        .await
        .map_err(|e| {
            tracing::error!("Storage error uploading file: {:?}", e);
            AppError::InternalError(anyhow::anyhow!(e))
        })?;

    tracing::info!(
        workflow_id = %workflow_id,
        activity_key = %activity_key,
        filename = %filename,
        size = metadata.size,
        content_type = ?metadata.content_type,
        "File upload completed"
    );

    // Return success response with file metadata
    let response = UploadActivityFileResponse::from(metadata);

    Ok(Response::builder()
        .status(StatusCode::CREATED)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap()
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::auth::ValidatedClaims;
    use crate::state::AppState;
    use crate::state::tests::*;
    use axum::extract::State;
    use kruxiaflow_core::cache::NoOpCache;
    use kruxiaflow_oauth::Claims;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn test_claims() -> ValidatedClaims {
        ValidatedClaims(Claims {
            sub: "test_user".to_string(),
            jti: "test_jti".to_string(),
            iss: "test".to_string(),
            aud: "test".to_string(),
            exp: 9999999999,
            iat: 1000000000,
        })
    }

    async fn setup_test_state() -> AppState {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });
        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn test_get_activity_output_workflow_not_found() {
        let state = setup_test_state().await;
        let workflow_id = Uuid::now_v7();

        let result = get_activity_output(
            State(state),
            Extension(test_claims()),
            Path((workflow_id, "step1".to_string())),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_workflow_output_workflow_not_found() {
        let state = setup_test_state().await;
        let workflow_id = Uuid::now_v7();

        let result =
            get_workflow_output(State(state), Extension(test_claims()), Path(workflow_id)).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_activity_file_not_found() {
        // MockWorkflowStorage.get_file_metadata returns Ok with dummy data,
        // so this will succeed at metadata but the download stream is empty.
        let state = setup_test_state().await;
        let workflow_id = Uuid::now_v7();

        let result = download_activity_file(
            State(state),
            Extension(test_claims()),
            Path((workflow_id, "step1".to_string(), "test.txt".to_string())),
        )
        .await;

        // MockWorkflowStorage returns Ok for get_file_metadata and download_file
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_upload_activity_file() {
        let state = setup_test_state().await;
        let workflow_id = Uuid::now_v7();

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());

        let body = Body::from("test file content");

        let result = upload_activity_file(
            State(state),
            Extension(test_claims()),
            Path((workflow_id, "step1".to_string(), "test.txt".to_string())),
            headers,
            body,
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_upload_activity_file_no_content_type() {
        let state = setup_test_state().await;
        let workflow_id = Uuid::now_v7();

        let headers = HeaderMap::new();
        let body = Body::from("test content");

        let result = upload_activity_file(
            State(state),
            Extension(test_claims()),
            Path((workflow_id, "step1".to_string(), "file.bin".to_string())),
            headers,
            body,
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // Test with a storage that returns errors
    struct ErrorWorkflowStorage;

    #[async_trait::async_trait]
    impl kruxiaflow_core::storage::WorkflowStorage for ErrorWorkflowStorage {
        async fn upload_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
            _content_type: Option<&str>,
            _data: Pin<
                Box<
                    dyn futures::Stream<
                            Item = std::result::Result<axum::body::Bytes, std::io::Error>,
                        > + Send
                        + Unpin,
                >,
            >,
        ) -> Result<kruxiaflow_core::storage::FileMetadata, StorageError> {
            Err(StorageError::FileNotFound("test.txt".to_string()))
        }

        async fn download_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<
            Pin<
                Box<
                    dyn futures::Stream<
                            Item = std::result::Result<axum::body::Bytes, std::io::Error>,
                        > + Send,
                >,
            >,
            StorageError,
        > {
            Err(StorageError::FileNotFound("test.txt".to_string()))
        }

        async fn get_file_metadata(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<kruxiaflow_core::storage::FileMetadata, StorageError> {
            Err(StorageError::FileNotFound("test.txt".to_string()))
        }

        async fn list_files(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> Result<Vec<kruxiaflow_core::storage::FileMetadata>, StorageError> {
            Ok(vec![])
        }

        async fn delete_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<(), StorageError> {
            Ok(())
        }

        async fn delete_workflow_files(&self, _workflow_id: Uuid) -> Result<(), StorageError> {
            Ok(())
        }

        async fn get_file_reference(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<String, StorageError> {
            Err(StorageError::FileNotFound("test.txt".to_string()))
        }
    }

    async fn setup_error_storage_state() -> AppState {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });
        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(ErrorWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn test_download_file_storage_error() {
        let state = setup_error_storage_state().await;
        let workflow_id = Uuid::now_v7();

        let result = download_activity_file(
            State(state),
            Extension(test_claims()),
            Path((workflow_id, "step1".to_string(), "missing.txt".to_string())),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_upload_file_storage_error() {
        let state = setup_error_storage_state().await;
        let workflow_id = Uuid::now_v7();

        let headers = HeaderMap::new();
        let body = Body::from("data");

        let result = upload_activity_file(
            State(state),
            Extension(test_claims()),
            Path((workflow_id, "step1".to_string(), "test.txt".to_string())),
            headers,
            body,
        )
        .await;

        assert!(result.is_err());
    }
}
