use crate::error::{ApiResult, AppError};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use kruxiaflow_core::scheduler::{
    NewSchedule, OverlapPolicy, ScheduleError, ScheduleService, ScheduleUpdate, WorkflowSchedule,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Create schedule request
///
/// Exactly one of `cron` / `interval_seconds` is required. `cron` is standard
/// 5-field crontab (minute granularity) or 6-field with leading seconds,
/// evaluated in `timezone` (IANA name, default UTC).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateScheduleRequest {
    /// Unique schedule name; also part of each run's unique_key
    /// (`schedule:<name>:<occurrence epoch>`)
    #[schema(example = "cache-flush-sweep")]
    pub name: String,

    /// Workflow definition to submit
    #[schema(example = "cache_flush_sweep")]
    pub definition_name: String,

    /// Pinned definition version (default: latest at fire time)
    #[serde(default)]
    pub definition_version: Option<String>,

    /// Workflow input for each run
    #[serde(default)]
    #[schema(example = json!({}))]
    pub input: Option<serde_json::Value>,

    /// Crontab expression (5-field, or 6-field with leading seconds)
    #[serde(default)]
    #[schema(example = "*/2 * * * *")]
    pub cron: Option<String>,

    /// IANA timezone for cron evaluation (default UTC)
    #[serde(default)]
    #[schema(example = "America/Chicago")]
    pub timezone: Option<String>,

    /// Fixed interval in seconds (alternative to cron)
    #[serde(default)]
    #[schema(example = 120)]
    pub interval_seconds: Option<i64>,

    /// skip (default): don't submit while the previous run is still active;
    /// allow: always submit
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "skip")]
    pub overlap_policy: Option<OverlapPolicy>,

    /// Whether the schedule fires (default true)
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Update schedule request (PATCH semantics: absent = unchanged).
///
/// Providing `cron` or `interval_seconds` replaces the cadence wholesale and
/// recomputes `next_run_at`; re-enabling also recomputes it (a stale past-due
/// schedule does not fire immediately on re-enable).
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateScheduleRequest {
    /// New workflow input
    #[serde(default)]
    pub input: Option<serde_json::Value>,

    /// New crontab expression (switches the schedule to cron)
    #[serde(default)]
    pub cron: Option<String>,

    /// IANA timezone for cron evaluation
    #[serde(default)]
    pub timezone: Option<String>,

    /// New fixed interval in seconds (switches the schedule to interval)
    #[serde(default)]
    pub interval_seconds: Option<i64>,

    /// New overlap policy
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub overlap_policy: Option<OverlapPolicy>,

    /// Enable or disable the schedule
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Schedule resource representation
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleResponse {
    pub id: Uuid,
    #[schema(example = "cache-flush-sweep")]
    pub name: String,
    #[schema(example = "cache_flush_sweep")]
    pub definition_name: String,
    pub definition_version: Option<String>,
    pub input: serde_json::Value,
    #[schema(example = "*/2 * * * *")]
    pub cron: Option<String>,
    pub timezone: Option<String>,
    #[schema(example = 120)]
    pub interval_seconds: Option<i64>,
    #[schema(value_type = String, example = "skip")]
    pub overlap_policy: OverlapPolicy,
    pub enabled: bool,
    /// Next occurrence the scheduler will fire
    pub next_run_at: DateTime<Utc>,
    /// When the schedule last submitted a workflow
    pub last_run_at: Option<DateTime<Utc>>,
    /// The workflow submitted by the most recent firing
    pub last_workflow_id: Option<Uuid>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<WorkflowSchedule> for ScheduleResponse {
    fn from(s: WorkflowSchedule) -> Self {
        Self {
            id: s.id,
            name: s.name,
            definition_name: s.definition_name,
            definition_version: s.definition_version,
            input: s.input,
            cron: s.cron,
            timezone: s.timezone,
            interval_seconds: s.interval_seconds,
            overlap_policy: s.overlap_policy,
            enabled: s.enabled,
            next_run_at: s.next_run_at,
            last_run_at: s.last_run_at,
            last_workflow_id: s.last_workflow_id,
            created_by: s.created_by,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// List schedules response
#[derive(Debug, Serialize, ToSchema)]
pub struct ListSchedulesResponse {
    pub schedules: Vec<ScheduleResponse>,
    pub count: i64,
}

fn map_schedule_error(e: ScheduleError) -> AppError {
    match e {
        ScheduleError::NotFound(id) => AppError::NotFound(format!("Schedule '{}' not found", id)),
        ScheduleError::DuplicateName(name) => {
            AppError::Conflict(format!("Schedule name '{}' already exists", name))
        }
        ScheduleError::DefinitionNotFound(what) => {
            AppError::BadRequest(format!("Workflow definition not found: {}", what))
        }
        ScheduleError::Invalid(msg) => AppError::BadRequest(msg),
        ScheduleError::Database(e) => AppError::DatabaseError(e),
    }
}

/// Create a recurring schedule
///
/// Endpoint: POST /api/v1/schedules
///
/// The scheduler submits the referenced workflow definition on the given
/// cadence, server-side — no client credentials are involved in the
/// recurrence. Missed occurrences (downtime) collapse into at most one
/// catch-up run.
#[utoipa::path(
    post,
    path = "/api/v1/schedules",
    tag = "Schedules",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = ScheduleResponse),
        (status = 400, description = "Invalid cadence or unknown definition"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Schedule name already exists")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(skip(state, claims, request), fields(user = %claims.subject()))]
pub async fn create_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<CreateScheduleRequest>,
) -> ApiResult<(StatusCode, Json<ScheduleResponse>)> {
    let service = ScheduleService::new(state.db_pool.clone());

    let schedule = service
        .create(NewSchedule {
            name: request.name,
            definition_name: request.definition_name,
            definition_version: request.definition_version,
            input: request.input.unwrap_or_else(|| serde_json::json!({})),
            cron: request.cron,
            timezone: request.timezone,
            interval_seconds: request.interval_seconds,
            overlap_policy: request.overlap_policy.unwrap_or(OverlapPolicy::Skip),
            enabled: request.enabled.unwrap_or(true),
            created_by: Some(claims.subject().to_string()),
        })
        .await
        .map_err(map_schedule_error)?;

    tracing::info!(
        schedule_id = %schedule.id,
        name = %schedule.name,
        definition_name = %schedule.definition_name,
        next_run_at = %schedule.next_run_at,
        "Schedule created"
    );

    Ok((StatusCode::CREATED, Json(schedule.into())))
}

/// List schedules
///
/// Endpoint: GET /api/v1/schedules
#[utoipa::path(
    get,
    path = "/api/v1/schedules",
    tag = "Schedules",
    responses(
        (status = 200, description = "Schedules list", body = ListSchedulesResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_schedules(
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
) -> ApiResult<Json<ListSchedulesResponse>> {
    let service = ScheduleService::new(state.db_pool.clone());
    let schedules = service.list().await.map_err(map_schedule_error)?;
    let count = schedules.len() as i64;

    Ok(Json(ListSchedulesResponse {
        schedules: schedules.into_iter().map(Into::into).collect(),
        count,
    }))
}

/// Get a schedule by ID
///
/// Endpoint: GET /api/v1/schedules/{schedule_id}
#[utoipa::path(
    get,
    path = "/api/v1/schedules/{schedule_id}",
    tag = "Schedules",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule found", body = ScheduleResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Schedule not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_schedule(
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<Json<ScheduleResponse>> {
    let service = ScheduleService::new(state.db_pool.clone());
    let schedule = service.get(schedule_id).await.map_err(map_schedule_error)?;
    Ok(Json(schedule.into()))
}

/// Update a schedule (PATCH semantics)
///
/// Endpoint: PATCH /api/v1/schedules/{schedule_id}
#[utoipa::path(
    patch,
    path = "/api/v1/schedules/{schedule_id}",
    tag = "Schedules",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Schedule updated", body = ScheduleResponse),
        (status = 400, description = "Invalid cadence"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Schedule not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(skip(state, claims, request), fields(user = %claims.subject(), schedule_id = %schedule_id))]
pub async fn update_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path(schedule_id): Path<Uuid>,
    Json(request): Json<UpdateScheduleRequest>,
) -> ApiResult<Json<ScheduleResponse>> {
    let service = ScheduleService::new(state.db_pool.clone());

    let schedule = service
        .update(
            schedule_id,
            ScheduleUpdate {
                input: request.input,
                cron: request.cron,
                timezone: request.timezone,
                interval_seconds: request.interval_seconds,
                overlap_policy: request.overlap_policy,
                enabled: request.enabled,
            },
        )
        .await
        .map_err(map_schedule_error)?;

    tracing::info!(
        schedule_id = %schedule.id,
        name = %schedule.name,
        enabled = schedule.enabled,
        next_run_at = %schedule.next_run_at,
        "Schedule updated"
    );

    Ok(Json(schedule.into()))
}

/// Delete a schedule
///
/// Endpoint: DELETE /api/v1/schedules/{schedule_id}
#[utoipa::path(
    delete,
    path = "/api/v1/schedules/{schedule_id}",
    tag = "Schedules",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 204, description = "Schedule deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Schedule not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(skip(state, claims), fields(user = %claims.subject(), schedule_id = %schedule_id))]
pub async fn delete_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let service = ScheduleService::new(state.db_pool.clone());
    service
        .delete(schedule_id)
        .await
        .map_err(map_schedule_error)?;

    tracing::info!(schedule_id = %schedule_id, "Schedule deleted");

    Ok(StatusCode::NO_CONTENT)
}
