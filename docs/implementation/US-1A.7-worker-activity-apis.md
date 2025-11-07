# Implementation Plan: US-1A.7 Worker Activity APIs

**Epic**: 1A - API Server
**User Story**: US-1A.7
**Status**: 📋 Ready for Implementation
**Priority**: P0 (Must Have for MVP)
**Last Updated**: 2025-11-06

---

## User Story

**As** an activity worker
**I want** HTTP APIs to poll for activities, send heartbeats, and report results
**So that** I can execute activities in distributed environments without direct database access

### Acceptance Criteria

- `POST /api/v1/workers/poll` - Poll for activities by activity type
  - Request body: `{activity_types: ["namespace.name"], worker_id, max_activities: 10}`
  - Response: `[{activity_id, workflow_id, activity_key, parameters, timeout}]`
  - Uses ActivityQueue::poll() internally with FOR UPDATE SKIP LOCKED
- `POST /api/v1/activities/{activity_id}/heartbeat` - Send heartbeat to prevent timeout
  - Request body: `{worker_id}`
  - Response: `{acknowledged: true, next_heartbeat_seconds: 30}`
- `POST /api/v1/activities/{activity_id}/complete` - Report successful completion
  - Request body: `{worker_id, output, cost_usd}`
  - Response: `{acknowledged: true}`
  - Publishes activity completion event to workflow orchestrator
- `POST /api/v1/activities/{activity_id}/fail` - Report activity failure
  - Request body: `{worker_id, error: {code, message, retryable}}`
  - Response: `{acknowledged: true, will_retry: boolean}`
- Worker authentication: Bearer token
- Timeout handling: Activities not heartbeat within timeout are released for retry
- Idempotency: Activities can only be completed/failed once (409 Conflict if already done or timed out/reassigned)

---

## Rationale

This user story enables activity workers (both built-in and external) to interact with the orchestration system via HTTP APIs. It completes the workflow execution cycle: submit workflow → orchestrator schedules activities → workers execute activities → report results → orchestrator continues workflow.

**Why This Story is Critical**:
- Enables distributed activity execution (workers on different machines)
- Provides same API for built-in and external workers (consistency)
- Supports multiple programming languages (any HTTP client)
- Enables horizontal scaling of workers (add more workers = more throughput)
- Handles long-running activities (heartbeat mechanism)
- Provides idempotent operations (safe retry on network failure)

**Key Design Decisions**:
1. **Unified API for All Workers**: Built-in and external workers use same endpoints
   - Built-in worker validates API design under real load
   - Consistent behavior and documentation
   - Single authentication model
2. **Poll-Based Activity Claiming**: Workers poll for activities (no push notifications)
   - Simple implementation (no WebSocket complexity)
   - Works with firewalls/NAT (outbound HTTP only)
   - Scalable (workers poll independently)
3. **Heartbeat Mechanism**: Long-running activities send periodic heartbeats
   - Prevents timeout for valid long-running work
   - Detects worker crashes (missed heartbeats)
   - Configurable heartbeat interval
4. **Idempotent Completion**: Activities can only complete/fail once
   - Safe retry on network failure
   - 409 Conflict if already completed
   - Clear error messages
5. **Event Publishing**: Activity results published to orchestrator
   - Orchestrator re-evaluates workflow
   - Schedules next activities
   - Async execution continues

---

## Architecture Reference

Per `docs/architecture.md` (Activity Worker):
- Workers authenticate with API to obtain JWT access token
- Poll activity queue via HTTP endpoints
- Execute activity implementations
- Report progress via heartbeats (long-running activities)
- Post completion status to API
- Built-in worker uses same API as external workers

Per `docs/architecture.md` (Activity Queue Interface):
```rust
#[async_trait]
pub trait ActivityQueue: Send + Sync {
    async fn schedule(&self, workflow_id: Uuid, activities: Vec<Activity>) -> Result<()>;
    async fn claim_next(&self, namespace: &str, name: &str) -> Result<Option<QueuedActivity>>;
    async fn complete(&self, activity_id: Uuid, result: ActivityResult) -> Result<()>;
    async fn heartbeat(&self, activity_id: Uuid) -> Result<()>;
}
```

**Database Schema**:
```sql
-- Activity Queue (from US-1.1)
CREATE TABLE activity_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    parameters JSONB NOT NULL,
    settings JSONB,
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_for TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claimed_by UUID,  -- Worker instance ID
    claimed_at TIMESTAMPTZ,
    last_heartbeat TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(workflow_id, activity_key)
);

CREATE INDEX idx_queue_pending
ON activity_queue(namespace, name, scheduled_for)
WHERE status = 'pending' AND scheduled_for <= NOW();
```

**Worker Polling Flow** (from architecture.md):
```
1. Worker obtains access token via POST /api/v1/auth/token
2. Worker polls via POST /api/v1/workers/poll (with Bearer token)
3. API executes: SELECT ... FOR UPDATE SKIP LOCKED
4. Worker receives activity details
5. Worker executes activity
6. Worker sends heartbeats periodically (if long-running)
7. Worker posts completion via POST /api/v1/activities/{id}/complete
8. API publishes ActivityCompleted event
9. Orchestrator picks up event and continues workflow
```

---

## Implementation Components

### Component 1: Worker Activity Request/Response Types

**Location**: `api/src/handlers/workers.rs` (new file)

**Responsibilities**:
1. Define request/response types for worker activity operations
2. Support polling, heartbeat, complete, fail operations
3. Provide OpenAPI documentation

