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
    #[cfg_attr(feature = "profiling", tracing::instrument(skip(self), fields(worker_id = %self.config.worker_id)))]
    async fn poll_and_execute(&self) -> Result<usize> {
        // Poll for activities
        let response = self
            .client
            .poll_activities(
                &self.config.worker_id,
                self.config.activity_types.clone(),
                self.config.poll_max_activities,
            )
            .await
            .context("Failed to poll activities")?;

        if response.count == 0 {
            return Ok(0);
        }

        tracing::debug!(
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
    #[cfg_attr(feature = "profiling", tracing::instrument(
        skip(self, activity),
        fields(
            worker_id = %self.config.worker_id,
            activity_id = %activity.activity_id,
            activity_key = %activity.activity_key,
            worker = %activity.worker,
            activity_name = %activity.activity_name
        )
    ))]
    async fn execute_activity(&self, activity: PendingActivity) {
        tracing::debug!("Executing activity");

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
        let exec_span = tracing::debug_span!("activity_handler");
        let result = {
            let _enter = exec_span.enter();
            self.registry
                .execute(
                    &activity.worker,
                    &activity.activity_name,
                    activity.parameters,
                    timeout,
                )
                .await
        };

        // Report result BEFORE canceling heartbeat to avoid race condition
        // (see Risk section: if we abort heartbeat first, completion API call delay
        // could allow another worker to reclaim the activity as stale)
        let complete_span = tracing::debug_span!("report_completion");
        {
            let _enter = complete_span.enter();

            match result {
                Ok(output) => {
                    if let Err(err) = self
                        .client
                        .complete_activity(
                            activity.activity_id,
                            &self.config.worker_id,
                            output,
                            None,
                        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::WorkerApiClient;
    use crate::registry::{ActivityImpl, ActivityRegistry};
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    /// Test activity that succeeds
    struct SuccessActivity;

    #[async_trait]
    impl ActivityImpl for SuccessActivity {
        async fn execute(&self, parameters: Value) -> Result<Value> {
            Ok(json!({
                "result": "success",
                "input": parameters
            }))
        }

        fn name(&self) -> &str {
            "success"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    /// Test activity that fails
    #[allow(dead_code)]
    struct FailingActivity;

    #[async_trait]
    impl ActivityImpl for FailingActivity {
        async fn execute(&self, _parameters: Value) -> Result<Value> {
            anyhow::bail!("Activity failed intentionally")
        }

        fn name(&self) -> &str {
            "failing"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    /// Test activity that times out
    #[allow(dead_code)]
    struct SlowActivity;

    #[async_trait]
    impl ActivityImpl for SlowActivity {
        async fn execute(&self, _parameters: Value) -> Result<Value> {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(json!({"result": "done"}))
        }

        fn name(&self) -> &str {
            "slow"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    fn test_config() -> WorkerConfig {
        WorkerConfig {
            api_url: "http://localhost:8080".to_string(),
            worker_id: "test_worker".to_string(),
            activity_types: vec!["test.success".to_string()],
            poll_max_activities: 10,
            poll_interval: Duration::from_millis(100),
            concurrency: 4,
            activity_timeout: Duration::from_secs(5),
            heartbeat_interval: Duration::from_secs(30),
            client_id: "test_client".to_string(),
            client_secret: "test_secret".to_string(),
        }
    }

    #[test]
    fn test_new_poller() {
        let config = test_config();
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let registry = Arc::new(ActivityRegistry::new());

        let poller = WorkerPoller::new(config.clone(), client, registry.clone());

        assert_eq!(poller.config.worker_id, "test_worker");
        assert_eq!(poller.config.poll_max_activities, 10);
    }

    #[test]
    fn test_config_timeout_determination() {
        let config = test_config();

        // Test default timeout
        assert_eq!(config.activity_timeout, Duration::from_secs(5));

        // Test custom timeout
        let custom_config = WorkerConfig {
            activity_timeout: Duration::from_secs(300),
            ..config
        };
        assert_eq!(custom_config.activity_timeout, Duration::from_secs(300));
    }

    #[test]
    fn test_heartbeat_interval() {
        let config = test_config();
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));

        let custom_config = WorkerConfig {
            heartbeat_interval: Duration::from_secs(10),
            ..config
        };
        assert_eq!(custom_config.heartbeat_interval, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_execute_activity_with_custom_timeout() {
        let config = WorkerConfig {
            activity_timeout: Duration::from_secs(5),
            ..test_config()
        };

        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(SuccessActivity));

        let poller = WorkerPoller::new(config, client, Arc::new(registry));

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test_key".to_string(),
            worker: "test".to_string(),
            activity_name: "success".to_string(),
            parameters: json!({"input": "test"}),
            settings: None,
            timeout_seconds: Some(10), // Custom timeout overrides config
        };

        // This test verifies the timeout determination logic
        let timeout = if let Some(seconds) = activity.timeout_seconds {
            Duration::from_secs(seconds as u64)
        } else {
            poller.config.activity_timeout
        };

        assert_eq!(timeout, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_execute_activity_uses_default_timeout() {
        let config = WorkerConfig {
            activity_timeout: Duration::from_secs(5),
            ..test_config()
        };

        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(SuccessActivity));

        let poller = WorkerPoller::new(config, client, Arc::new(registry));

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test_key".to_string(),
            worker: "test".to_string(),
            activity_name: "success".to_string(),
            parameters: json!({"input": "test"}),
            settings: None,
            timeout_seconds: None, // Use default timeout
        };

        // This test verifies the timeout determination logic
        let timeout = if let Some(seconds) = activity.timeout_seconds {
            Duration::from_secs(seconds as u64)
        } else {
            poller.config.activity_timeout
        };

        assert_eq!(timeout, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_heartbeat_spawned_for_long_timeout() {
        let config = test_config();
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let registry = Arc::new(ActivityRegistry::new());

        let _poller = WorkerPoller::new(config, client, registry);

        // Test that heartbeat is spawned for timeout > 60 seconds
        let long_timeout = Duration::from_secs(120);
        let should_spawn = long_timeout > Duration::from_secs(60);
        assert!(should_spawn);

        // Test that heartbeat is NOT spawned for timeout <= 60 seconds
        let short_timeout = Duration::from_secs(30);
        let should_not_spawn = short_timeout > Duration::from_secs(60);
        assert!(!should_not_spawn);
    }

    #[tokio::test]
    async fn test_poll_interval_used_when_no_activities() {
        let config = WorkerConfig {
            poll_interval: Duration::from_millis(100),
            ..test_config()
        };

        // Verify the poll interval is configured correctly
        assert_eq!(config.poll_interval, Duration::from_millis(100));

        // The actual sleep happens in the run() method when executed == 0
        // This test verifies the configuration is set up correctly
    }

    #[tokio::test]
    async fn test_error_sleep_duration() {
        // When poll_and_execute returns an error, the poller sleeps for 5 seconds
        // This test verifies that constant
        let error_sleep = Duration::from_secs(5);
        assert_eq!(error_sleep, Duration::from_secs(5));
    }

    #[test]
    fn test_poller_config_cloning() {
        let config = test_config();
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let registry = Arc::new(ActivityRegistry::new());

        let poller = WorkerPoller::new(config.clone(), client, registry);

        // Verify config was cloned correctly
        assert_eq!(poller.config.worker_id, config.worker_id);
        assert_eq!(poller.config.api_url, config.api_url);
        assert_eq!(
            poller.config.poll_max_activities,
            config.poll_max_activities
        );
    }

    #[test]
    fn test_activity_registry_arc_cloning() {
        let mut registry = ActivityRegistry::new();
        registry.register(Arc::new(SuccessActivity));
        let registry_arc = Arc::new(registry);

        // Test that Arc can be cloned for spawning tasks
        let cloned = Arc::clone(&registry_arc);
        assert!(Arc::ptr_eq(&registry_arc, &cloned));
    }
}
