//! Recurring workflow schedules.
//!
//! A schedule is an operational resource that submits a workflow on a cadence
//! (standard crontab or a fixed interval). The scheduler runs server-side and
//! submits through [`WorkflowService`] directly — no client credentials ride
//! the recurrence, so nothing stales mid-chain and schedule death is
//! impossible short of engine death.
//!
//! Semantics:
//! - **Fire-once misfire policy**: `next_run_at` always advances to the next
//!   occurrence strictly in the future, so scheduler downtime collapses any
//!   backlog into at most one catch-up run.
//! - **Idempotent submission**: each run is submitted with
//!   `unique_key = schedule:<name>:<epoch of the occurrence>`, so multiple
//!   scheduler instances (or a crash between submit and advance) cannot
//!   duplicate a run — the loser's 409 is treated as success.
//! - **Overlap policy**: `skip` (default) suppresses submission while the
//!   previous run is still non-terminal; `allow` always submits. `next_run_at`
//!   advances either way.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::str::FromStr;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::events::models::WorkflowStatus;
use crate::workflow::service::{WorkflowService, WorkflowServiceError};

/// Schedule error
#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("Schedule not found: {0}")]
    NotFound(Uuid),

    #[error("Schedule name already exists: '{0}'")]
    DuplicateName(String),

    #[error("Workflow definition not found: {0}")]
    DefinitionNotFound(String),

    #[error("Invalid schedule: {0}")]
    Invalid(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, ScheduleError>;

/// What to do when a schedule fires while the previous run is still going
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlapPolicy {
    /// Don't submit while the previous run is non-terminal (default)
    Skip,
    /// Always submit
    Allow,
}

impl OverlapPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            OverlapPolicy::Skip => "skip",
            OverlapPolicy::Allow => "allow",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "skip" => Ok(OverlapPolicy::Skip),
            "allow" => Ok(OverlapPolicy::Allow),
            other => Err(ScheduleError::Invalid(format!(
                "Unknown overlap_policy '{}'",
                other
            ))),
        }
    }
}

/// A recurring workflow schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSchedule {
    pub id: Uuid,
    pub name: String,
    pub definition_name: String,
    /// None = resolve the latest definition version at fire time
    pub definition_version: Option<String>,
    pub input: Value,
    /// Standard 5-field crontab (minute granularity) or 6-field with leading
    /// seconds; mutually exclusive with `interval_seconds`
    pub cron: Option<String>,
    /// IANA timezone for cron evaluation (default UTC); cron only
    pub timezone: Option<String>,
    /// Fixed interval in seconds; mutually exclusive with `cron`
    pub interval_seconds: Option<i64>,
    pub overlap_policy: OverlapPolicy,
    pub enabled: bool,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_workflow_id: Option<Uuid>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields for creating a schedule
#[derive(Debug, Clone)]
pub struct NewSchedule {
    pub name: String,
    pub definition_name: String,
    pub definition_version: Option<String>,
    pub input: Value,
    pub cron: Option<String>,
    pub timezone: Option<String>,
    pub interval_seconds: Option<i64>,
    pub overlap_policy: OverlapPolicy,
    pub enabled: bool,
    pub created_by: Option<String>,
}

/// Partial update; None = unchanged. Providing `cron` or `interval_seconds`
/// replaces the cadence wholesale (the other side is cleared).
#[derive(Debug, Clone, Default)]
pub struct ScheduleUpdate {
    pub input: Option<Value>,
    pub cron: Option<String>,
    pub timezone: Option<String>,
    pub interval_seconds: Option<i64>,
    pub overlap_policy: Option<OverlapPolicy>,
    pub enabled: Option<bool>,
}

/// Normalize a crontab expression for the `cron` crate, which requires a
/// seconds field: standard 5-field crontab gets "0 " prepended; 6/7-field
/// expressions pass through unchanged.
fn normalize_cron(expression: &str) -> String {
    if expression.split_whitespace().count() == 5 {
        format!("0 {}", expression)
    } else {
        expression.to_string()
    }
}

