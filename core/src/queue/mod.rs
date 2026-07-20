pub mod config;
pub mod error;
pub mod models;
pub mod monitor;
pub mod postgres_queue;

pub use config::QueueConfig;
pub use error::{QueueError, Result};
pub use models::*;
pub use monitor::QueueMonitor;
pub use postgres_queue::PostgresQueue;

use async_trait::async_trait;
use uuid::Uuid;

/// Activity details for event publishing
#[derive(Debug, Clone, Default)]
pub struct ActivitySummary {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub iteration: Option<i32>,
    /// Current attempt (1-indexed: retry_count + 1 at read time)
    pub attempt: i32,
}

/// Activity Queue interface for scheduling and claiming activities
#[async_trait]
pub trait ActivityQueue: Send + Sync {
    /// Schedule activities to the queue (idempotent via UNIQUE constraint)
    async fn schedule(&self, workflow_id: Uuid, activities: Vec<Activity>) -> Result<()>;

    /// Claim next pending activities for the given worker (includes stale activity detection)
    ///
    /// Claims up to `max_activities` activities for the specified worker, regardless of
    /// activity type. Activities are claimed in `scheduled_for` order for fair scheduling
    /// across all activity types.
    async fn claim_next(
        &self,
        worker_id: &str,
        worker: &str,
        max_activities: usize,
    ) -> Result<Vec<QueuedActivity>>;

    /// Get activity details (workflow_id, activity_key) for event publishing
    async fn get_activity_summary(&self, activity_id: Uuid) -> Result<ActivitySummary>;

    /// Complete an activity and remove it from queue
    async fn complete(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        result: ActivityResult,
    ) -> Result<()>;

    /// Fail an activity (remove from queue or requeue for retry based on retry settings)
    ///
    /// Returns true if the activity will be retried, false if it's permanently failed.
    async fn fail(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        retryable: bool,
        result: ActivityResult,
    ) -> Result<bool>;

    /// Send heartbeat for long-running activity (extends timeout deadline)
    async fn heartbeat(&self, activity_id: Uuid, worker_id: &str) -> Result<()>;

    /// Cancel a pending (not yet claimed) activity, marking it failed.
    ///
    /// Used by the orchestrator to withdraw a retry the queue has already
    /// requeued when a post-failure check (e.g. budget exhaustion) determines
    /// the retry must not run. Returns true if a pending row was cancelled;
    /// false if there was nothing to cancel (already claimed or terminal).
    async fn cancel_pending(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        iteration: Option<i32>,
    ) -> Result<bool>;

    /// Reclaim stale running activities that have exceeded their timeout.
    ///
    /// This is a background maintenance operation that:
    /// - Finds activities where status='running' AND NOW() > claimed_at + timeout_duration
    /// - If retry_count < max_retries: resets to pending for retry
    /// - If retry_count >= max_retries: marks as failed
    ///
    /// Returns information about all reclaimed activities for event emission.
    async fn reclaim_stale_activities(&self, limit: i64) -> Result<Vec<StaleActivityInfo>>;
}
