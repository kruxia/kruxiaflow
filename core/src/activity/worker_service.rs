use crate::queue::{ActivityQueue, ActivityResult, QueueError, QueuedActivity};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

/// Activity worker service error
#[derive(Debug, Error)]
pub enum ActivityWorkerError {
    #[error("Activity not found: {0}")]
    ActivityNotFound(Uuid),

    #[error("Activity already completed or failed")]
    ActivityAlreadyCompleted,

    #[error("Activity claimed by different worker")]
    WrongWorker,

    #[error("Queue error: {0}")]
    QueueError(#[from] QueueError),

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

impl From<QueuedActivity> for PendingActivityRecord {
    fn from(qa: QueuedActivity) -> Self {
        Self {
            id: qa.id,
            workflow_id: qa.workflow_id,
            activity_key: qa.activity_key,
            namespace: qa.namespace,
            name: qa.name,
            parameters: qa.parameters,
            settings: qa.settings.map(|s| serde_json::to_value(s).unwrap()),
        }
    }
}

/// Activity worker service
///
/// Coordinates between ActivityQueue (for queue operations) and event publishing
/// (for orchestrator notifications). This service delegates queue operations to
/// the ActivityQueue trait and handles the service-layer concern of publishing
/// workflow events.
///
/// This follows the service interface pattern - ActivityQueue handles database
/// operations while this service provides higher-level coordination.
#[derive(Clone)]
pub struct ActivityWorkerService {
    queue: Arc<dyn ActivityQueue>,
    pool: PgPool, // For event publishing until EventPublisher trait exists
}

impl ActivityWorkerService {
    pub fn new(queue: Arc<dyn ActivityQueue>, pool: PgPool) -> Self {
        Self { queue, pool }
    }

    /// Poll for pending activities
    ///
    /// Claims activities matching the specified types by calling ActivityQueue::claim_next.
    /// Returns up to max_activities that are ready to execute.
    pub async fn poll_activities(
        &self,
        activity_types: Vec<(String, String)>, // Vec of (namespace, name)
        worker_id: String,
        max_activities: usize,
    ) -> ActivityWorkerResult<Vec<PendingActivityRecord>> {
        let mut claimed = Vec::new();

        // Poll for each activity type until we reach max_activities
        for (namespace, name) in activity_types {
            while claimed.len() < max_activities {
                // Delegate to ActivityQueue
                match self.queue.claim_next(&worker_id, &namespace, &name).await? {
                    Some(activity) => {
                        claimed.push(PendingActivityRecord::from(activity));
                    }
                    None => break, // No more activities of this type
                }
            }

            if claimed.len() >= max_activities {
                break;
            }
        }

        tracing::info!(
            worker_id = %worker_id,
            claimed_count = claimed.len(),
            "Activities claimed by worker"
        );

        Ok(claimed)
    }

    /// Send heartbeat for an activity
    ///
    /// Delegates to ActivityQueue::heartbeat to extend timeout deadline.
    /// Returns recommended heartbeat interval in seconds.
    pub async fn heartbeat_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
    ) -> ActivityWorkerResult<i64> {
        // Delegate to ActivityQueue
        self.queue
            .heartbeat(activity_id, &worker_id)
            .await
            .map_err(|e| match e {
                QueueError::ActivityNotFound(id) => ActivityWorkerError::ActivityNotFound(id),
                QueueError::ActivityReclaimed => ActivityWorkerError::WrongWorker,
                other => ActivityWorkerError::QueueError(other),
            })?;

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
    /// Delegates to ActivityQueue::complete and publishes ActivityCompleted event.
    pub async fn complete_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        output: Value,
        cost_usd: Option<f64>,
    ) -> ActivityWorkerResult<()> {
        let mut tx = self.pool.begin().await?;

        // Get activity details before completion (for event publishing)
        let activity = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key
            FROM activity_queue
            WHERE id = $1
            "#,
            activity_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ActivityWorkerError::ActivityNotFound(activity_id))?;

        // Delegate to ActivityQueue
        let result = ActivityResult {
            success: true,
            outputs: Some(output.clone()),
            error: None,
            cost_usd,
            token_usage: None,
        };

        self.queue
            .complete(activity_id, &worker_id, result)
            .await
            .map_err(|e| match e {
                QueueError::ActivityNotFound(id) => ActivityWorkerError::ActivityNotFound(id),
                QueueError::ActivityReclaimed => ActivityWorkerError::WrongWorker,
                QueueError::InvalidStatus { .. } => ActivityWorkerError::ActivityAlreadyCompleted,
                other => ActivityWorkerError::QueueError(other),
            })?;

        // Service-layer responsibility: Publish ActivityCompleted event
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
    /// Delegates to ActivityQueue::fail and publishes ActivityFailed event.
    pub async fn fail_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        error_code: String,
        error_message: String,
        retryable: bool,
    ) -> ActivityWorkerResult<bool> {
        let mut tx = self.pool.begin().await?;

        // Get activity details before failure (for event publishing)
        let activity = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key
            FROM activity_queue
            WHERE id = $1
            "#,
            activity_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ActivityWorkerError::ActivityNotFound(activity_id))?;

        // Delegate to ActivityQueue
        let result = ActivityResult {
            success: false,
            outputs: None,
            error: Some(error_message.clone()),
            cost_usd: None,
            token_usage: None,
        };

        let will_retry = self
            .queue
            .fail(activity_id, &worker_id, retryable, result)
            .await
            .map_err(|e| match e {
                QueueError::ActivityNotFound(id) => ActivityWorkerError::ActivityNotFound(id),
                QueueError::ActivityReclaimed => ActivityWorkerError::WrongWorker,
                QueueError::InvalidStatus { .. } => ActivityWorkerError::ActivityAlreadyCompleted,
                other => ActivityWorkerError::QueueError(other),
            })?;

        // Service-layer responsibility: Publish ActivityFailed event
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
