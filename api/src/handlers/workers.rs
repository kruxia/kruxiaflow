use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{Extension, Json, extract::Path};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use kruxiaflow_core::activity::{ActivityWorkerError, ActivityWorkerService};
use utoipa::ToSchema;
use uuid::Uuid;

/// Poll for activities request
///
/// Workers poll for pending activities by specifying which activity types
/// they can execute. The API returns activities matching those types.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PollActivitiesRequest {
    /// Activity types this worker can execute (format: "worker.name")
    #[schema(example = json!(["builtin.http_request", "builtin.postgres_query"]))]
    pub activity_types: Vec<String>,

    /// Worker instance ID (for tracking which worker claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,

    /// Maximum number of activities to return (default 1, max 100)
    #[serde(default = "default_max_activities")]
    pub max_activities: usize,
}

fn default_max_activities() -> usize {
    1
}

impl PollActivitiesRequest {
    /// Validate request structure
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.activity_types.is_empty() {
            errors.add("activity_types", "At least one activity type is required");
        }

        // Validate activity type format (worker.name)
        for activity_type in &self.activity_types {
            if !activity_type.contains('.') {
                errors.add(
                    "activity_types",
                    &format!("Invalid format '{}': must be 'worker.name'", activity_type),
                );
            }
        }

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        if self.max_activities == 0 {
            errors.add("max_activities", "max_activities must be at least 1");
        }
        if self.max_activities > 100 {
            errors.add("max_activities", "max_activities must be at most 100");
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity for worker execution
#[derive(Debug, Serialize, ToSchema)]
pub struct PendingActivity {
    /// Unique activity ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub activity_id: Uuid,

    /// Workflow ID this activity belongs to
    #[schema(example = "660e8400-e29b-41d4-a716-446655440001")]
    pub workflow_id: Uuid,

    /// Activity key (unique within workflow)
    #[schema(example = "authorize_card")]
    pub activity_key: String,

    /// Activity worker type
    #[schema(example = "builtin")]
    pub worker: String,

    /// Activity name
    #[schema(example = "http_request")]
    pub activity_name: String,

    /// Activity input parameters
    #[schema(example = json!({"card_token": "tok_123", "amount": 100.00}))]
    pub parameters: serde_json::Value,

    /// Activity settings (timeout, retry, etc.)
    #[schema(example = json!({"timeout": 300, "max_retries": 3}))]
    pub settings: Option<serde_json::Value>,

    /// Timeout in seconds (extracted from settings for convenience)
    #[schema(example = 300)]
    pub timeout_seconds: Option<i64>,

    /// Output definitions (for file outputs)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"name": "document", "type": "file"}]))]
    pub output_definitions: Option<serde_json::Value>,
}

/// Poll for activities response
#[derive(Debug, Serialize, ToSchema)]
pub struct PollActivitiesResponse {
    /// List of pending activities (may be empty if none available)
    pub activities: Vec<PendingActivity>,

    /// Number of activities returned
    pub count: usize,
}

/// Activity heartbeat request
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActivityHeartbeatRequest {
    /// Worker instance ID (must match the worker that claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,
}

impl ActivityHeartbeatRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity heartbeat response
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityHeartbeatResponse {
    /// Heartbeat acknowledged
    #[schema(example = true)]
    pub acknowledged: bool,

    /// Recommended seconds until next heartbeat
    #[schema(example = 30)]
    pub next_heartbeat_seconds: i64,
}

/// Activity completion request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteActivityRequest {
    /// Worker instance ID (must match the worker that claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,

    /// Activity output (result of execution)
    #[schema(example = json!({"authorization_id": "auth_123", "approved": true}))]
    pub output: serde_json::Value,

    /// Cost in USD (for AI/LLM activities, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.015)]
    pub cost_usd: Option<Decimal>,
}

impl CompleteActivityRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        // Validate output is an object
        if !self.output.is_object() {
            errors.add("output", "Output must be a JSON object");
        }

