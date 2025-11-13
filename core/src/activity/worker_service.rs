use crate::events::{EventSource, NewWorkflowEvent, WorkflowEventType};
use crate::queue::{ActivityQueue, ActivityResult, QueueError, QueuedActivity};
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
            worker: qa.worker,
            name: qa.name,
            parameters: qa.parameters,
            settings: qa.settings.map(|s| serde_json::to_value(s).unwrap()),
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
    /// Claims activities matching the specified types by calling ActivityQueue::claim_next.
    /// Returns up to max_activities that are ready to execute.
    pub async fn poll_activities(
        &self,
        activity_types: Vec<(String, String)>, // Vec of (worker, name)
        worker_id: String,
        max_activities: usize,
    ) -> ActivityWorkerResult<Vec<PendingActivityRecord>> {
        let mut claimed = Vec::new();

        // Poll for each activity type until we reach max_activities
        for (worker, name) in activity_types {
            while claimed.len() < max_activities {
                // Delegate to ActivityQueue
                match self.queue.claim_next(&worker_id, &worker, &name).await? {
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

        if claimed.len() > 0 {
            tracing::debug!(
                worker_id = %worker_id,
                claimed_count = claimed.len(),
                "Activities claimed by worker"
            );
        }

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
        output: Value,
        cost_usd: Option<f64>,
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

        // Service-layer responsibility: Publish ActivityCompleted event via EventSource
        let event_payload = serde_json::json!({
            "activity_key": activity.activity_key,
            "output": output,
            "cost_usd": cost_usd
        });

        self.event_source
            .publish(NewWorkflowEvent {
                workflow_id: activity.workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some(activity.activity_key.clone()),
                payload: event_payload,
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
    pub async fn fail_activity(
        &self,
        activity_id: Uuid,
        worker_id: String,
        error_code: String,
        error_message: String,
        retryable: bool,
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

        // Service-layer responsibility: Publish ActivityFailed event via EventSource
        let event_payload = serde_json::json!({
            "activity_key": activity.activity_key,
            "error_code": error_code,
            "error_message": error_message,
            "retryable": retryable,
            "will_retry": will_retry
        });

        self.event_source
            .publish(NewWorkflowEvent {
                workflow_id: activity.workflow_id,
                event_type: WorkflowEventType::ActivityFailed,
                activity_key: Some(activity.activity_key.clone()),
                payload: event_payload,
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
