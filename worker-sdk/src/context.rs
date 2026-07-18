//! Activity execution context.

use crate::client::WorkerApiClient;
use crate::error::ClientError;
use serde_json::Value;
use uuid::Uuid;

/// Context passed to activity handlers during execution.
#[derive(Clone)]
pub struct ActivityContext {
    /// Workflow instance this activity belongs to
    pub workflow_id: Uuid,
    /// Unique identifier for this activity execution
    pub activity_id: Uuid,
    /// Activity key from the workflow definition
    pub activity_key: String,
    /// Data received from an external signal (when `wait_for_signal` is
    /// configured on the activity)
    pub signal: Option<Value>,

    worker_id: String,
    client: Option<WorkerApiClient>,
}

impl ActivityContext {
    /// Create a context without a client connection (useful in tests).
    pub fn new(workflow_id: Uuid, activity_id: Uuid, activity_key: impl Into<String>) -> Self {
        Self {
            workflow_id,
            activity_id,
            activity_key: activity_key.into(),
            signal: None,
            worker_id: String::new(),
            client: None,
        }
    }

    /// Attach signal data.
    pub fn with_signal(mut self, signal: Option<Value>) -> Self {
        self.signal = signal;
        self
    }

    /// Attach the API client and worker id so [`heartbeat`](Self::heartbeat)
    /// reaches the server. The poller does this for every claimed activity.
    pub fn with_client(mut self, client: WorkerApiClient, worker_id: impl Into<String>) -> Self {
        self.client = Some(client);
        self.worker_id = worker_id.into();
        self
    }

    /// The worker instance id executing this activity.
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Send a heartbeat to prevent the activity from being reclaimed as
    /// stale.
    ///
    /// The worker automatically heartbeats activities whose timeout exceeds
    /// 60 seconds; call this for finer control during long-running
    /// operations. No-op when the context has no client (tests).
    ///
    /// Returns [`ClientError::Conflict`] when the activity was completed or
    /// reassigned elsewhere — stop working on it.
    pub async fn heartbeat(&self) -> Result<(), ClientError> {
        if let Some(client) = &self.client {
            client.heartbeat(self.activity_id, &self.worker_id).await?;
        }
        Ok(())
    }
}
