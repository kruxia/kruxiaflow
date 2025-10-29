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

/// Activity Queue interface for scheduling and claiming activities
#[async_trait]
pub trait ActivityQueue: Send + Sync {
    /// Schedule activities to the queue (idempotent via UNIQUE constraint)
    async fn schedule(&self, workflow_id: Uuid, activities: Vec<Activity>) -> Result<()>;

    /// Claim next pending activity for the given namespace/name (includes stale activity detection)
    async fn claim_next(
        &self,
        worker_id: Uuid,
        namespace: &str,
        name: &str,
    ) -> Result<Option<QueuedActivity>>;

    /// Complete an activity and remove it from queue
    async fn complete(&self, activity_id: Uuid, result: ActivityResult) -> Result<()>;

    /// Send heartbeat for long-running activity (extends timeout deadline)
    async fn heartbeat(&self, activity_id: Uuid, worker_id: Uuid) -> Result<()>;
}
