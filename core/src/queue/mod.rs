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
}

/// Activity Queue interface for scheduling and claiming activities
#[async_trait]
pub trait ActivityQueue: Send + Sync {
    /// Schedule activities to the queue (idempotent via UNIQUE constraint)
    async fn schedule(&self, workflow_id: Uuid, activities: Vec<Activity>) -> Result<()>;

    /// Claim next pending activity for the given worker/name (includes stale activity detection)
    async fn claim_next(
        &self,
        worker_id: &str,
        worker: &str,
        name: &str,
    ) -> Result<Option<QueuedActivity>>;

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
}
