use crate::queue::{
    Activity, ActivityQueue, ActivityResult, ActivitySettings, ActivityStatus, ActivitySummary,
    QueueConfig, QueueError, QueuedActivity, Result, StaleActivityAction, StaleActivityInfo,
};
use async_trait::async_trait;
use sqlx::PgPool;
use tracing::{debug, error, trace, warn};
use uuid::Uuid;

pub struct PostgresQueue {
    pool: PgPool,
    config: QueueConfig,
}

impl PostgresQueue {
    pub fn new(pool: PgPool, config: QueueConfig) -> Self {
        Self { pool, config }
    }

    fn extract_timeout(&self, settings: &Option<ActivitySettings>) -> u64 {
        settings
            .as_ref()
            .and_then(|s| s.timeout_seconds)
            .unwrap_or(self.config.default_timeout.as_secs())
    }

    fn extract_max_retries(&self, settings: &Option<ActivitySettings>) -> i32 {
        settings
            .as_ref()
            .and_then(|s| s.retry.as_ref())
            .map(|rc| rc.max_attempts as i32)
            .unwrap_or(self.config.default_max_retries as i32)
    }
}

#[async_trait]
impl ActivityQueue for PostgresQueue {
    #[tracing::instrument(skip(self, activities), level = "trace", fields(num_activities = activities.len()))]
    async fn schedule(&self, workflow_id: Uuid, activities: Vec<Activity>) -> Result<()> {
        for activity in activities {
            // Validate parameters are valid JSON
            if !activity.parameters.is_object() && !activity.parameters.is_array() {
                return Err(QueueError::InvalidParameters(format!(
                    "Activity parameters must be a JSON object or array for activity {}",
                    activity.key
                )));
            }

            // Validate activity key is not empty
            if activity.key.trim().is_empty() {
                return Err(QueueError::InvalidParameters(
                    "Activity key cannot be empty".to_string(),
                ));
            }

            let timeout = self.extract_timeout(&activity.settings);
            let max_retries = self.extract_max_retries(&activity.settings);

            let settings_json = activity
                .settings
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?;

            let output_definitions_json = activity
                .output_definitions
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?;

            // Idempotent insert - ON CONFLICT DO NOTHING
            // Use COALESCE($7, NOW()) to default scheduled_for to database NOW() if None
            // This ensures consistency with claim_next which uses NOW() for comparison
            let result = sqlx::query!(
                r#"
                INSERT INTO activity_queue (
                    workflow_id, activity_key, worker, name,
                    parameters, settings, scheduled_for, timeout_duration,
                    max_retries, output_definitions, iteration, signal_data
                ) VALUES ($1, $2, $3, $4,
                          $5, $6, COALESCE($7, NOW()), make_interval(secs => $8),
                          $9, $10, $11, $12)
                ON CONFLICT (workflow_id, activity_key, iteration) DO NOTHING
                "#,
                workflow_id,
                activity.key,
                activity.worker,
                activity.activity_name,
                activity.parameters,
                settings_json,
                activity.scheduled_for,
                timeout as f64,
                max_retries,
                output_definitions_json,
                activity.iteration,
                activity.signal_data
            )
            .execute(&self.pool)
            .await?;

            if result.rows_affected() > 0 {
                trace!(
                    workflow_id = %workflow_id,
                    activity_key = %activity.key,
                    "Activity scheduled"
                );
            } else {
                trace!(
                    workflow_id = %workflow_id,
                    activity_key = %activity.key,
                    "Activity already scheduled (idempotent)"
                );
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip(self), level = "trace", fields(worker = %worker, max_activities = max_activities))]
    async fn claim_next(
        &self,
        worker_id: &str,
        worker: &str,
        max_activities: usize,
    ) -> Result<Vec<QueuedActivity>> {
        // Return early if max_activities is 0
        if max_activities == 0 {
            return Ok(vec![]);
        }

        // This query:
        // 1. Finds up to max_activities claimable activities for the specified worker
        // 2. Updates them to running status
        // 3. Sets claimed_by, claimed_at
        // 4. Increments retry_count if reclaiming a stale activity
        // 5. Uses FOR UPDATE SKIP LOCKED for safe concurrency
        // 6. Orders by scheduled_for for fair scheduling across all activity types
        //
        // Filtering by worker only (not activity name) ensures fair scheduling
        // across all activity types that this worker handles.
        let activities = sqlx::query!(
            r#"
            UPDATE activity_queue
            SET status = 'running'::activity_status,
                claimed_at = NOW(),
                claimed_by = $1::TEXT,
                retry_count = CASE
                    WHEN status = 'running'::activity_status THEN retry_count + 1
                    ELSE retry_count
                END
            WHERE id = ANY(
                SELECT id FROM activity_queue
                WHERE worker = $2
                AND (
                    -- Fresh pending activities
                    (status = 'pending'::activity_status AND scheduled_for <= NOW())
                    OR
                    -- Stale running activities (timeout expired, retries not exhausted)
                    (status = 'running'::activity_status
                     AND NOW() > claimed_at + timeout_duration
                     AND retry_count < max_retries)
                )
                ORDER BY scheduled_for ASC
                LIMIT $3
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, workflow_id, activity_key, worker, name as activity_name, parameters, 
                      settings, retry_count, claimed_at, output_definitions, iteration, signal_data
            "#,
            worker_id,
            worker,
            max_activities as i64
        )
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::with_capacity(activities.len());

        for row in activities {
            let settings: Option<ActivitySettings> = row
                .settings
                .map(|v| serde_json::from_value(v))
                .transpose()?;

            let output_definitions = row
                .output_definitions
                .map(|v| serde_json::from_value(v))
                .transpose()?;

            let queued = QueuedActivity {
                id: row.id,
                workflow_id: row.workflow_id,
                activity_key: row.activity_key,
                worker: row.worker,
                activity_name: row.activity_name,
                parameters: row.parameters,
                settings,
                retry_count: row.retry_count,
                claimed_at: row.claimed_at.unwrap(),
                output_definitions,
                iteration: row.iteration,
                signal_data: row.signal_data,
            };

            if queued.retry_count > 0 {
                warn!(
                    activity_id = %queued.id,
                    workflow_id = %queued.workflow_id,
                    retry_count = queued.retry_count,
                    "Reclaimed stale activity"
                );
            } else {
                debug!(
                    activity_id = %queued.id,
                    workflow_id = %queued.workflow_id,
                    "Claimed activity"
                );
            }

            results.push(queued);
        }

        if results.is_empty() {
            trace!(worker = %worker, "No claimable activities for worker");
        } else {
            trace!(worker = %worker, claimed_count = results.len(), "Activities claimed");
        }

        Ok(results)
    }

    async fn get_activity_summary(&self, activity_id: Uuid) -> Result<ActivitySummary> {
        let details = sqlx::query!(
            r#"
            SELECT workflow_id, activity_key, iteration
            FROM activity_queue
            WHERE id = $1
            "#,
            activity_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(QueueError::ActivityNotFound(activity_id))?;

        Ok(ActivitySummary {
            workflow_id: details.workflow_id,
            activity_key: details.activity_key,
            iteration: details.iteration,
        })
    }

    #[tracing::instrument(skip(self, _result), level = "debug")]
    async fn complete(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        _result: ActivityResult,
    ) -> Result<()> {
        // Verify the activity is claimed by this worker before completing
        // Use soft-delete (status='completed') instead of hard-delete to prevent
        // race condition with in-flight heartbeat requests
        let result = sqlx::query!(
            r#"
            UPDATE activity_queue
            SET status = 'completed'::activity_status,
                completed_at = NOW()
            WHERE id = $1 AND claimed_by = $2 AND status = 'running'::activity_status
            "#,
            activity_id,
            worker_id
        )
        .execute(&self.pool)
        .await?;

        if result.rows_affected() > 0 {
            debug!(
                activity_id = %activity_id,
                worker_id = %worker_id,
                "Activity marked as completed"
            );
            Ok(())
        } else {
            // Check if activity exists to provide better error
            let exists = sqlx::query!(
                r#"
                SELECT id, claimed_by, status AS "status: ActivityStatus"
                FROM activity_queue
                WHERE id = $1
                "#,
                activity_id
            )
            .fetch_optional(&self.pool)
            .await?;

            match exists {
                Some(activity) => {
                    if activity.claimed_by.as_deref() != Some(worker_id) {
                        error!(
                            activity_id = %activity_id,
                            expected_worker = ?activity.claimed_by,
                            actual_worker = %worker_id,
                            "Activity claimed by different worker"
                        );
                        Err(QueueError::ActivityReclaimed)
                    } else if activity.status == ActivityStatus::Completed {
                        // Activity already completed - this is idempotent, return success
                        debug!(
                            activity_id = %activity_id,
                            "Activity already completed (idempotent)"
                        );
                        Ok(())
                    } else {
                        // Activity exists but status is wrong
                        warn!(
                            activity_id = %activity_id,
                            status = ?activity.status,
                            "Activity not in running state"
                        );
                        Err(QueueError::InvalidStatus {
                            expected: "running".to_string(),
                            actual: format!("{:?}", activity.status),
                        })
                    }
                }
                None => {
                    // Activity doesn't exist (shouldn't happen with soft-delete)
                    debug!(
                        activity_id = %activity_id,
                        "Activity not found"
                    );
                    Err(QueueError::ActivityNotFound(activity_id))
                }
            }
        }
    }

    async fn heartbeat(&self, activity_id: Uuid, worker_id: &str) -> Result<()> {
        // Reset claimed_at to extend timeout deadline
        // Only update if status is 'running' - ignore heartbeats for completed activities
        let result = sqlx::query!(
            r#"
            UPDATE activity_queue
            SET claimed_at = NOW()
            WHERE id = $1
              AND claimed_by = $2::TEXT
              AND status = 'running'::activity_status
            RETURNING claimed_by
            "#,
            activity_id,
            worker_id
        )
        .fetch_optional(&self.pool)
        .await?;

        match result {
            Some(_) => {
                debug!(activity_id = %activity_id, "Heartbeat accepted");
                Ok(())
            }
            None => {
                // Check if activity exists and its status
                let exists = sqlx::query!(
                    r#"
                    SELECT id, status AS "status: ActivityStatus", claimed_by
                    FROM activity_queue
                    WHERE id = $1
                    "#,
                    activity_id
                )
                .fetch_optional(&self.pool)
                .await?;

                match exists {
                    Some(activity) => {
                        if activity.status == ActivityStatus::Completed
                            || activity.status == ActivityStatus::Failed
                        {
                            // Activity already finished - heartbeat no longer needed
                            // This is normal for race condition where completion/failure happens
                            // while heartbeat is in-flight. Return success silently.
                            debug!(
                                activity_id = %activity_id,
                                status = ?activity.status,
                                "Heartbeat received for finished activity (ignored)"
                            );
                            Ok(())
                        } else if activity.claimed_by.as_deref() != Some(worker_id) {
                            // Activity exists but claimed_by doesn't match
                            error!(
                                activity_id = %activity_id,
                                worker_id = %worker_id,
                                "Activity reclaimed by another worker"
                            );
                            Err(QueueError::ActivityReclaimed)
                        } else {
                            // Activity exists, correct worker, but wrong status
                            warn!(
                                activity_id = %activity_id,
                                status = ?activity.status,
                                "Activity not in running state"
                            );
                            Err(QueueError::InvalidStatus {
                                expected: "running".to_string(),
                                actual: format!("{:?}", activity.status),
                            })
                        }
                    }
                    None => {
                        // Activity doesn't exist at all
                        error!(activity_id = %activity_id, "Activity not found");
                        Err(QueueError::ActivityNotFound(activity_id))
                    }
                }
            }
        }
    }

    async fn fail(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        retryable: bool,
        result: ActivityResult,
    ) -> Result<bool> {
        // Begin explicit transaction to hold FOR UPDATE lock throughout the operation
        // This prevents race condition where lock is released between SELECT and UPDATE
        let mut tx = self.pool.begin().await?;

        // Get activity details to determine if we should retry
        // FOR UPDATE lock is held until transaction commits
        let activity = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key, status AS "status: ActivityStatus",
                   claimed_by, retry_count, max_retries, settings
            FROM activity_queue
            WHERE id = $1
            FOR UPDATE
            "#,
            activity_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(QueueError::ActivityNotFound(activity_id))?;

        // Verify the activity is claimed by this worker
        if activity.claimed_by.as_deref() != Some(worker_id) {
            error!(
                activity_id = %activity_id,
                expected_worker = ?activity.claimed_by,
                actual_worker = %worker_id,
                "Activity claimed by different worker"
            );
            return Err(QueueError::ActivityReclaimed);
        }

        // Verify the activity is still running
        if activity.status != ActivityStatus::Running {
            warn!(
                activity_id = %activity_id,
                status = ?activity.status,
                "Activity not in running state"
            );
            return Err(QueueError::InvalidStatus {
                expected: "running".to_string(),
                actual: format!("{:?}", activity.status),
            });
        }

        // Determine if we should retry based on retryable flag and retry count
        let will_retry = retryable && (activity.retry_count < activity.max_retries);

        if will_retry {
            // Requeue for retry (immediate, but behind other pending work)
            // TODO(post-MVP): Implement exponential backoff based on retry_count and activity settings
            sqlx::query!(
                r#"
                UPDATE activity_queue
                SET status = 'pending'::activity_status,
                    claimed_by = NULL,
                    claimed_at = NULL,
                    scheduled_for = NOW(),
                    retry_count = retry_count + 1
                WHERE id = $1
                "#,
                activity_id
            )
            .execute(&mut *tx)
            .await?;

            debug!(
                activity_id = %activity_id,
                workflow_id = %activity.workflow_id,
                retry_count = activity.retry_count + 1,
                max_retries = activity.max_retries,
                "Activity requeued for retry"
            );
        } else {
            // Permanent failure - mark as failed (soft-delete)
            sqlx::query!(
                r#"
                UPDATE activity_queue
                SET status = 'failed'::activity_status,
                    completed_at = NOW()
                WHERE id = $1
                "#,
                activity_id
            )
            .execute(&mut *tx)
            .await?;

            warn!(
                activity_id = %activity_id,
                workflow_id = %activity.workflow_id,
                activity_key = %activity.activity_key,
                error = ?result.error,
                "Activity permanently failed"
            );
        }

        // Commit transaction - releases FOR UPDATE lock
        tx.commit().await?;

        Ok(will_retry)
    }

    async fn reclaim_stale_activities(&self, limit: i64) -> Result<Vec<StaleActivityInfo>> {
        // Use a transaction to ensure consistent handling of all stale activities
        let mut tx = self.pool.begin().await?;

        // Find all stale activities (running + timeout exceeded)
        // We need to handle two cases:
        // 1. Activities with retries remaining -> reset to pending
        // 2. Activities with no retries remaining -> mark as failed
        let stale_activities = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key, iteration, retry_count, max_retries
            FROM activity_queue
            WHERE status = 'running'::activity_status
              AND claimed_at IS NOT NULL
              AND NOW() > claimed_at + timeout_duration
            ORDER BY claimed_at ASC
            LIMIT $1
            FOR UPDATE SKIP LOCKED
            "#,
            limit
        )
        .fetch_all(&mut *tx)
        .await?;

        if stale_activities.is_empty() {
            return Ok(vec![]);
        }

        let mut results = Vec::with_capacity(stale_activities.len());

        for activity in stale_activities {
            let has_retries = activity.retry_count < activity.max_retries;

            if has_retries {
                // Reset to pending for retry
                sqlx::query!(
                    r#"
                    UPDATE activity_queue
                    SET status = 'pending'::activity_status,
                        claimed_by = NULL,
                        claimed_at = NULL,
                        scheduled_for = NOW(),
                        retry_count = retry_count + 1
                    WHERE id = $1
                    "#,
                    activity.id
                )
                .execute(&mut *tx)
                .await?;

                warn!(
                    activity_id = %activity.id,
                    workflow_id = %activity.workflow_id,
                    activity_key = %activity.activity_key,
                    retry_count = activity.retry_count + 1,
                    max_retries = activity.max_retries,
                    "Reclaimed stale activity - reset to pending for retry"
                );

                results.push(StaleActivityInfo {
                    workflow_id: activity.workflow_id,
                    activity_key: activity.activity_key,
                    iteration: activity.iteration,
                    action: StaleActivityAction::ResetToPending,
                    retry_count: activity.retry_count + 1,
                    max_retries: activity.max_retries,
                });
            } else {
                // No retries remaining - mark as failed
                sqlx::query!(
                    r#"
                    UPDATE activity_queue
                    SET status = 'failed'::activity_status,
                        completed_at = NOW()
                    WHERE id = $1
                    "#,
                    activity.id
                )
                .execute(&mut *tx)
                .await?;

                warn!(
                    activity_id = %activity.id,
                    workflow_id = %activity.workflow_id,
                    activity_key = %activity.activity_key,
                    retry_count = activity.retry_count,
                    max_retries = activity.max_retries,
                    "Reclaimed stale activity - marked as failed (retries exhausted)"
                );

                results.push(StaleActivityInfo {
                    workflow_id: activity.workflow_id,
                    activity_key: activity.activity_key,
                    iteration: activity.iteration,
                    action: StaleActivityAction::MarkedFailed,
                    retry_count: activity.retry_count,
                    max_retries: activity.max_retries,
                });
            }
        }

        tx.commit().await?;

        Ok(results)
    }
}
