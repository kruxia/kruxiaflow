use crate::client::WorkerApiClient;
use crate::config::WorkerConfig;
use crate::poller::WorkerPoller;
use crate::registry::ActivityRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Worker manager
///
/// Spawns and manages multiple worker poller tasks.
pub struct WorkerManager {
    config: WorkerConfig,
    registry: Arc<ActivityRegistry>,
}

impl WorkerManager {
    pub fn new(config: WorkerConfig, registry: ActivityRegistry) -> Self {
        Self {
            config,
            registry: Arc::new(registry),
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
            let poller = WorkerPoller::new(
                self.config.clone(),
                client.clone(),
                Arc::clone(&self.registry),
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