**Implementation Notes**:
Following US-1A.5/US-1A.6 patterns:
- Request/response types defined in handlers module
- Use `ToSchema` for OpenAPI generation
- Validation via `ValidationErrors` pattern
- Return types use `ApiResult<Json<T>>` pattern

**Implementation**:

```rust
use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use axum::{Extension, Json, extract::Path};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Poll for activities request
///
/// Workers poll for pending activities by specifying which activity types
/// they can execute. The API returns activities matching those types.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PollActivitiesRequest {
    /// Activity types this worker can execute (format: "namespace.name")
    #[schema(example = json!(["payments.authorize", "payments.capture"]))]
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

        // Validate activity type format (namespace.name)
        for activity_type in &self.activity_types {
            if !activity_type.contains('.') {
                errors.add(
                    "activity_types",
                    &format!("Invalid format '{}': must be 'namespace.name'", activity_type),
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

    /// Activity namespace
    #[schema(example = "payments")]
    pub namespace: String,

    /// Activity name
    #[schema(example = "authorize")]
    pub name: String,

    /// Activity input parameters
    #[schema(example = json!({"card_token": "tok_123", "amount": 100.00}))]
    pub parameters: serde_json::Value,

    /// Activity settings (timeout, retry, etc.)
    #[schema(example = json!({"timeout": 300, "max_retries": 3}))]
    pub settings: Option<serde_json::Value>,

    /// Timeout in seconds (extracted from settings for convenience)
    #[schema(example = 300)]
    pub timeout_seconds: Option<i64>,
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
    pub cost_usd: Option<f64>,
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
            if cost < 0.0 {
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
```

**Key Features**:
- Clear request/response structure for all operations
- Activity type filtering in poll request
- Worker ID tracking for claiming activities
- Heartbeat mechanism for long-running activities
- Structured error reporting for failures
- Validation with field-level errors
- OpenAPI documentation

---

### Component 2: Activity Worker Service Layer

**Location**: `core/src/activity/worker_service.rs` (new file)

**Responsibilities**:
1. Poll for pending activities (claim with FOR UPDATE SKIP LOCKED)
2. Send heartbeats for active activities
3. Complete activities with results
4. Fail activities with error details
5. Publish events to orchestrator
6. Handle idempotency and conflict detection

**Implementation Notes**:
Following US-1A.5 service pattern:
- Service struct holds PgPool (cloneable)
- Service implements Clone for AppState
- Methods return `Result<T, ActivityWorkerError>`
- Error type uses thiserror
- Events published directly via sqlx (no separate trait)

**Implementation**:

