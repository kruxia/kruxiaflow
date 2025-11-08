use crate::client::{PendingActivity, WorkerApiClient};
use crate::config::WorkerConfig;
use crate::registry::ActivityRegistry;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// Worker poller task
///
/// Continuously polls for activities, executes them, and reports results.
pub struct WorkerPoller {
    config: WorkerConfig,
    client: WorkerApiClient,
    registry: Arc<ActivityRegistry>,
}

impl WorkerPoller {
    pub fn new(
        config: WorkerConfig,
        client: WorkerApiClient,
        registry: Arc<ActivityRegistry>,
    ) -> Self {
        Self {
            config,
            client,
            registry,
        }
    }

    /// Run the poller loop
    pub async fn run(&self) -> Result<()> {
        tracing::info!(
            worker_id = %self.config.worker_id,
            activity_types = ?self.config.activity_types,
            "Starting worker poller"
        );

        loop {
            match self.poll_and_execute().await {
                Ok(executed) => {
                    if executed == 0 {
                        // No activities available, sleep before next poll
                        tokio::time::sleep(self.config.poll_interval).await;
                    }
                    // If activities were executed, poll immediately for more
                }
                Err(err) => {
                    tracing::error!("Poller error: {:?}", err);
                    // Sleep before retry on error
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// Poll for activities and execute them
    ///
    /// Returns number of activities executed.
    async fn poll_and_execute(&self) -> Result<usize> {
        // Poll for activities
        let response = self
            .client
            .poll_activities(
                &self.config.worker_id,
                self.config.activity_types.clone(),
                self.config.max_activities_per_poll,
            )
            .await
            .context("Failed to poll activities")?;

        if response.count == 0 {
            return Ok(0);
        }

        tracing::info!(
            worker_id = %self.config.worker_id,
            count = response.count,
            "Claimed activities"
        );

        // Execute all activities concurrently
        // Since activities are already ready (no dependencies), spawn them all in parallel
        let mut tasks = Vec::new();
        for activity in response.activities {
            let config = self.config.clone();
            let client = self.client.clone();
            let registry = Arc::clone(&self.registry);

            tasks.push(tokio::spawn(async move {
                let poller = WorkerPoller {
                    config,
                    client,
                    registry,
                };
                poller.execute_activity(activity).await;
            }));
        }

        // Wait for all activities to complete
        for task in tasks {
            if let Err(err) = task.await {
                tracing::error!("Activity task panicked: {:?}", err);
            }
        }

        Ok(response.count)
    }

    /// Execute a single activity
    async fn execute_activity(&self, activity: PendingActivity) {
        tracing::info!(
            activity_id = %activity.activity_id,
            activity_key = %activity.activity_key,
            namespace = %activity.namespace,
            name = %activity.name,
            "Executing activity"
        );

        // Determine timeout
        let timeout = if let Some(seconds) = activity.timeout_seconds {
            Duration::from_secs(seconds as u64)
        } else {
            self.config.activity_timeout
        };

        // Spawn heartbeat task for long-running activities
        let heartbeat_handle = if timeout > Duration::from_secs(60) {
            Some(self.spawn_heartbeat_task(activity.activity_id))
        } else {
            None
        };

        // Execute activity
        let result = self
            .registry
            .execute(
                &activity.namespace,
                &activity.name,
                activity.parameters,
                timeout,
            )
            .await;

        // Report result BEFORE canceling heartbeat to avoid race condition
        // (see Risk section: if we abort heartbeat first, completion API call delay
        // could allow another worker to reclaim the activity as stale)
        match result {
            Ok(output) => {
                if let Err(err) = self
                    .client
                    .complete_activity(activity.activity_id, &self.config.worker_id, output, None)
                    .await
                {
                    tracing::error!(
                        activity_id = %activity.activity_id,
                        error = ?err,
                        "Failed to report activity completion"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    activity_id = %activity.activity_id,
                    error = %err,
                    "Activity execution failed"
                );

                if let Err(err) = self
                    .client
                    .fail_activity(
                        activity.activity_id,
                        &self.config.worker_id,
                        "EXECUTION_ERROR".to_string(),
                        err.to_string(),
                        true, // Retryable by default
                    )
                    .await
                {
                    tracing::error!(
                        activity_id = %activity.activity_id,
                        error = ?err,
                        "Failed to report activity failure"
                    );
                }
            }
        }

        // Cancel heartbeat task AFTER reporting completion
        // This ensures activity is marked completed in database before heartbeats stop,
        // preventing race condition where activity could be reclaimed as stale
        if let Some(handle) = heartbeat_handle {
            handle.abort();
        }
    }

    /// Spawn heartbeat task
    ///
    /// Sends periodic heartbeats until cancelled.
    fn spawn_heartbeat_task(&self, activity_id: uuid::Uuid) -> tokio::task::JoinHandle<()> {
        let client = self.client.clone();
        let worker_id = self.config.worker_id.clone();
        let interval = self.config.heartbeat_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                if let Err(err) = client.heartbeat(activity_id, &worker_id).await {
                    tracing::warn!(
                        activity_id = %activity_id,
                        error = ?err,
                        "Failed to send heartbeat"
                    );
                }
            }
        })
    }
}
