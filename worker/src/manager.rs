use crate::client::WorkerApiClient;
use crate::config::WorkerConfig;
use crate::poller::WorkerPoller;
use crate::registry::ActivityRegistry;
use anyhow::Result;
use std::sync::Arc;
use streamflow_core::storage::WorkflowStorage;
use tokio::task::JoinHandle;

/// Worker manager
///
/// Spawns and manages multiple worker poller tasks.
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
    /// Spawns N worker poller tasks based on config.concurrency.
    pub async fn start(&self) -> Result<Vec<JoinHandle<()>>> {
        tracing::info!(
            worker_id = %self.config.worker_id,
            concurrency = self.config.concurrency,
            "Starting worker manager"
        );

        let client = WorkerApiClient::new(
            self.config.api_url.clone(),
            self.config.client_id.clone(),
            self.config.client_secret.clone(),
        );

        let mut handles = Vec::new();

        for i in 0..self.config.concurrency {
            // Create a unique worker_id for each poller thread
            let mut poller_config = self.config.clone();
            poller_config.worker_id = format!("{}_poller_{}", self.config.worker_id, i);

            let poller = WorkerPoller::new(
                poller_config,
                client.clone(),
                Arc::clone(&self.registry),
                Arc::clone(&self.storage),
            );

            let handle = tokio::spawn(async move {
                tracing::info!(poller_id = i, "Starting poller task");
                if let Err(err) = poller.run().await {
                    tracing::error!(poller_id = i, error = ?err, "Poller task failed");
                }
            });

            handles.push(handle);
        }

        tracing::info!("Worker manager started with {} pollers", handles.len());

        Ok(handles)
    }

    /// Stop worker
    ///
    /// Gracefully shuts down all poller tasks.
    pub async fn stop(&self, handles: Vec<JoinHandle<()>>) {
        tracing::info!("Stopping worker manager");

        for handle in handles {
            handle.abort();
        }

        tracing::info!("Worker manager stopped");
    }
}