```rust
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Activity worker service error
#[derive(Debug, Error)]
pub enum ActivityWorkerError {
    #[error("Activity not found: {0}")]
    ActivityNotFound(Uuid),

    #[error("Activity already completed or failed")]
    ActivityAlreadyCompleted,

    #[error("Activity claimed by different worker: expected {expected}, got {actual}")]
    WrongWorker { expected: String, actual: String },

    #[error("Activity not claimed by this worker")]
    ActivityNotClaimed,

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type ActivityWorkerResult<T> = Result<T, ActivityWorkerError>;

/// Pending activity for worker execution
#[derive(Debug, Clone)]
pub struct PendingActivityRecord {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub namespace: String,
    pub name: String,
    pub parameters: Value,
    pub settings: Option<Value>,
}

/// Activity worker service
///
/// Provides worker operations: poll, heartbeat, complete, fail.
/// Follows the repository pattern - holds PgPool and is cloneable.
#[derive(Clone)]
pub struct ActivityWorkerService {
    pool: PgPool,
}

impl ActivityWorkerService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Poll for pending activities
    ///
    /// Claims activities matching the specified types using FOR UPDATE SKIP LOCKED.
    /// Returns up to max_activities that are ready to execute.
    pub async fn poll_activities(
        &self,
        activity_types: Vec<(String, String)>,  // Vec of (namespace, name)
        worker_id: String,
        max_activities: usize,
    ) -> ActivityWorkerResult<Vec<PendingActivityRecord>> {
        let mut tx = self.pool.begin().await?;
        let mut claimed = Vec::new();

        // Poll for each activity type (separate queries for simplicity)
        for (namespace, name) in activity_types {
            if claimed.len() >= max_activities {
                break;
            }

            let remaining = max_activities - claimed.len();

            // Claim activities with FOR UPDATE SKIP LOCKED
            let rows = sqlx::query!(
                r#"
                UPDATE activity_queue
                SET status = 'running',
                    claimed_by = $3,
                    claimed_at = NOW(),
                    last_heartbeat = NOW()
                WHERE id IN (
                    SELECT id FROM activity_queue
                    WHERE status = 'pending'
                      AND scheduled_for <= NOW()
                      AND namespace = $1
                      AND name = $2
                    ORDER BY scheduled_for ASC
                    LIMIT $4
                    FOR UPDATE SKIP LOCKED
                )
                RETURNING id, workflow_id, activity_key, namespace, name, parameters, settings
                "#,
                namespace,
                name,
                worker_id,
                remaining as i64
            )
            .fetch_all(&mut *tx)
            .await?;

            for row in rows {
                claimed.push(PendingActivityRecord {
                    id: row.id,
                    workflow_id: row.workflow_id,
                    activity_key: row.activity_key,
                    namespace: row.namespace,
                    name: row.name,
                    parameters: row.parameters,
                    settings: row.settings,
                });
            }
        }

        tx.commit().await?;

        tracing::info!(
            worker_id = %worker_id,
            claimed_count = claimed.len(),
            "Activities claimed by worker"
        );

        Ok(claimed)
    }

    /// Send heartbeat for an activity
    ///
    /// Updates last_heartbeat timestamp to prevent timeout.
    /// Returns recommended heartbeat interval.
    pub async fn heartbeat_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
    ) -> ActivityWorkerResult<i64> {
        // Verify activity is claimed by this worker and still running
        let result = sqlx::query!(
            r#"
            UPDATE activity_queue
            SET last_heartbeat = NOW()
            WHERE id = $1
              AND claimed_by = $2
              AND status = 'running'
            RETURNING id
            "#,
            activity_id,
            worker_id
        )
        .fetch_optional(&self.pool)
        .await?;

        if result.is_none() {
            // Activity not found, already completed, or claimed by different worker
            let activity = sqlx::query!(
                r#"
                SELECT id, status AS "status: String", claimed_by
                FROM activity_queue
                WHERE id = $1
                "#,
                activity_id
            )
            .fetch_optional(&self.pool)
            .await?;

            return match activity {
                None => Err(ActivityWorkerError::ActivityNotFound(activity_id)),
                Some(row) if row.status != "running" => {
                    Err(ActivityWorkerError::ActivityAlreadyCompleted)
                }
                Some(row) => Err(ActivityWorkerError::WrongWorker {
                    expected: row.claimed_by.unwrap_or_default(),
                    actual: worker_id,
                }),
            };
        }

        tracing::debug!(
            activity_id = %activity_id,
            worker_id = %worker_id,
            "Heartbeat received"
        );

        // Return recommended heartbeat interval (30 seconds)
        Ok(30)
    }

    /// Complete an activity successfully
    ///
    /// Removes activity from queue, updates workflow state, publishes event.
    pub async fn complete_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        output: Value,
        cost_usd: Option<f64>,
    ) -> ActivityWorkerResult<()> {
        let mut tx = self.pool.begin().await?;

        // Get activity details and verify it's claimed by this worker
        let activity = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key, status AS "status: String", claimed_by
            FROM activity_queue
            WHERE id = $1
            FOR UPDATE
            "#,
            activity_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ActivityWorkerError::ActivityNotFound(activity_id))?;

        // Check if already completed
        if activity.status != "running" {
            return Err(ActivityWorkerError::ActivityAlreadyCompleted);
        }

        // Check if claimed by this worker
        if activity.claimed_by.as_deref() != Some(&worker_id) {
            return Err(ActivityWorkerError::WrongWorker {
                expected: activity.claimed_by.unwrap_or_default(),
                actual: worker_id,
            });
        }

        // Remove from queue (activity completed)
        sqlx::query!(
            r#"
            DELETE FROM activity_queue
            WHERE id = $1
            "#,
            activity_id
        )
        .execute(&mut *tx)
        .await?;

        // Publish ActivityCompleted event
        let event_id = Uuid::now_v7();
        let event_payload = serde_json::json!({
            "activity_key": activity.activity_key,
            "output": output,
            "cost_usd": cost_usd
        });

        sqlx::query!(
            r#"
            INSERT INTO workflow_events (id, workflow_id, event_type, activity_key, payload, timestamp)
            VALUES ($1, $2, 'ActivityCompleted', $3, $4, NOW())
            "#,
            event_id,
            activity.workflow_id,
            activity.activity_key,
            event_payload
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        tracing::info!(
            activity_id = %activity_id,
            workflow_id = %activity.workflow_id,
            activity_key = %activity.activity_key,
            worker_id = %worker_id,
            "Activity completed"
        );

        Ok(())
    }

    /// Fail an activity
    ///
    /// Removes activity from queue (or requeues if retryable), publishes event.
    pub async fn fail_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        error_code: String,
        error_message: String,
        retryable: bool,
    ) -> ActivityWorkerResult<bool> {
        let mut tx = self.pool.begin().await?;

        // Get activity details
        let activity = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key, status AS "status: String",
                   claimed_by, settings
            FROM activity_queue
            WHERE id = $1
            FOR UPDATE
            "#,
            activity_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ActivityWorkerError::ActivityNotFound(activity_id))?;

        // Check if already completed
        if activity.status != "running" {
            return Err(ActivityWorkerError::ActivityAlreadyCompleted);
        }

        // Check if claimed by this worker
        if activity.claimed_by.as_deref() != Some(&worker_id) {
            return Err(ActivityWorkerError::WrongWorker {
                expected: activity.claimed_by.unwrap_or_default(),
                actual: worker_id,
            });
        }

        // Determine if retry should happen
        let will_retry = if retryable {
            // Check retry settings (for MVP, simple retry logic)
            // Post-MVP: Extract max_retries from settings, track attempt count
            false  // For MVP, don't retry
        } else {
            false
        };

        if will_retry {
            // Requeue for retry (reset to pending)
            sqlx::query!(
                r#"
                UPDATE activity_queue
                SET status = 'pending',
                    claimed_by = NULL,
                    claimed_at = NULL,
                    last_heartbeat = NULL,
                    scheduled_for = NOW() + INTERVAL '5 seconds'
                WHERE id = $1
                "#,
                activity_id
            )
            .execute(&mut *tx)
            .await?;
        } else {
            // Remove from queue (permanent failure)
            sqlx::query!(
                r#"
                DELETE FROM activity_queue
                WHERE id = $1
                "#,
                activity_id
            )
            .execute(&mut *tx)
            .await?;
        }

        // Publish ActivityFailed event
        let event_id = Uuid::now_v7();
        let event_payload = serde_json::json!({
            "activity_key": activity.activity_key,
            "error_code": error_code,
            "error_message": error_message,
            "retryable": retryable,
            "will_retry": will_retry
        });

        sqlx::query!(
            r#"
            INSERT INTO workflow_events (id, workflow_id, event_type, activity_key, payload, timestamp)
            VALUES ($1, $2, 'ActivityFailed', $3, $4, NOW())
            "#,
            event_id,
            activity.workflow_id,
            activity.activity_key,
            event_payload
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        tracing::info!(
            activity_id = %activity_id,
            workflow_id = %activity.workflow_id,
            activity_key = %activity.activity_key,
            worker_id = %worker_id,
            error_code = %error_code,
            will_retry = will_retry,
            "Activity failed"
        );

        Ok(will_retry)
    }
}
```

