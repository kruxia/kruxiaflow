use crate::dto;
use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use streamflow_core::workflow::{RepositoryError, WorkflowDefinitionRepository};
use utoipa::ToSchema;

/// Deploy workflow definition request
///
/// Note: Version is auto-generated at deployment time (timestamp-based).
/// Users only provide the workflow name and definition.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeployWorkflowDefinitionRequest {
    /// Workflow definition (version auto-generated)
    #[serde(flatten)]
    pub definition: dto::WorkflowDefinition,
}

/// Deploy workflow definition response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeployWorkflowDefinitionResponse {
    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// When the definition was deployed
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// List workflow definitions response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListWorkflowDefinitionsResponse {
    /// All workflow definitions
    pub definitions: Vec<WorkflowDefinitionSummary>,

    /// Total count
    pub total: usize,
}

/// Workflow definition summary (without full definition body)
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowDefinitionSummary {
    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// Number of activities
    pub activity_count: usize,

    /// When deployed
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Get workflow definition response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GetWorkflowDefinitionResponse {
    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// Activities in this workflow
    pub activities: Vec<dto::ActivityDefinition>,

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
/// generated from the deployment timestamp (format: YYYYmmdd.HHMMSS.uuuuuu).
///
/// Validation includes:
/// - Syntax validation (valid YAML structure)
/// - Semantic validation (activity references, no cycles)
/// - Graph structure validation (no cycles)
///
/// Returns 409 Conflict if a definition with the same name and timestamp already exists
/// (virtually impossible due to microsecond precision).
#[utoipa::path(
    post,
    path = "/api/v1/workflow_definitions",
    tag = "Workflow Definitions",
    request_body = DeployWorkflowDefinitionRequest,
    responses(
        (status = 201, description = "Workflow definition deployed successfully", body = DeployWorkflowDefinitionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Workflow definition already exists"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn deploy_workflow_definition(
    repo: WorkflowDefinitionRepository,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<DeployWorkflowDefinitionRequest>,
) -> ApiResult<(StatusCode, Json<DeployWorkflowDefinitionResponse>)> {
    tracing::info!(
        workflow_name = %request.definition.name,
        user = %claims.subject(),
        "Deploying workflow definition (version will be auto-generated)"
    );

    // Store definition (includes validation)
    let stored = repo
        .store(request.definition.into())
        .await
        .map_err(|e| match e {
            RepositoryError::ValidationError(ve) => {
                tracing::warn!("Workflow definition validation failed: {:?}", ve);
                AppError::ValidationError(ValidationErrors::from_workflow_validation(ve))
            }
            RepositoryError::DuplicateVersion { name, version } => {
                tracing::warn!(
                    "Duplicate workflow definition: {} version {}",
                    name,
                    version
                );
                AppError::Conflict(format!(
                    "Workflow definition '{}' version '{}' already exists",
                    name, version
                ))
            }
            RepositoryError::DatabaseError(e) => {
                tracing::error!("Database error storing workflow definition: {:?}", e);
                AppError::DatabaseError(e)
            }
            RepositoryError::SerializationError(e) => {
                tracing::error!("Serialization error: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
            RepositoryError::InvalidVersion { .. } => {
                // This shouldn't happen in deploy (no version provided by user)
                tracing::error!("Unexpected InvalidVersion error during deploy: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(DeployWorkflowDefinitionResponse {
            name: stored.name.clone(),
            version: stored.version.clone(),
            created_at: stored.created_at,
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
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_workflow_definitions(
    repo: WorkflowDefinitionRepository,
    Extension(_claims): Extension<ValidatedClaims>,
) -> ApiResult<Json<ListWorkflowDefinitionsResponse>> {
    let definitions = repo.list().await.map_err(|e| {
        tracing::error!("Failed to list workflow definitions: {:?}", e);
        match e {
            RepositoryError::DatabaseError(db_err) => AppError::DatabaseError(db_err),
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        }
    })?;

    let summaries: Vec<WorkflowDefinitionSummary> = definitions
        .into_iter()
        .map(|d| WorkflowDefinitionSummary {
            name: d.name,
            version: d.version,
            activity_count: d.activities.len(),
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
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow definition not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_workflow_definition(
    repo: WorkflowDefinitionRepository,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(name): Path<String>,
    Query(query): Query<GetWorkflowDefinitionQuery>,
) -> ApiResult<Json<GetWorkflowDefinitionResponse>> {
    let stored = if let Some(version) = query.version {
        // Get specific version
        repo
            .get(&name, &version)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get workflow definition: {:?}", e);
                match e {
                    RepositoryError::InvalidVersion { version, error } => {
                        AppError::BadRequest(format!(
                            "Invalid version format '{}': {}. Expected format: YYYYmmdd.HHMMSS.uuuuuu",
                            version, error
                        ))
                    }
                    RepositoryError::DatabaseError(db_err) => AppError::DatabaseError(db_err),
                    _ => AppError::InternalError(anyhow::anyhow!(e)),
                }
            })?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "Workflow definition '{}' version '{}' not found",
                    name, version
                ))
            })?
    } else {
        // Get latest version
        repo.get_latest(&name)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get latest workflow definition: {:?}", e);
                match e {
                    RepositoryError::DatabaseError(db_err) => AppError::DatabaseError(db_err),
                    _ => AppError::InternalError(anyhow::anyhow!(e)),
                }
            })?
            .ok_or_else(|| {
                AppError::NotFound(format!("Workflow definition '{}' not found", name))
            })?
    };

    Ok(Json(GetWorkflowDefinitionResponse {
        name: stored.name,
        version: stored.version,
        activities: stored.activities.into_iter().map(Into::into).collect(),
        created_at: stored.created_at,
    }))
}