/// Validate a cadence: exactly one of cron/interval, parseable cron,
/// recognized timezone (cron only), interval >= 1s.
fn validate_cadence(
    cron_expr: Option<&str>,
    timezone: Option<&str>,
    interval_seconds: Option<i64>,
) -> Result<()> {
    match (cron_expr, interval_seconds) {
        (Some(_), Some(_)) => Err(ScheduleError::Invalid(
            "Provide exactly one of cron or interval_seconds, not both".to_string(),
        )),
        (None, None) => Err(ScheduleError::Invalid(
            "Provide exactly one of cron or interval_seconds".to_string(),
        )),
        (Some(expr), None) => {
            cron::Schedule::from_str(&normalize_cron(expr)).map_err(|e| {
                ScheduleError::Invalid(format!("Invalid cron expression '{}': {}", expr, e))
            })?;
            if let Some(tz) = timezone {
                tz.parse::<chrono_tz::Tz>()
                    .map_err(|_| ScheduleError::Invalid(format!("Unknown timezone '{}'", tz)))?;
            }
            Ok(())
        }
        (None, Some(secs)) => {
            if timezone.is_some() {
                return Err(ScheduleError::Invalid(
                    "timezone only applies to cron schedules".to_string(),
                ));
            }
            if secs < 1 {
                return Err(ScheduleError::Invalid(
                    "interval_seconds must be at least 1".to_string(),
                ));
            }
            Ok(())
        }
    }
}

/// Next occurrence strictly after `after`.
///
/// Interval schedules keep their phase: the result is `next_run_at + k *
/// interval` for the smallest k that lands strictly after `after` — long
/// downtime advances arithmetically, never by looping.
fn next_occurrence(
    cron_expr: Option<&str>,
    timezone: Option<&str>,
    interval_seconds: Option<i64>,
    phase_anchor: DateTime<Utc>,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    if let Some(expr) = cron_expr {
        let schedule = cron::Schedule::from_str(&normalize_cron(expr)).map_err(|e| {
            ScheduleError::Invalid(format!("Invalid cron expression '{}': {}", expr, e))
        })?;
        let tz: chrono_tz::Tz = timezone
            .unwrap_or("UTC")
            .parse()
            .map_err(|_| ScheduleError::Invalid(format!("Unknown timezone {:?}", timezone)))?;
        schedule
            .after(&after.with_timezone(&tz))
            .next()
            .map(|dt| dt.with_timezone(&Utc))
            .ok_or_else(|| {
                ScheduleError::Invalid(format!(
                    "Cron expression '{}' yields no future occurrences",
                    expr
                ))
            })
    } else if let Some(secs) = interval_seconds {
        if secs < 1 {
            return Err(ScheduleError::Invalid(
                "interval_seconds must be at least 1".to_string(),
            ));
        }
        if phase_anchor > after {
            return Ok(phase_anchor);
        }
        let elapsed = (after - phase_anchor).num_seconds();
        let k = elapsed / secs + 1;
        Ok(phase_anchor + ChronoDuration::seconds(k * secs))
    } else {
        Err(ScheduleError::Invalid(
            "Schedule has neither cron nor interval_seconds".to_string(),
        ))
    }
}

/// CRUD service for workflow schedules
#[derive(Clone)]
pub struct ScheduleService {
    pool: PgPool,
}

impl ScheduleService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewSchedule) -> Result<WorkflowSchedule> {
        if new.name.trim().is_empty() {
            return Err(ScheduleError::Invalid(
                "Schedule name cannot be empty".to_string(),
            ));
        }
        validate_cadence(
            new.cron.as_deref(),
            new.timezone.as_deref(),
            new.interval_seconds,
        )?;

        // The referenced definition must exist (version resolution at fire
        // time still uses the latest when unpinned)
        let repo = crate::workflow::WorkflowDefinitionRepository::new(self.pool.clone());
        let found = match new.definition_version.as_deref() {
            Some(v) => repo
                .get(&new.definition_name, v)
                .await
                .map_err(|e| ScheduleError::Invalid(e.to_string()))?
                .is_some(),
            None => repo
                .get_latest(&new.definition_name)
                .await
                .map_err(|e| ScheduleError::Invalid(e.to_string()))?
                .is_some(),
        };
        if !found {
            return Err(ScheduleError::DefinitionNotFound(format!(
                "{} (version {})",
                new.definition_name,
                new.definition_version.as_deref().unwrap_or("latest")
            )));
        }