**Key Features**:
- Poll with FOR UPDATE SKIP LOCKED (safe concurrent claiming)
- Heartbeat updates last_heartbeat timestamp
- Complete removes from queue and publishes event
- Fail handles retry logic and publishes event
- Worker ID validation (ensures correct worker)
- Idempotency checks (already completed)
- Event publishing for orchestrator

---

### Component 3: API Handlers

**Location**: `api/src/handlers/workers.rs` (continued)

**Responsibilities**:
1. Handle POST /api/v1/workers/poll
2. Handle POST /api/v1/activities/{activity_id}/heartbeat
3. Handle POST /api/v1/activities/{activity_id}/complete
4. Handle POST /api/v1/activities/{activity_id}/fail
5. Map errors to HTTP status codes
6. Return responses

**Implementation**:

```rust
use crate::state::AppState;

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
    State(state): State<AppState>,
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

    // Parse activity types (namespace.name → (namespace, name))
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
    let activities = state
        .activity_worker_service
        .poll_activities(activity_types, request.worker_id.clone(), request.max_activities)
        .await
        .map_err(|e| match e {
            ActivityWorkerError::DatabaseError(e) => {
                tracing::error!("Database error polling activities: {:?}", e);
                AppError::DatabaseError(e)
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    let count = activities.len();

    tracing::info!(
        worker_id = %request.worker_id,
        claimed_count = count,
        "Activities claimed"
    );

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
                    namespace: a.namespace,
                    name: a.name,
                    parameters: a.parameters,
                    settings: a.settings,
                    timeout_seconds,
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
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
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

    let next_heartbeat_seconds = state
        .activity_worker_service
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
            ActivityWorkerError::WrongWorker { expected, actual } => {
                tracing::warn!(
                    "Wrong worker for activity {}: expected {}, got {}",
                    activity_id,
                    expected,
                    actual
                );
                AppError::Conflict(format!(
                    "Activity claimed by different worker: {}",
                    expected
                ))
            }
            ActivityWorkerError::DatabaseError(e) => {
                tracing::error!("Database error sending heartbeat: {:?}", e);
                AppError::DatabaseError(e)
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
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<CompleteActivityRequest>,
) -> ApiResult<Json<CompleteActivityResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::info!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        "Completing activity"
    );

    state
        .activity_worker_service
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
            ActivityWorkerError::WrongWorker { expected, actual } => {
                tracing::warn!(
                    "Wrong worker for activity {}: expected {}, got {}",
                    activity_id,
                    expected,
                    actual
                );
                AppError::Conflict(format!(
                    "Activity claimed by different worker: {}",
                    expected
                ))
            }
            ActivityWorkerError::DatabaseError(e) => {
                tracing::error!("Database error completing activity: {:?}", e);
                AppError::DatabaseError(e)
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(CompleteActivityResponse {
        acknowledged: true,
    }))
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
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
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

    let will_retry = state
        .activity_worker_service
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
            ActivityWorkerError::WrongWorker { expected, actual } => {
                tracing::warn!(
                    "Wrong worker for activity {}: expected {}, got {}",
                    activity_id,
                    expected,
                    actual
                );
                AppError::Conflict(format!(
                    "Activity claimed by different worker: {}",
                    expected
                ))
            }
            ActivityWorkerError::DatabaseError(e) => {
                tracing::error!("Database error failing activity: {:?}", e);
                AppError::DatabaseError(e)
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(FailActivityResponse {
        acknowledged: true,
        will_retry,
    }))
}
```

**Key Features**:
- Four worker endpoints (poll, heartbeat, complete, fail)
- Clear OpenAPI documentation
- Error mapping to HTTP status codes
- Structured logging
- Authentication via ValidatedClaims
- Activity type parsing (namespace.name)

---

### Component 4: Update Application State

**Location**: Update `api/src/state.rs`

**Implementation**:

