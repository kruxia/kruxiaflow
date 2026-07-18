use crate::executor::StdActivityExecutor;
use crate::registry::ActivityRegistry;
use anyhow::Result;
use kruxiaflow_core::storage::WorkflowStorage;
use kruxiaflow_worker::{WorkerApiClient, WorkerConfig, WorkerPoller};
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Worker manager
///
/// Spawns and manages a single SDK worker poller task (semaphore-based
/// concurrency) wired to the std-worker activity executor.
pub struct WorkerManager {
    config: WorkerConfig,
    registry: Arc<ActivityRegistry>,
    storage: Arc<dyn WorkflowStorage>,
}

impl WorkerManager {
    pub fn new(
        config: WorkerConfig,
        registry: ActivityRegistry,
        storage: Arc<dyn WorkflowStorage>,
    ) -> Self {
        Self {
            config,
            registry: Arc::new(registry),
            storage,
        }
    }

    /// Start worker
    ///
    /// Spawns a single worker poller task that uses a semaphore to manage
    /// concurrent activity execution (configured by max_concurrent_activities).
    pub async fn start(&self) -> Result<Vec<JoinHandle<()>>> {
        tracing::info!(
            worker_id = %self.config.worker_id,
            max_concurrent_activities = self.config.max_concurrent_activities,
            "Starting worker manager with semaphore-based concurrency"
        );

        let client = match (&self.config.client_id, &self.config.client_secret) {
            (Some(id), Some(secret)) => {
                WorkerApiClient::with_credentials(&self.config.api_url, id, secret)
            }
            _ => WorkerApiClient::new(&self.config.api_url),
        };

        let executor = Arc::new(StdActivityExecutor::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.storage),
            client.clone(),
        ));

        let poller = WorkerPoller::new(self.config.clone(), client, executor);

        let handle = tokio::spawn(async move {
            tracing::info!("Starting poller task");
            poller.run().await;
        });

        tracing::info!(
            "Worker manager started with 1 poller (max {} concurrent activities)",
            self.config.max_concurrent_activities
        );

        Ok(vec![handle])
    }

    /// Stop worker
    ///
    /// Shuts down the poller task.
    pub async fn stop(&self, handles: Vec<JoinHandle<()>>) {
        tracing::info!("Stopping worker manager");

        for handle in handles {
            handle.abort();
        }

        tracing::info!("Worker manager stopped");
    }
}
