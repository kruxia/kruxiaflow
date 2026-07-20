use crate::cost::UsageEntry;
use crate::events::{EventSource, NewWorkflowEvent, WorkflowEventType};
use crate::queue::{ActivityQueue, ActivityResult, QueueError, QueuedActivity};
use rust_decimal::Decimal;
use serde_json::Value;
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

    #[error("Event source error: {0}")]
    EventError(#[from] crate::events::EventError),

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
    pub worker: String,
    pub activity_name: String,
    pub parameters: Value,
    pub settings: Option<Value>,
    pub output_definitions: Option<Value>,
    /// Signal data for activities that were waiting for an external signal
    pub signal_data: Option<Value>,
}

impl From<QueuedActivity> for PendingActivityRecord {
    fn from(qa: QueuedActivity) -> Self {
        Self {
            id: qa.id,
            workflow_id: qa.workflow_id,
            activity_key: qa.activity_key,
            worker: qa.worker,
            activity_name: qa.activity_name,
            parameters: qa.parameters,
            settings: qa.settings.map(|s| serde_json::to_value(s).unwrap()),
            output_definitions: qa
                .output_definitions
                .map(|defs| serde_json::to_value(defs).unwrap()),
            signal_data: qa.signal_data,
        }
    }
}

/// Activity worker service
///
/// Coordinates between ActivityQueue (for queue operations) and EventSource
/// (for orchestrator notifications). This service delegates queue operations to
/// the ActivityQueue trait and event publishing to the EventSource trait.
///
/// This follows the service interface pattern - all infrastructure dependencies
/// are abstracted behind interfaces.
#[derive(Clone)]
pub struct ActivityWorkerService {
    queue: Arc<dyn ActivityQueue>,
    event_source: Arc<dyn EventSource>,
}

impl ActivityWorkerService {
    pub fn new(queue: Arc<dyn ActivityQueue>, event_source: Arc<dyn EventSource>) -> Self {
        Self {
            queue,
            event_source,
        }
    }

    /// Poll for pending activities
    ///
    /// Claims activities for the specified worker by calling ActivityQueue::claim_next.
    /// Returns up to max_activities that are ready to execute, ordered by scheduled_for
    /// for fair scheduling across all activity types.
    pub async fn poll_activities(
        &self,
        worker: &str,
        worker_id: &str,
        max_activities: usize,
    ) -> ActivityWorkerResult<Vec<PendingActivityRecord>> {
        // Single call to claim_next filtering by worker only
        let claimed = self
            .queue
            .claim_next(worker_id, worker, max_activities)
            .await?
            .into_iter()
            .map(PendingActivityRecord::from)
            .collect();

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
    /// Delegates to ActivityQueue::complete and publishes ActivityCompleted event via EventSource.
    pub async fn complete_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        outputs: Value,
        cost_usd: Option<Decimal>,
        usage: Option<Vec<UsageEntry>>,
    ) -> ActivityWorkerResult<()> {
        // Get activity details before completion (needed for event publishing)
        // This is read-only and doesn't modify state
        let activity = self
            .queue
            .get_activity_summary(activity_id)
            .await
            .map_err(|e| match e {
                QueueError::ActivityNotFound(id) => ActivityWorkerError::ActivityNotFound(id),
                other => ActivityWorkerError::QueueError(other),
            })?;

        // Delegate to ActivityQueue to complete the activity
        // This handles worker verification and queue removal atomically
        let mut result = ActivityResult::success(outputs.clone());
        result.cost_usd = cost_usd;

        self.queue
            .complete(activity_id, &worker_id, result)
            .await
            .map_err(|e| match e {
                QueueError::ActivityNotFound(id) => ActivityWorkerError::ActivityNotFound(id),
                QueueError::ActivityReclaimed => ActivityWorkerError::WrongWorker,
                QueueError::InvalidStatus { .. } => ActivityWorkerError::ActivityAlreadyCompleted,
                other => ActivityWorkerError::QueueError(other),
            })?;

        // Service-layer responsibility: Publish ActivityCompleted event via EventSource
        // IMPORTANT: outputs must be an object mapping names to values (e.g., {"response": {...}})
        // The orchestrator will convert this to Vec<ActivityOutput> using output_definitions
        let mut event_payload = serde_json::json!({
            "activity_key": activity.activity_key,
            "outputs": outputs,
            "cost_usd": cost_usd
        });
        if let Some(usage) = &usage
            && !usage.is_empty()
        {
            event_payload["usage"] = serde_json::to_value(usage)?;
        }

        self.event_source
            .publish(NewWorkflowEvent {
                workflow_id: activity.workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some(activity.activity_key.clone()),
                payload: event_payload,
                iteration: activity.iteration,
            })
            .await?;

        tracing::debug!(
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
    /// Delegates to ActivityQueue::fail and publishes ActivityFailed event via EventSource.
    #[allow(clippy::too_many_arguments)]
    pub async fn fail_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        error_code: String,
        error_message: String,
        retryable: bool,
        cost_usd: Option<Decimal>,
        usage: Option<Vec<UsageEntry>>,
    ) -> ActivityWorkerResult<bool> {
        // Get activity details before failure (needed for event publishing)
        // This is read-only and doesn't modify state
        let activity = self
            .queue
            .get_activity_summary(activity_id)
            .await
            .map_err(|e| match e {
                QueueError::ActivityNotFound(id) => ActivityWorkerError::ActivityNotFound(id),
                other => ActivityWorkerError::QueueError(other),
            })?;

        // Delegate to ActivityQueue to fail the activity
        // This handles worker verification, retry logic, and queue management atomically
        let result = ActivityResult {
            success: false,
            outputs: None,
            error: Some(error_message.clone()),
            cost_usd,
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

        // Service-layer responsibility: Publish ActivityFailed event via EventSource
        // A failed activity may still have spent money before erroring; carry the
        // reported cost/usage so the orchestrator records it under this attempt.
        // `attempt` (the 1-indexed attempt that just failed, read before
        // queue.fail incremented retry_count) makes each attempt's failure
        // event distinct under the per-attempt event dedup — without it, the
        // terminal failure would be deduped against the first retryable one
        // and the workflow would hang.
        let mut event_payload = serde_json::json!({
            "activity_key": activity.activity_key,
            "error_code": error_code,
            "error_message": error_message,
            "retryable": retryable,
            "will_retry": will_retry,
            "attempt": activity.attempt,
            "cost_usd": cost_usd
        });
        if let Some(usage) = &usage
            && !usage.is_empty()
        {
            event_payload["usage"] = serde_json::to_value(usage)?;
        }

        self.event_source
            .publish(NewWorkflowEvent {
                workflow_id: activity.workflow_id,
                event_type: WorkflowEventType::ActivityFailed,
                activity_key: Some(activity.activity_key.clone()),
                payload: event_payload,
                iteration: activity.iteration,
            })
            .await?;

        tracing::debug!(
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