```rust
use core::workflow::repository::WorkflowDefinitionRepository;
use core::workflow::service::WorkflowService;
use core::workflow::query_service::WorkflowQueryService;
use core::activity::worker_service::ActivityWorkerService;
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
    pub workflow_query_service: WorkflowQueryService,
    pub activity_worker_service: ActivityWorkerService,
}

impl AppState {
    /// Create new application state
    pub async fn new(db_pool: PgPool, auth_config: AuthConfig) -> Self {
        let auth_service = PostgresAuthService::new(db_pool.clone(), auth_config);
        let workflow_definition_repo = WorkflowDefinitionRepository::new(db_pool.clone());
        let workflow_service = WorkflowService::new(db_pool.clone());
        let workflow_query_service = WorkflowQueryService::new(db_pool.clone());
        let activity_worker_service = ActivityWorkerService::new(db_pool.clone());

        Self {
            db_pool,
            auth_service: Arc::new(auth_service),
            workflow_definition_repo,
            workflow_service,
            workflow_query_service,
            activity_worker_service,
        }
    }
}
```

---

### Component 5: Update Routes

**Location**: Update `api/src/routes.rs`

**Implementation**:

```rust
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

        // Workflow Submission and Query
        .route(
            "/api/v1/workflows",
            post(handlers::workflows::submit_workflow)
                .get(handlers::workflows::list_workflows),
        )
        .route(
            "/api/v1/workflows/:workflow_id",
            get(handlers::workflows::get_workflow),
        )
        .route(
            "/api/v1/workflows/:workflow_id/activities",
            get(handlers::workflows::get_workflow_activities),
        )

        // Worker Activity APIs
        .route(
            "/api/v1/workers/poll",
            post(handlers::workers::poll_activities),
        )
        .route(
            "/api/v1/activities/:activity_id/heartbeat",
            post(handlers::workers::heartbeat_activity),
        )
        .route(
            "/api/v1/activities/:activity_id/complete",
            post(handlers::workers::complete_activity),
        )
        .route(
            "/api/v1/activities/:activity_id/fail",
            post(handlers::workers::fail_activity),
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

**Implementation**:

```rust
use crate::handlers::workers::{
    PollActivitiesRequest, PollActivitiesResponse, PendingActivity,
    ActivityHeartbeatRequest, ActivityHeartbeatResponse,
    CompleteActivityRequest, CompleteActivityResponse,
    FailActivityRequest, FailActivityResponse, ActivityError,
};

#[derive(OpenApi)]
#[openapi(
    // ... existing config ...
    paths(
        // ... existing paths ...

        // Worker activity APIs
        crate::handlers::workers::poll_activities,
        crate::handlers::workers::heartbeat_activity,
        crate::handlers::workers::complete_activity,
        crate::handlers::workers::fail_activity,
    ),
    components(
        schemas(
            // ... existing schemas ...

            // Worker activity schemas
            PollActivitiesRequest,
            PollActivitiesResponse,
            PendingActivity,
            ActivityHeartbeatRequest,
            ActivityHeartbeatResponse,
            CompleteActivityRequest,
            CompleteActivityResponse,
            FailActivityRequest,
            FailActivityResponse,
            ActivityError,
        )
    ),
    tags(
        // ... existing tags ...
        (name = "Workers", description = "Worker activity polling and execution"),
    ),
)]
pub struct ApiDoc;
```

---

### Component 7: Update Handlers Module

**Location**: Update `api/src/handlers/mod.rs`

**Implementation**:

```rust
pub mod health;
pub mod oauth;
pub mod workflow_definitions;
pub mod workflows;
pub mod workers;  // New module