        let now = Utc::now();
        // First interval occurrence is one period after creation; cron fires
        // at its next matching instant.
        let next_run_at = next_occurrence(
            new.cron.as_deref(),
            new.timezone.as_deref(),
            new.interval_seconds,
            now,
            now,
        )?;

        let row = sqlx::query!(
            r#"
            INSERT INTO workflow_schedules (
                name, definition_name, definition_version, input,
                cron, timezone, interval_seconds, overlap_policy,
                enabled, next_run_at, created_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING id, created_at, updated_at
            "#,
            new.name,
            new.definition_name,
            new.definition_version,
            new.input,
            new.cron,
            new.timezone,
            new.interval_seconds,
            new.overlap_policy.as_str(),
            new.enabled,
            next_run_at,
            new.created_by
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e
                && db_err.constraint() == Some("workflow_schedules_name_key")
            {
                return ScheduleError::DuplicateName(new.name.clone());
            }
            ScheduleError::Database(e)
        })?;

        Ok(WorkflowSchedule {
            id: row.id,
            name: new.name,
            definition_name: new.definition_name,
            definition_version: new.definition_version,
            input: new.input,
            cron: new.cron,
            timezone: new.timezone,
            interval_seconds: new.interval_seconds,
            overlap_policy: new.overlap_policy,
            enabled: new.enabled,
            next_run_at,
            last_run_at: None,
            last_workflow_id: None,
            created_by: new.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub async fn get(&self, id: Uuid) -> Result<WorkflowSchedule> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, definition_name, definition_version, input,
                   cron, timezone, interval_seconds, overlap_policy,
                   enabled, next_run_at, last_run_at, last_workflow_id,
                   created_by, created_at, updated_at
            FROM workflow_schedules
            WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ScheduleError::NotFound(id))?;

        Ok(WorkflowSchedule {
            id: row.id,
            name: row.name,
            definition_name: row.definition_name,
            definition_version: row.definition_version,
            input: row.input,
            cron: row.cron,
            timezone: row.timezone,
            interval_seconds: row.interval_seconds,
            overlap_policy: OverlapPolicy::parse(&row.overlap_policy)?,
            enabled: row.enabled,
            next_run_at: row.next_run_at,
            last_run_at: row.last_run_at,
            last_workflow_id: row.last_workflow_id,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub async fn list(&self) -> Result<Vec<WorkflowSchedule>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name, definition_name, definition_version, input,
                   cron, timezone, interval_seconds, overlap_policy,
                   enabled, next_run_at, last_run_at, last_workflow_id,
                   created_by, created_at, updated_at
            FROM workflow_schedules
            ORDER BY name
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(WorkflowSchedule {
                    id: row.id,
                    name: row.name,
                    definition_name: row.definition_name,
                    definition_version: row.definition_version,
                    input: row.input,
                    cron: row.cron,
                    timezone: row.timezone,
                    interval_seconds: row.interval_seconds,
                    overlap_policy: OverlapPolicy::parse(&row.overlap_policy)?,
                    enabled: row.enabled,
                    next_run_at: row.next_run_at,
                    last_run_at: row.last_run_at,
                    last_workflow_id: row.last_workflow_id,
                    created_by: row.created_by,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                })
            })
            .collect()
    }

