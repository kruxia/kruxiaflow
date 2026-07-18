use crate::dto;
use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{
    Extension, Json,
    extract::{Path, Query},
    http::StatusCode,
};
use kruxiaflow_core::workflow::{RepositoryError, WorkflowDefinitionRepository};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Deploy workflow definition request (accepts both JSON and YAML)
///
/// Note: Version is auto-generated at deployment time (timestamp-based).
/// Users provide the workflow as either JSON or YAML (JSON is valid YAML).
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

    /// True if definition was identical to existing version (no new version created)
    /// Only present when unchanged is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unchanged: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deploy_response_serialize() {
        let response = DeployWorkflowDefinitionResponse {
            name: "my-workflow".to_string(),
            version: "20260201.120000.000000".to_string(),
            created_at: chrono::Utc::now(),
            unchanged: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["name"], "my-workflow");
        assert!(json.get("unchanged").is_none()); // skip_serializing_if
    }

    #[test]
    fn test_deploy_response_serialize_unchanged() {
        let response = DeployWorkflowDefinitionResponse {
            name: "wf".to_string(),
            version: "v1".to_string(),
            created_at: chrono::Utc::now(),
            unchanged: Some(true),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["unchanged"], true);
    }

    #[test]
    fn test_deploy_response_deserialize() {
        let json = r#"{"name":"wf","version":"v1","created_at":"2026-01-01T00:00:00Z"}"#;
        let response: DeployWorkflowDefinitionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.name, "wf");
        assert!(response.unchanged.is_none());
    }

    #[test]
    fn test_list_response_serialize() {
        let response = ListWorkflowDefinitionsResponse {
            definitions: vec![WorkflowDefinitionSummary {
                name: "wf1".to_string(),
                version: "v1".to_string(),
                activity_count: 5,
                created_at: chrono::Utc::now(),
            }],
            total: 1,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["definitions"][0]["activity_count"], 5);
    }

    #[test]
    fn test_get_definition_query_no_version() {
        let json = r#"{}"#;
        let query: GetWorkflowDefinitionQuery = serde_json::from_str(json).unwrap();
        assert!(query.version.is_none());
    }

    #[test]
    fn test_get_definition_query_with_version() {
        let json = r#"{"version": "20260201.120000.000000"}"#;
        let query: GetWorkflowDefinitionQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.version, Some("20260201.120000.000000".to_string()));
    }

    #[test]
    fn test_get_definition_response_serialize() {
        let response = GetWorkflowDefinitionResponse {
            name: "test-wf".to_string(),
            version: "v1".to_string(),
            activities: vec![],
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["name"], "test-wf");
        assert_eq!(json["activities"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_workflow_definition_summary_serialize() {
        let summary = WorkflowDefinitionSummary {
            name: "wf".to_string(),
            version: "v1".to_string(),
            activity_count: 3,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["name"], "wf");
        assert_eq!(json["activity_count"], 3);
    }

    // =========================================================================
    // Handler integration tests
    // =========================================================================

    #[allow(unused_imports)]
    use crate::state::tests::*;
    use axum::extract::Query;
    use kruxiaflow_core::workflow::WorkflowDefinitionRepository;
    use kruxiaflow_oauth::Claims;
    use sqlx::PgPool;

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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_deploy_workflow_definition_valid_yaml(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let yaml = r#"
name: test-deploy-handler
activities:
  - key: step1
    activity_type: std.echo
    params:
      message: hello
"#;

        let result =
            deploy_workflow_definition(repo, Extension(test_claims()), yaml.to_string()).await;

        assert!(result.is_ok());
        let (status, Json(response)) = result.unwrap();
        assert!(status == StatusCode::CREATED || status == StatusCode::OK);
        assert_eq!(response.name, "test-deploy-handler");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_deploy_workflow_definition_invalid_yaml(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let yaml = "this is not valid yaml: [[[";

        let result =
            deploy_workflow_definition(repo, Extension(test_claims()), yaml.to_string()).await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_deploy_workflow_definition_missing_name(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let yaml = r#"
activities:
  - key: step1
    activity_type: std.echo
    params:
      message: hello
"#;

        let result =
            deploy_workflow_definition(repo, Extension(test_claims()), yaml.to_string()).await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_deploy_workflow_definition_idempotent(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool.clone());

        let yaml = r#"
name: test-deploy-idempotent
activities:
  - key: step1
    activity_type: std.echo
    params:
      message: hello
"#;

        // First deploy
        let result1 =
            deploy_workflow_definition(repo, Extension(test_claims()), yaml.to_string()).await;
        assert!(result1.is_ok());

        // Second deploy with same content
        let repo2 = WorkflowDefinitionRepository::new(pool);
        let result2 =
            deploy_workflow_definition(repo2, Extension(test_claims()), yaml.to_string()).await;
        assert!(result2.is_ok());
        let (status, Json(response)) = result2.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response.unchanged, Some(true));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_list_workflow_definitions_handler(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let result = list_workflow_definitions(repo, Extension(test_claims())).await;

        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert_eq!(response.total, response.definitions.len());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_workflow_definition_latest(pool: PgPool) {
        // First deploy a definition
        let repo = WorkflowDefinitionRepository::new(pool.clone());
        let yaml = r#"
name: test-get-latest-handler
activities:
  - key: step1
    activity_type: std.echo
    params:
      message: hello
"#;
        let _ = deploy_workflow_definition(repo, Extension(test_claims()), yaml.to_string())
            .await
            .unwrap();

        // Now get it
        let repo2 = WorkflowDefinitionRepository::new(pool);
        let result = get_workflow_definition(
            repo2,
            Extension(test_claims()),
            Path("test-get-latest-handler".to_string()),
            Query(GetWorkflowDefinitionQuery { version: None }),
        )
        .await;

        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert_eq!(response.name, "test-get-latest-handler");
        assert_eq!(response.activities.len(), 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_workflow_definition_not_found(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let result = get_workflow_definition(
            repo,
            Extension(test_claims()),
            Path("nonexistent-workflow-def".to_string()),
            Query(GetWorkflowDefinitionQuery { version: None }),
        )
        .await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_workflow_definition_specific_version_not_found(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let result = get_workflow_definition(
            repo,
            Extension(test_claims()),
            Path("nonexistent-wf".to_string()),
            Query(GetWorkflowDefinitionQuery {
                version: Some("20260101.000000.000000".to_string()),
            }),
        )
        .await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_workflow_definition_invalid_version_format(pool: PgPool) {
        let repo = WorkflowDefinitionRepository::new(pool);

        let result = get_workflow_definition(
            repo,
            Extension(test_claims()),
            Path("some-wf".to_string()),
            Query(GetWorkflowDefinitionQuery {
                version: Some("not-a-valid-version".to_string()),
            }),
        )
        .await;

        assert!(result.is_err());
    }
}

/// Deploy workflow definition (idempotent)
///
/// Endpoint: POST /api/v1/workflow_definitions
///
/// Accepts workflow definitions in either JSON or YAML format (JSON is valid YAML).
/// The version is automatically generated from the deployment timestamp.
///
/// **Idempotent behavior:**
/// - If the definition is identical to an existing version, returns the existing version (200 OK)
/// - If the definition is new or changed, creates a new version (201 Created)
///
/// Validation includes:
/// - Syntax validation (valid YAML/JSON structure)
/// - Semantic validation (activity references, no cycles)
/// - Graph structure validation (no cycles)
///
/// Content-Type headers:
/// - application/json - JSON format
/// - text/yaml or application/yaml - YAML format
/// - If not specified, parses as YAML (handles both)
///
/// Returns 409 Conflict if a definition with the same name and timestamp already exists
/// (virtually impossible due to microsecond precision).
#[utoipa::path(
    post,
    path = "/api/v1/workflow_definitions",
    tag = "Workflow Definitions",
    request_body(content = String, description = "Workflow definition in JSON or YAML format", content_type = "application/json"),
    responses(
        (status = 200, description = "Definition unchanged (identical to existing version)", body = DeployWorkflowDefinitionResponse),
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
    body: String,
) -> ApiResult<(StatusCode, Json<DeployWorkflowDefinitionResponse>)> {
    tracing::info!(
        user = %claims.subject(),
        "Deploying workflow definition from YAML/JSON (version will be auto-generated)"
    );

    // Parse as YAML (this handles both JSON and YAML since JSON is valid YAML)
    let definition =
        kruxiaflow_core::workflow::WorkflowDefinition::from_yaml(&body).map_err(|e| {
            tracing::warn!("Workflow parsing/validation failed: {:?}", e);
            AppError::ValidationError(ValidationErrors::from_workflow_validation(e))
        })?;

    tracing::info!(
        workflow_name = %definition.name,
        activity_count = definition.activities.len(),
        "Workflow parsed successfully, storing definition"
    );

    // Store definition (idempotent - returns existing if identical)
    let result = repo.store(definition).await.map_err(|e| match e {
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

    let status = if result.is_new {
        tracing::info!(
            workflow_name = %result.definition.name,
            version = %result.definition.version,
            "New workflow definition version created"
        );
        StatusCode::CREATED
    } else {
        tracing::info!(
            workflow_name = %result.definition.name,
            version = %result.definition.version,
            "Identical workflow definition already exists, returning existing version"
        );
        StatusCode::OK
    };

    let unchanged = if result.is_new { None } else { Some(true) };

    Ok((
        status,
        Json(DeployWorkflowDefinitionResponse {
            name: result.definition.name.clone(),
            version: result.definition.version.clone(),
            created_at: result.definition.created_at,
            unchanged,
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