// Re-export for convenience
pub use workers::{poll_activities, heartbeat_activity, complete_activity, fail_activity};
```

---

## Testing Requirements

### Unit Tests

**File**: `core/src/activity/worker_service_test.rs`

**Test Scenarios**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_poll_activities_success() {
        let pool = test_db_pool().await;
        let service = ActivityWorkerService::new(pool.clone());

        // Create test workflow with pending activities
        let workflow_id = create_test_workflow(&pool).await;
        schedule_test_activity(&pool, workflow_id, "payments", "authorize").await;

        // Poll for activities
        let activity_types = vec![("payments".to_string(), "authorize".to_string())];
        let result = service
            .poll_activities(activity_types, "worker_01".to_string(), 10)
            .await;

        assert!(result.is_ok());
        let activities = result.unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].namespace, "payments");
    }

    #[tokio::test]
    async fn test_poll_activities_concurrent_workers() {
        let pool = test_db_pool().await;
        let service1 = ActivityWorkerService::new(pool.clone());
        let service2 = ActivityWorkerService::new(pool.clone());

        // Create 10 pending activities
        let workflow_id = create_test_workflow(&pool).await;
        for _ in 0..10 {
            schedule_test_activity(&pool, workflow_id, "payments", "authorize").await;
        }

        // Two workers poll concurrently
        let activity_types = vec![("payments".to_string(), "authorize".to_string())];

        let (result1, result2) = tokio::join!(
            service1.poll_activities(activity_types.clone(), "worker_01".to_string(), 10),
            service2.poll_activities(activity_types.clone(), "worker_02".to_string(), 10)
        );

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        // Each worker should get some activities (no overlap due to SKIP LOCKED)
        let activities1 = result1.unwrap();
        let activities2 = result2.unwrap();
        let total = activities1.len() + activities2.len();
        assert_eq!(total, 10);

        // Verify no duplicate IDs
        let mut ids = activities1.iter().map(|a| a.id).collect::<Vec<_>>();
        ids.extend(activities2.iter().map(|a| a.id));
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 10);
    }

    #[tokio::test]
    async fn test_heartbeat_activity() {
        let pool = test_db_pool().await;
        let service = ActivityWorkerService::new(pool.clone());

        // Claim an activity
        let activity_id = claim_test_activity(&pool, "worker_01").await;

        // Send heartbeat
        let result = service
            .heartbeat_activity(activity_id, "worker_01".to_string())
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 30);
    }

    #[tokio::test]
    async fn test_heartbeat_wrong_worker() {
        let pool = test_db_pool().await;
        let service = ActivityWorkerService::new(pool.clone());

        // Claim an activity with worker_01
        let activity_id = claim_test_activity(&pool, "worker_01").await;

        // Try to send heartbeat from worker_02
        let result = service
            .heartbeat_activity(activity_id, "worker_02".to_string())
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ActivityWorkerError::WrongWorker { .. }
        ));
    }

    #[tokio::test]
    async fn test_complete_activity() {
        let pool = test_db_pool().await;
        let service = ActivityWorkerService::new(pool.clone());

        // Claim an activity
        let (activity_id, workflow_id) = claim_test_activity_with_workflow(&pool, "worker_01").await;

        // Complete the activity
        let output = serde_json::json!({"result": "success"});
        let result = service
            .complete_activity(activity_id, "worker_01".to_string(), output, None)
            .await;

        assert!(result.is_ok());

        // Verify event was published
        let event = get_latest_workflow_event(&pool, workflow_id).await;
        assert_eq!(event.event_type, "ActivityCompleted");
    }

    #[tokio::test]
    async fn test_complete_activity_idempotency() {
        let pool = test_db_pool().await;
        let service = ActivityWorkerService::new(pool.clone());

        // Claim and complete an activity
        let activity_id = claim_test_activity(&pool, "worker_01").await;
        let output = serde_json::json!({"result": "success"});

        service
            .complete_activity(activity_id, "worker_01".to_string(), output.clone(), None)
            .await
            .unwrap();

        // Try to complete again
        let result = service
            .complete_activity(activity_id, "worker_01".to_string(), output, None)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ActivityWorkerError::ActivityAlreadyCompleted
        ));
    }

    #[tokio::test]
    async fn test_fail_activity() {
        let pool = test_db_pool().await;
        let service = ActivityWorkerService::new(pool.clone());

        // Claim an activity
        let (activity_id, workflow_id) = claim_test_activity_with_workflow(&pool, "worker_01").await;

        // Fail the activity
        let result = service
            .fail_activity(
                activity_id,
                "worker_01".to_string(),
                "PAYMENT_DECLINED".to_string(),
                "Card was declined".to_string(),
                false,
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);  // will_retry = false

        // Verify event was published
        let event = get_latest_workflow_event(&pool, workflow_id).await;
        assert_eq!(event.event_type, "ActivityFailed");
    }
}
```

---

### Integration Tests

**File**: `api/tests/worker_activity_test.rs`

**Test Scenarios**:

```rust
#[tokio::test]
async fn test_poll_activities_success() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Submit workflow and schedule activities
    let workflow_id = submit_and_schedule_workflow(&app, &token).await;

    // Poll for activities
    let response = app
        .post("/api/v1/workers/poll")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: PollActivitiesResponse = response.json().await;
    assert!(body.count > 0);
    assert!(!body.activities.is_empty());
}

#[tokio::test]
async fn test_poll_activities_empty() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Poll for non-existent activity type
    let response = app
        .post("/api/v1/workers/poll")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "activity_types": ["nonexistent.type"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: PollActivitiesResponse = response.json().await;
    assert_eq!(body.count, 0);
    assert!(body.activities.is_empty());
}

#[tokio::test]
async fn test_heartbeat_activity() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Claim an activity
    let activity_id = poll_and_claim_activity(&app, &token, "worker_test_01").await;

    // Send heartbeat
    let response = app
        .post(&format!("/api/v1/activities/{}/heartbeat", activity_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "worker_id": "worker_test_01"
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: ActivityHeartbeatResponse = response.json().await;
    assert!(body.acknowledged);
    assert_eq!(body.next_heartbeat_seconds, 30);
}

#[tokio::test]
async fn test_complete_activity() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Claim an activity
    let activity_id = poll_and_claim_activity(&app, &token, "worker_test_01").await;

    // Complete the activity
    let response = app
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": {"result": "success"},
            "cost_usd": 0.015
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: CompleteActivityResponse = response.json().await;
    assert!(body.acknowledged);
}

#[tokio::test]
async fn test_fail_activity() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Claim an activity
    let activity_id = poll_and_claim_activity(&app, &token, "worker_test_01").await;

    // Fail the activity
    let response = app
        .post(&format!("/api/v1/activities/{}/fail", activity_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "worker_id": "worker_test_01",
            "error": {
                "code": "PAYMENT_DECLINED",
                "message": "Card was declined by the bank",
                "retryable": false
            }
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: FailActivityResponse = response.json().await;
    assert!(body.acknowledged);
    assert!(!body.will_retry);
}

#[tokio::test]
async fn test_worker_authentication_required() {
    let app = test_app().await;

    // Try to poll without authentication
    let response = app
        .post("/api/v1/workers/poll")
        .json(&json!({
            "activity_types": ["payments.authorize"],
            "worker_id": "worker_test_01",
            "max_activities": 10
        }))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_complete_activity_idempotency() {
    let app = test_app().await;
    let token = create_test_token(&app).await;

    // Claim and complete an activity
    let activity_id = poll_and_claim_activity(&app, &token, "worker_test_01").await;

    // First completion
    let response1 = app
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": {"result": "success"}
        }))
        .await;

    assert_eq!(response1.status(), StatusCode::OK);

    // Second completion (should fail with conflict)
    let response2 = app
        .post(&format!("/api/v1/activities/{}/complete", activity_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "worker_id": "worker_test_01",
            "output": {"result": "success"}
        }))
        .await;

    assert_eq!(response2.status(), StatusCode::CONFLICT);
}
```