        // Validate cost_usd if provided
        if let Some(cost) = self.cost_usd {
            if cost < Decimal::ZERO {
                errors.add("cost_usd", "Cost must be non-negative");
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity completion response
#[derive(Debug, Serialize, ToSchema)]
pub struct CompleteActivityResponse {
    /// Completion acknowledged
    #[schema(example = true)]
    pub acknowledged: bool,
}

/// Activity error details
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ActivityError {
    /// Error code (for categorization)
    #[schema(example = "PAYMENT_DECLINED")]
    pub code: String,

    /// Error message (human-readable)
    #[schema(example = "Card was declined by the bank")]
    pub message: String,

    /// Whether this error is retryable
    #[schema(example = false)]
    pub retryable: bool,
}

/// Activity failure request
#[derive(Debug, Deserialize, ToSchema)]
pub struct FailActivityRequest {
    /// Worker instance ID (must match the worker that claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,

    /// Error details
    pub error: ActivityError,
}

impl FailActivityRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        if self.error.code.is_empty() {
            errors.add("error.code", "Error code cannot be empty");
        }

        if self.error.message.is_empty() {
            errors.add("error.message", "Error message cannot be empty");
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity failure response
#[derive(Debug, Serialize, ToSchema)]
pub struct FailActivityResponse {
    /// Failure acknowledged
    #[schema(example = true)]
    pub acknowledged: bool,

    /// Whether the activity will be retried
    #[schema(example = true)]
    pub will_retry: bool,
}

/// Poll for activities
///
/// Endpoint: POST /api/v1/workers/poll
///
/// Workers poll for pending activities matching their capabilities.
/// Activities are claimed atomically using FOR UPDATE SKIP LOCKED.
#[utoipa::path(
    post,
    path = "/api/v1/workers/poll",
    tag = "Workers",
    request_body = PollActivitiesRequest,
    responses(
        (status = 200, description = "Activities claimed (may be empty list)", body = PollActivitiesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn poll_activities(
    service: ActivityWorkerService,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<PollActivitiesRequest>,
) -> ApiResult<Json<PollActivitiesResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::debug!(
        worker_id = %request.worker_id,
        activity_types = ?request.activity_types,
        max_activities = request.max_activities,
        user = %claims.subject(),
        "Polling for activities"
    );

    // Parse activity types (worker.name → (worker, name))
    let activity_types: Vec<(String, String)> = request
        .activity_types
        .iter()
        .filter_map(|t| {
            let parts: Vec<&str> = t.splitn(2, '.').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect();

    // Poll for activities
    let activities = service
        .poll_activities(
            activity_types,
            request.worker_id.clone(),
            request.max_activities,
        )
        .await
        .map_err(|e| {
            tracing::error!("Error polling activities: {:?}", e);
            AppError::InternalError(anyhow::anyhow!(e))
        })?;

    let count = activities.len();

    if count > 0 {
        tracing::debug!(
            worker_id = %request.worker_id,
            claimed_count = count,
            "Activities claimed"
        );
    }

    Ok(Json(PollActivitiesResponse {
        activities: activities
            .into_iter()
            .map(|a| {
                // Extract timeout from settings
                let timeout_seconds = a
                    .settings
                    .as_ref()
                    .and_then(|s| s.get("timeout"))
                    .and_then(|t| t.as_i64());

                PendingActivity {
                    activity_id: a.id,
                    workflow_id: a.workflow_id,
                    activity_key: a.activity_key,
                    worker: a.worker,
                    activity_name: a.activity_name,
                    parameters: a.parameters,
                    settings: a.settings,
                    timeout_seconds,
                    output_definitions: a.output_definitions,
                }
            })
            .collect(),
        count,
    }))
}

/// Send activity heartbeat
///
/// Endpoint: POST /api/v1/activities/{activity_id}/heartbeat
///
/// Workers send periodic heartbeats for long-running activities to prevent timeout.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/heartbeat",
    tag = "Workers",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID")
    ),
    request_body = ActivityHeartbeatRequest,
    responses(
        (status = 200, description = "Heartbeat acknowledged", body = ActivityHeartbeatResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Activity not found"),
        (status = 409, description = "Activity already completed or wrong worker")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn heartbeat_activity(
    service: ActivityWorkerService,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<ActivityHeartbeatRequest>,
) -> ApiResult<Json<ActivityHeartbeatResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::debug!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        "Heartbeat received"
    );

    let next_heartbeat_seconds = service
        .heartbeat_activity(activity_id, request.worker_id.clone())
        .await
        .map_err(|e| match e {
            ActivityWorkerError::ActivityNotFound(id) => {
                tracing::warn!("Activity not found: {}", id);
                AppError::NotFound(format!("Activity '{}' not found", id))
            }
            ActivityWorkerError::ActivityAlreadyCompleted => {
                tracing::warn!("Activity already completed: {}", activity_id);
                AppError::Conflict("Activity already completed".to_string())
            }
            ActivityWorkerError::WrongWorker => {
                tracing::warn!("Wrong worker for activity {}", activity_id);
                AppError::Conflict("Activity claimed by different worker".to_string())
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(ActivityHeartbeatResponse {
        acknowledged: true,
        next_heartbeat_seconds,
    }))
}

/// Complete activity successfully
///
/// Endpoint: POST /api/v1/activities/{activity_id}/complete
///
/// Workers report successful activity completion with output.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/complete",
    tag = "Workers",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID")
    ),
    request_body = CompleteActivityRequest,
    responses(
        (status = 200, description = "Activity completed", body = CompleteActivityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Activity not found"),
        (status = 409, description = "Activity already completed or wrong worker"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn complete_activity(
    service: ActivityWorkerService,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<CompleteActivityRequest>,
) -> ApiResult<Json<CompleteActivityResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::debug!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        "Completing activity"
    );

    service
        .complete_activity(
            activity_id,
            request.worker_id.clone(),
            request.output,
            request.cost_usd,
        )
        .await
        .map_err(|e| match e {
            ActivityWorkerError::ActivityNotFound(id) => {
                tracing::warn!("Activity not found: {}", id);
                AppError::NotFound(format!("Activity '{}' not found", id))
            }
            ActivityWorkerError::ActivityAlreadyCompleted => {
                tracing::warn!("Activity already completed: {}", activity_id);
                AppError::Conflict("Activity already completed".to_string())
            }
            ActivityWorkerError::WrongWorker => {
                tracing::warn!("Wrong worker for activity {}", activity_id);
                AppError::Conflict("Activity claimed by different worker".to_string())
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(CompleteActivityResponse { acknowledged: true }))
}

/// Fail activity
///
/// Endpoint: POST /api/v1/activities/{activity_id}/fail
///
/// Workers report activity failure with error details.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/fail",
    tag = "Workers",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID")
    ),
    request_body = FailActivityRequest,
    responses(
        (status = 200, description = "Activity failure recorded", body = FailActivityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Activity not found"),
        (status = 409, description = "Activity already completed or wrong worker"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn fail_activity(
    service: ActivityWorkerService,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<FailActivityRequest>,
) -> ApiResult<Json<FailActivityResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::warn!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        error_code = %request.error.code,
        "Activity failed"
    );

    let will_retry = service
        .fail_activity(
            activity_id,
            request.worker_id.clone(),
            request.error.code,
            request.error.message,
            request.error.retryable,
        )
        .await
        .map_err(|e| match e {
            ActivityWorkerError::ActivityNotFound(id) => {
                tracing::warn!("Activity not found: {}", id);
                AppError::NotFound(format!("Activity '{}' not found", id))
            }
            ActivityWorkerError::ActivityAlreadyCompleted => {
                tracing::warn!("Activity already completed: {}", activity_id);
                AppError::Conflict("Activity already completed".to_string())
            }
            ActivityWorkerError::WrongWorker => {
                tracing::warn!("Wrong worker for activity {}", activity_id);
                AppError::Conflict("Activity claimed by different worker".to_string())
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(FailActivityResponse {
        acknowledged: true,
        will_retry,
    }))
}