    pub async fn update(&self, id: Uuid, changes: ScheduleUpdate) -> Result<WorkflowSchedule> {
        let current = self.get(id).await?;

        // Cadence replacement: providing cron or interval_seconds replaces the
        // cadence wholesale; providing only timezone retunes an existing cron.
        let cadence_changed = changes.cron.is_some()
            || changes.interval_seconds.is_some()
            || changes.timezone.is_some();
        let (cron, timezone, interval_seconds) = if changes.cron.is_some() {
            (changes.cron.clone(), changes.timezone.clone(), None)
        } else if changes.interval_seconds.is_some() {
            (None, None, changes.interval_seconds)
        } else if changes.timezone.is_some() {
            (
                current.cron.clone(),
                changes.timezone.clone(),
                current.interval_seconds,
            )
        } else {
            (
                current.cron.clone(),
                current.timezone.clone(),
                current.interval_seconds,
            )
        };
        validate_cadence(cron.as_deref(), timezone.as_deref(), interval_seconds)?;

        let enabled = changes.enabled.unwrap_or(current.enabled);
        let re_enabled = enabled && !current.enabled;

        // Recompute next_run_at when the cadence changes or the schedule is
        // re-enabled (a stale past-due next_run_at must not fire immediately)
        let now = Utc::now();
        let next_run_at = if cadence_changed || re_enabled {
            next_occurrence(
                cron.as_deref(),
                timezone.as_deref(),
                interval_seconds,
                now,
                now,
            )?
        } else {
            current.next_run_at
        };

        let input = changes.input.unwrap_or(current.input);
        let overlap_policy = changes.overlap_policy.unwrap_or(current.overlap_policy);

        sqlx::query!(
            r#"
            UPDATE workflow_schedules
            SET input = $2, cron = $3, timezone = $4, interval_seconds = $5,
                overlap_policy = $6, enabled = $7, next_run_at = $8
            WHERE id = $1
            "#,
            id,
            input,
            cron,
            timezone,
            interval_seconds,
            overlap_policy.as_str(),
            enabled,
            next_run_at
        )
        .execute(&self.pool)
        .await?;

        self.get(id).await
    }

    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let result = sqlx::query!("DELETE FROM workflow_schedules WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(ScheduleError::NotFound(id));
        }
        Ok(())
    }
}

/// Scheduler loop configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Whether the scheduler loop runs at all (default true)
    pub enabled: bool,
    /// Tick interval (default 1s)
    pub tick_interval: std::time::Duration,
    /// Max due schedules claimed per tick (default 100)
    pub batch_limit: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tick_interval: std::time::Duration::from_secs(1),
            batch_limit: 100,
        }
    }
}