---

## Dependencies

### Existing Dependencies

All required dependencies should already be available:
- `sqlx` - Database access with FOR UPDATE SKIP LOCKED
- `serde` / `serde_json` - Serialization
- `uuid` - UUID handling
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

### Activity Timeout Configuration

For MVP, timeout handling is simple (activities not heartbeat within timeout are released).
Post-MVP: Add configuration for:
- `STREAMFLOW_ACTIVITY_DEFAULT_TIMEOUT` - Default timeout in seconds
- `STREAMFLOW_ACTIVITY_HEARTBEAT_INTERVAL` - Recommended heartbeat interval

---

## Documentation Updates

### API Documentation

Update `docs/api-reference.md`:

```markdown
## Worker Activity APIs

Workers poll for activities, execute them, and report results.

### Poll for Activities

**Endpoint**: `POST /api/v1/workers/poll`

**Authentication**: Required (Bearer token)

**Request Body**:
\`\`\`json
{
  "activity_types": ["payments.authorize", "payments.capture"],
  "worker_id": "worker_payments_01",
  "max_activities": 10
}
\`\`\`

**Response** (200 OK):
\`\`\`json
{
  "activities": [
    {
      "activity_id": "550e8400-e29b-41d4-a716-446655440000",
      "workflow_id": "660e8400-e29b-41d4-a716-446655440001",
      "activity_key": "authorize_card",
      "namespace": "payments",
      "name": "authorize",
      "parameters": {"card_token": "tok_123", "amount": 100.00},
      "settings": {"timeout": 300},
      "timeout_seconds": 300
    }
  ],
  "count": 1
}
\`\`\`

### Send Heartbeat

**Endpoint**: `POST /api/v1/activities/{activity_id}/heartbeat`

**Request Body**:
\`\`\`json
{
  "worker_id": "worker_payments_01"
}
\`\`\`

**Response** (200 OK):
\`\`\`json
{
  "acknowledged": true,
  "next_heartbeat_seconds": 30
}
\`\`\`

### Complete Activity

**Endpoint**: `POST /api/v1/activities/{activity_id}/complete`

**Request Body**:
\`\`\`json
{
  "worker_id": "worker_payments_01",
  "output": {"authorization_id": "auth_123", "approved": true},
  "cost_usd": 0.015
}
\`\`\`

**Response** (200 OK):
\`\`\`json
{
  "acknowledged": true
}
\`\`\`

### Fail Activity

**Endpoint**: `POST /api/v1/activities/{activity_id}/fail`

**Request Body**:
\`\`\`json
{
  "worker_id": "worker_payments_01",
  "error": {
    "code": "PAYMENT_DECLINED",
    "message": "Card was declined by the bank",
    "retryable": false
  }
}
\`\`\`

**Response** (200 OK):
\`\`\`json
{
  "acknowledged": true,
  "will_retry": false
}
\`\`\`
```

---

## Success Criteria

### Functional Requirements

- ✅ `POST /api/v1/workers/poll` returns pending activities
- ✅ Activities claimed with FOR UPDATE SKIP LOCKED (no conflicts)
- ✅ `POST /api/v1/activities/{id}/heartbeat` updates heartbeat timestamp
- ✅ `POST /api/v1/activities/{id}/complete` completes activity and publishes event
- ✅ `POST /api/v1/activities/{id}/fail` fails activity and publishes event
- ✅ Worker authentication required (Bearer token)
- ✅ Idempotency (409 Conflict if already completed)
- ✅ Worker ID validation (correct worker owns activity)

### Non-Functional Requirements

- ✅ FOR UPDATE SKIP LOCKED prevents race conditions
- ✅ Heartbeat mechanism for long-running activities
- ✅ Event publishing for orchestrator integration
- ✅ Clear error messages with HTTP status codes
- ✅ OpenAPI documentation for all endpoints
- ✅ Structured logging for debugging

---

## Implementation Phases

### Phase 1: Service Layer (P0)
- Implement ActivityWorkerService
- Implement poll, heartbeat, complete, fail methods
- Unit tests for service layer
- **Estimated Time**: 4 hours

### Phase 2: API Handlers (P0)
- Implement POST /api/v1/workers/poll
- Implement POST /api/v1/activities/{id}/heartbeat
- Implement POST /api/v1/activities/{id}/complete
- Implement POST /api/v1/activities/{id}/fail
- Error mapping to HTTP status codes
- **Estimated Time**: 3 hours

### Phase 3: Application State and Routes (P0)
- Add ActivityWorkerService to AppState
- Add routes for worker endpoints
- Update OpenAPI documentation
- **Estimated Time**: 1 hour

### Phase 4: Integration Tests (P0)
- Test poll (success, empty, concurrent)
- Test heartbeat (success, wrong worker)
- Test complete (success, idempotency)
- Test fail (success)
- Test authentication required
- **Estimated Time**: 3 hours

### Phase 5: End-to-End Testing (P0)
- Test full workflow: submit → poll → execute → complete → orchestrator continues
- Test with built-in worker
- Test with external worker client
- Manual testing with curl/Postman
- Update documentation
- **Estimated Time**: 3 hours

**Total Estimated Time**: 14 hours

---

## Risks and Mitigations

### Risk 1: Concurrent Worker Race Conditions

**Probability**: Low
**Impact**: High (duplicate activity execution)

**Mitigation**:
- FOR UPDATE SKIP LOCKED ensures atomic claiming
- Integration tests verify concurrent polling
- Activity queue UNIQUE constraint prevents duplicates
- Monitoring for duplicate executions

### Risk 2: Heartbeat Timeout Detection

**Probability**: Medium
**Impact**: Medium (activities timeout when they shouldn't)

**Mitigation**:
- Clear heartbeat interval documentation (30 seconds)
- Workers responsible for sending heartbeats
- Timeout detection via background job (Post-MVP)
- Activity retry on timeout

### Risk 3: Event Publishing Failure

**Probability**: Low
**Impact**: High (orchestrator doesn't continue workflow)

**Mitigation**:
- Activity completion and event publishing in same transaction
- Transaction rollback if event publishing fails
- Worker can safely retry completion
- Idempotency prevents duplicate events

### Risk 4: Worker Crash Before Completion

**Probability**: Medium
**Impact**: Medium (activity remains claimed, workflow hangs)

**Mitigation**:
- Heartbeat timeout detection (Post-MVP background job)
- Activity released for retry if no heartbeat
- Worker crash recovery mechanism
- Monitoring for stuck activities

---

## Future Enhancements (Post-MVP)

### Timeout Detection Background Job
- Periodically check for activities without recent heartbeat
- Release timed-out activities for retry
- Configurable timeout per activity type
- Metrics for timeout frequency

### Advanced Retry Logic
- Extract max_retries from activity settings
- Track attempt count per activity
- Exponential backoff for retries
- Dead-letter queue for permanent failures

### Worker Health Monitoring
- Worker registration and heartbeat
- Detect crashed workers
- Reassign activities from crashed workers
- Worker pool visualization

### Activity Priority
- Priority queue for urgent activities
- Workers poll high-priority first
- Configurable priority per activity
- Fair scheduling across workflows

### Batch Activity Claiming
- Claim multiple activities in single poll
- Reduce polling overhead
- Worker-side batch execution
- Configurable batch size

---

## Related User Stories

- **US-1.1**: Activity Queue with Ordering Guarantees (provides activity queue)
- **US-1.2**: Event-Driven Dynamic Scheduling (schedules activities to queue)
- **US-1A.5**: Workflow Submission API (creates workflows)
- **US-1A.6**: Workflow Status and Query API (monitors workflow progress)
- **US-1B.1**: Worker Polling with Concurrency Safety (built-in worker uses these APIs)

---

## References

- Architecture: `docs/architecture.md` (Activity Worker, ActivityQueue Interface)
- Requirements: `docs/mvp-requirements.md` (Epic 1A, US-1A.7)
- US-1.1 Implementation: `docs/implementation/US-1.1-activity-queue.md`
- US-1.2 Implementation: `docs/implementation/US-1.2-event-driven-scheduling.md`

---

## Implementation Notes

**Key Design Decisions**:

1. **Unified Worker API**: Built-in and external workers use same endpoints
   - Validates API design under real load (built-in worker)
   - Consistent documentation and behavior
   - Single authentication model

2. **FOR UPDATE SKIP LOCKED**: Safe concurrent activity claiming
   - PostgreSQL handles locking and conflicts
   - No application-level coordination needed
   - Scalable to many workers

3. **Poll-Based Model**: Workers poll (no push notifications)
   - Simple implementation
   - Firewall-friendly (outbound HTTP only)
   - Works in all network environments

4. **Heartbeat Mechanism**: Long-running activities send heartbeats
   - Prevents timeout for valid work
   - Detects worker crashes
   - Configurable interval (30 seconds default)

5. **Idempotent Completion**: Activities complete/fail once
   - Safe retry on network failure
   - 409 Conflict if already done
   - Transaction ensures atomicity

6. **Event Publishing**: Results published to orchestrator
   - Workflow continues automatically
   - Same transaction as completion
   - Orchestrator picks up via polling

**Implementation Order**:
1. Service layer (poll, heartbeat, complete, fail)
2. API handlers with error mapping
3. Application state and routes
4. Integration tests
5. End-to-end testing with orchestrator

**Post-Implementation**:
- US-1B.1 will implement built-in worker using these APIs
- US-1A.8 will enable output retrieval (complements activity completion)
- US-1A.9 will provide real-time updates via WebSocket
- Post-MVP: Timeout detection background job

---

## Definition of Done

- [ ] ActivityWorkerService implemented (poll, heartbeat, complete, fail)
- [ ] POST /api/v1/workers/poll handler implemented
- [ ] POST /api/v1/activities/{id}/heartbeat handler implemented
- [ ] POST /api/v1/activities/{id}/complete handler implemented
- [ ] POST /api/v1/activities/{id}/fail handler implemented
- [ ] Request/response validation implemented
- [ ] Error mapping to HTTP status codes complete
- [ ] Application state includes ActivityWorkerService
- [ ] Routes include worker endpoints
- [ ] OpenAPI documentation updated
- [ ] Unit tests passing (service layer)
- [ ] Integration tests passing (API endpoints)
- [ ] End-to-end tests passing (poll → complete → orchestrator)
- [ ] All acceptance criteria met
- [ ] Zero cargo warnings
- [ ] Documentation updated

---

**Last Updated**: 2025-11-06
**Next Review**: After implementation complete