impl SchedulerConfig {
    /// Load from environment:
    /// - `KRUXIAFLOW_SCHEDULER_ENABLED` (default: true)
    /// - `KRUXIAFLOW_SCHEDULER_TICK_INTERVAL_MS` (default: 1000)
    /// - `KRUXIAFLOW_SCHEDULER_BATCH_LIMIT` (default: 100)
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            enabled: std::env::var("KRUXIAFLOW_SCHEDULER_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default.enabled),
            tick_interval: std::env::var("KRUXIAFLOW_SCHEDULER_TICK_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(std::time::Duration::from_millis)
                .unwrap_or(default.tick_interval),
            batch_limit: std::env::var("KRUXIAFLOW_SCHEDULER_BATCH_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default.batch_limit),
        }
    }
}

/// Run the scheduler loop until cancelled.
///
/// Safe to run in multiple processes concurrently: due schedules are claimed
/// with FOR UPDATE SKIP LOCKED and submissions are idempotent via unique_key.
pub async fn run_scheduler(
    pool: PgPool,
    config: SchedulerConfig,
    shutdown_token: Option<CancellationToken>,
) {
    if !config.enabled {
        tracing::info!("Scheduler disabled (KRUXIAFLOW_SCHEDULER_ENABLED=false)");
        return;
    }

    let workflow_service = WorkflowService::new(pool.clone());
    let mut interval = tokio::time::interval(config.tick_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tracing::info!(
        tick_interval_ms = config.tick_interval.as_millis() as u64,
        batch_limit = config.batch_limit,
        "Scheduler started"
    );

    loop {
        if let Some(token) = &shutdown_token {
            tokio::select! {
                _ = interval.tick() => {}
                _ = token.cancelled() => {
                    tracing::info!("Scheduler shutdown signal received");
                    break;
                }
            }
        } else {
            interval.tick().await;
        }

        match process_due_schedules(&pool, &workflow_service, config.batch_limit).await {
            Ok(fired) if fired > 0 => {
                tracing::debug!(fired = fired, "Scheduler tick submitted workflows");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(error = %e, "Scheduler tick failed");
            }
        }
    }
}

/// Process one batch of due schedules. Returns the number of workflows
/// submitted.
///
/// Submission failures other than duplicate-key advance `next_run_at` anyway
/// (a broken schedule must not hot-loop every tick) and are logged.
pub async fn process_due_schedules(
    pool: &PgPool,
    workflow_service: &WorkflowService,
    batch_limit: i64,
) -> Result<u32> {
    let mut tx = pool.begin().await?;

    let rows = sqlx::query!(
        r#"
        SELECT id, name, definition_name, definition_version, input,
               cron, timezone, interval_seconds, overlap_policy,
               enabled, next_run_at, last_run_at, last_workflow_id
        FROM workflow_schedules
        WHERE enabled AND next_run_at <= NOW()
        ORDER BY next_run_at
        LIMIT $1
        FOR UPDATE SKIP LOCKED
        "#,
        batch_limit
    )
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        tx.commit().await?;
        return Ok(0);
    }

    let now = Utc::now();
    let mut fired: u32 = 0;

    for row in rows {
        let occurrence = row.next_run_at;

        // Advance first (in-memory): fire-once misfire policy
        let next_run_at = match next_occurrence(
            row.cron.as_deref(),
            row.timezone.as_deref(),
            row.interval_seconds,
            occurrence,
            now,
        ) {
            Ok(next) => next,
            Err(e) => {
                // Validated at create/update; reaching this means external
                // drift (e.g., timezone database). Disable rather than
                // hot-loop.
                tracing::error!(
                    schedule = %row.name,
                    error = %e,
                    "Schedule cadence no longer computable; disabling"
                );
                sqlx::query!(
                    "UPDATE workflow_schedules SET enabled = FALSE WHERE id = $1",
                    row.id
                )
                .execute(&mut *tx)
                .await?;
                continue;
            }
        };

        // Overlap policy: skip while the previous run is non-terminal
        let mut submit = true;
        if OverlapPolicy::parse(&row.overlap_policy)? == OverlapPolicy::Skip
            && let Some(prev_id) = row.last_workflow_id
        {
            let prev_status = sqlx::query_scalar!(
                r#"SELECT status AS "status: WorkflowStatus" FROM workflows WHERE id = $1"#,
                prev_id
            )
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(status) = prev_status
                && matches!(
                    status,
                    WorkflowStatus::Created | WorkflowStatus::Running | WorkflowStatus::Paused
                )
            {
                tracing::debug!(
                    schedule = %row.name,
                    previous_workflow_id = %prev_id,
                    "Skipping occurrence: previous run still active (overlap_policy=skip)"
                );
                submit = false;
            }
        }

        if submit {
            let unique_key = format!("schedule:{}:{}", row.name, occurrence.timestamp());
            match workflow_service
                .submit_workflow(
                    &row.definition_name,
                    row.definition_version.as_deref(),
                    row.input.clone(),
                    Some(unique_key),
                    None,
                )
                .await
            {
                Ok(workflow) => {
                    fired += 1;
                    tracing::info!(
                        schedule = %row.name,
                        workflow_id = %workflow.id,
                        occurrence = %occurrence,
                        "Schedule fired"
                    );
                    sqlx::query!(
                        r#"
                        UPDATE workflow_schedules
                        SET next_run_at = $2, last_run_at = $3, last_workflow_id = $4
                        WHERE id = $1
                        "#,
                        row.id,
                        next_run_at,
                        now,
                        workflow.id
                    )
                    .execute(&mut *tx)
                    .await?;
                    continue;
                }
                Err(WorkflowServiceError::DuplicateSubmission(key)) => {
                    // Another scheduler instance (or a pre-crash tick) already
                    // submitted this occurrence — idempotent success.
                    tracing::debug!(
                        schedule = %row.name,
                        unique_key = %key,
                        "Occurrence already submitted elsewhere"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        schedule = %row.name,
                        error = %e,
                        "Schedule submission failed; advancing to next occurrence"
                    );
                }
            }
        }

        sqlx::query!(
            "UPDATE workflow_schedules SET next_run_at = $2 WHERE id = $1",
            row.id,
            next_run_at
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(fired)
}
