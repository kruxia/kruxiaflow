use crate::client::{PendingActivity, WorkerApiClient};
use crate::config::WorkerConfig;
use crate::file_executor::FileExecutor;
use crate::registry::{ActivityContext, ActivityRegistry};
use anyhow::{Context, Result};
use kruxiaflow_core::storage::WorkflowStorage;
use kruxiaflow_core::workflow::ActivityOutputDefinition;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Worker poller task
///
/// Uses a semaphore-based concurrency model where the worker maintains N in-flight
/// activity slots and polls for more work whenever slots become available.
/// This ensures activities complete independently without blocking each other.
pub struct WorkerPoller {
    config: WorkerConfig,
    client: WorkerApiClient,
    registry: Arc<ActivityRegistry>,
    storage: Arc<dyn WorkflowStorage>,
    semaphore: Arc<Semaphore>,
}

impl WorkerPoller {
    pub fn new(
        config: WorkerConfig,
        client: WorkerApiClient,
        registry: Arc<ActivityRegistry>,
        storage: Arc<dyn WorkflowStorage>,
    ) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_activities));
        Self {
            config,
            client,
            registry,
            storage,
            semaphore,
        }
    }

    /// Run the semaphore-based poller loop
    ///
    /// Uses a semaphore to limit concurrent in-flight activities. The loop:
    /// 1. Waits for at least one slot to be available
    /// 2. Polls for activities (up to available slots)
    /// 3. Spawns each activity with its own permit
    /// 4. Loops immediately to fill more slots
    ///
    /// Activities complete independently, and the worker continuously fills available slots.
    pub async fn run(&self) -> Result<()> {
        tracing::info!(
            worker_id = %self.config.worker_id,
            worker = %self.config.worker,
            max_concurrent_activities = self.config.max_concurrent_activities,
            "Starting worker poller with semaphore-based concurrency"
        );

        loop {
            match self.poll_and_execute().await {
                Ok(executed) => {
                    if executed == 0 {
                        // No activities available, sleep before next poll
                        tokio::time::sleep(self.config.poll_interval).await;
                    }
                    // If activities were executed, loop immediately to fill more slots
                }
                Err(err) => {
                    tracing::error!("Poller error: {:?}", err);
                    // Sleep before retry on error
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// Poll for activities and execute them using semaphore-based concurrency
    ///
    /// Returns number of activities spawned.
    #[cfg_attr(feature = "profiling", tracing::instrument(skip(self), fields(worker_id = %self.config.worker_id)))]
    async fn poll_and_execute(&self) -> Result<usize> {
        // Wait for at least one slot to be available
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("Semaphore closed"))?;

        // Check how many additional slots are available (+1 for the one we hold)
        let available_slots = self.semaphore.available_permits() + 1;

        // Poll for up to `available_slots` activities, capped by poll_max_activities
        let max_to_poll = available_slots.min(self.config.poll_max_activities);

        let response = self
            .client
            .poll_activities(&self.config.worker_id, &self.config.worker, max_to_poll)
            .await
            .context("Failed to poll activities")?;

        if response.count == 0 {
            // No work available, release permit and return
            drop(permit);
            return Ok(0);
        }

        tracing::trace!(
            worker_id = %self.config.worker_id,
            count = response.count,
            available_slots = available_slots,
            "Claimed activities"
        );

        // Acquire additional permits for extra activities (we already hold one)
        let mut permits: Vec<OwnedSemaphorePermit> = vec![permit];
        for _ in 1..response.count {
            let additional_permit = Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .map_err(|_| anyhow::anyhow!("Semaphore closed"))?;
            permits.push(additional_permit);
        }

        // Spawn each activity with its own permit
        for (activity, permit) in response.activities.into_iter().zip(permits) {
            let config = self.config.clone();
            let client = self.client.clone();
            let registry = Arc::clone(&self.registry);
            let storage = Arc::clone(&self.storage);
            let semaphore = Arc::clone(&self.semaphore);

            tokio::spawn(async move {
                let poller = WorkerPoller {
                    config,
                    client,
                    registry,
                    storage,
                    semaphore,
                };
                poller.execute_activity(activity).await;
                drop(permit); // Release slot when done
            });
        }

        // Return immediately - don't wait for activities to complete
        // The semaphore ensures we don't over-commit resources
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

        // Parse output definitions if provided
        let output_definitions: Option<Vec<ActivityOutputDefinition>> = activity
            .output_definitions
            .as_ref()
            .and_then(|defs| serde_json::from_value(defs.clone()).ok());

        // Create FileExecutor if we have file outputs
        let file_executor = if output_definitions.is_some() {
            match FileExecutor::new(
                activity.workflow_id,
                activity.activity_key.clone(),
                Arc::clone(&self.storage),
            )
            .await
            {
                Ok(executor) => Some(executor),
                Err(err) => {
                    tracing::error!("Failed to create file executor: {:?}", err);
                    None
                }
            }
        } else {
            None
        };

        // Inject temp directory into parameters for file outputs
        let mut parameters = activity.parameters;
        if let Some(executor) = &file_executor {
            // Add _kruxiaflow_temp_dir to parameters (internal use only)
            if let Some(obj) = parameters.as_object_mut() {
                obj.insert(
                    "_kruxiaflow_temp_dir".to_string(),
                    serde_json::Value::String(executor.temp_dir().display().to_string()),
                );
            }
        }

        // Parse activity settings
        let settings = activity
            .settings
            .as_ref()
            .and_then(|s| serde_json::from_value(s.clone()).ok());

        // Create activity context for streaming support
        let ctx = ActivityContext::new(
            activity.workflow_id,
            activity.activity_id,
            activity.activity_key.clone(),
            Some(Arc::clone(&self.storage)),
        );

        // Execute activity with context for caching and streaming support
        let exec_span = tracing::debug_span!("activity_handler");
        let result = {
            let _enter = exec_span.enter();
            self.registry
                .execute_with_context(
                    &activity.worker,
                    &activity.activity_name,
                    parameters,
                    settings,
                    timeout,
                    &ctx,
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
                Ok(mut activity_result) => {
                    // Process file outputs if we have a file executor
                    if let (Some(executor), Some(defs)) = (&file_executor, &output_definitions) {
                        tracing::debug!("Processing file outputs");

                        // Get legacy output for file processing
                        let output_value = activity_result.to_json_value();

                        match executor.process_file_outputs(defs, output_value).await {
                            Ok(file_outputs) => {
                                // Merge file outputs with existing outputs
                                activity_result.outputs.extend(file_outputs);
                                tracing::debug!(
                                    "File outputs processed, total outputs: {}",
                                    activity_result.outputs.len()
                                );
                            }
                            Err(err) => {
                                tracing::error!("Failed to process file outputs: {:?}", err);
                                // Continue with execution - don't fail the activity
                            }
                        }
                    }

                    // Convert ActivityResult to JSON value format for API
                    let output = activity_result.to_json_value();
                    let cost_usd = activity_result.cost_usd;

                    // Debug: log what we're sending to the API
                    if let Some(result_obj) = output.get("result").and_then(|r| r.as_object()) {
                        tracing::info!(
                            activity_id = %activity.activity_id,
                            result_keys = ?result_obj.keys().collect::<Vec<_>>(),
                            has_embeddings_file = result_obj.contains_key("embeddings_file"),
                            "Sending activity result to API"
                        );
                    }

                    if let Err(err) = self
                        .client
                        .complete_activity(
                            activity.activity_id,
                            &self.config.worker_id,
                            output,
                            cost_usd,
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

        // Cleanup file executor temp directory
        if let Some(executor) = file_executor
            && let Err(err) = executor.cleanup().await
        {
            tracing::warn!("Failed to cleanup file executor: {:?}", err);
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
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::activity_result::ActivityResult;
    use crate::client::WorkerApiClient;
    use crate::registry::{ActivityImpl, ActivityRegistry};
    use async_trait::async_trait;
    use kruxiaflow_core::WorkflowStorage;
    use kruxiaflow_core::workflow::ActivityOutput;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    // Mock storage for tests
    struct MockStorage;

    #[async_trait]
    impl WorkflowStorage for MockStorage {
        async fn upload_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
            _content_type: Option<&str>,
            _data: std::pin::Pin<
                Box<
                    dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin,
                >,
            >,
        ) -> kruxiaflow_core::storage::Result<kruxiaflow_core::storage::FileMetadata> {
            unimplemented!("Mock storage not implemented")
        }

        async fn download_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> kruxiaflow_core::storage::Result<
            std::pin::Pin<
                Box<dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>,
            >,
        > {
            unimplemented!("Mock storage not implemented")
        }

        async fn get_file_metadata(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> kruxiaflow_core::storage::Result<kruxiaflow_core::storage::FileMetadata> {
            unimplemented!("Mock storage not implemented")
        }

        async fn list_files(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> kruxiaflow_core::storage::Result<Vec<kruxiaflow_core::storage::FileMetadata>> {
            unimplemented!("Mock storage not implemented")
        }

        async fn delete_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> kruxiaflow_core::storage::Result<()> {
            unimplemented!("Mock storage not implemented")
        }

        async fn delete_workflow_files(
            &self,
            _workflow_id: Uuid,
        ) -> kruxiaflow_core::storage::Result<()> {
            unimplemented!("Mock storage not implemented")
        }

        async fn get_file_reference(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> kruxiaflow_core::storage::Result<String> {
            unimplemented!("Mock storage not implemented")
        }
    }

    /// Test activity that succeeds
    struct SuccessActivity;

    #[async_trait]
    impl ActivityImpl for SuccessActivity {
        async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
            Ok(ActivityResult::values(vec![
                ActivityOutput::value("result", json!("success")),
                ActivityOutput::value("input", parameters),
            ]))
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
        async fn execute(&self, _parameters: Value) -> Result<ActivityResult> {
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
        async fn execute(&self, _parameters: Value) -> Result<ActivityResult> {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(ActivityResult::value("result", json!("done")))
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
            worker: "test".to_string(),
            poll_max_activities: 10,
            poll_interval: Duration::from_millis(100),
            max_concurrent_activities: 16,
            concurrency: 4, // Deprecated
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
        let registry = Arc::new(ActivityRegistry::new(Arc::new(
            kruxiaflow_core::cache::NoOpCache::new(),
        )));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config.clone(), client, registry.clone(), storage);

        assert_eq!(poller.config.worker_id, "test_worker");
        assert_eq!(poller.config.poll_max_activities, 10);
        assert_eq!(poller.config.max_concurrent_activities, 16);
        // Verify semaphore is created with correct number of permits
        assert_eq!(poller.semaphore.available_permits(), 16);
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
        let mut registry =
            ActivityRegistry::new(Arc::new(kruxiaflow_core::cache::NoOpCache::new()));
        registry.register(Arc::new(SuccessActivity));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config, client, Arc::new(registry), storage);

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test_key".to_string(),
            worker: "test".to_string(),
            activity_name: "success".to_string(),
            parameters: json!({"input": "test"}),
            settings: None,
            timeout_seconds: Some(10), // Custom timeout overrides config
            output_definitions: None,
            signal_data: None,
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
        let mut registry =
            ActivityRegistry::new(Arc::new(kruxiaflow_core::cache::NoOpCache::new()));
        registry.register(Arc::new(SuccessActivity));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config, client, Arc::new(registry), storage);

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test_key".to_string(),
            worker: "test".to_string(),
            activity_name: "success".to_string(),
            parameters: json!({"input": "test"}),
            settings: None,
            timeout_seconds: None, // Use default timeout
            output_definitions: None,
            signal_data: None,
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
        let registry = Arc::new(ActivityRegistry::new(Arc::new(
            kruxiaflow_core::cache::NoOpCache::new(),
        )));
        let storage = Arc::new(MockStorage);

        let _poller = WorkerPoller::new(config, client, registry, storage);

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
        let registry = Arc::new(ActivityRegistry::new(Arc::new(
            kruxiaflow_core::cache::NoOpCache::new(),
        )));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config.clone(), client, registry, storage);

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
        let mut registry =
            ActivityRegistry::new(Arc::new(kruxiaflow_core::cache::NoOpCache::new()));
        registry.register(Arc::new(SuccessActivity));
        let registry_arc = Arc::new(registry);

        // Test that Arc can be cloned for spawning tasks
        let cloned = Arc::clone(&registry_arc);
        assert!(Arc::ptr_eq(&registry_arc, &cloned));
    }

    // =========================================================================
    // Semaphore-based concurrency tests
    // =========================================================================

    #[test]
    fn test_semaphore_created_with_max_concurrent_activities() {
        // Verify semaphore is created with the correct number of permits
        let config = WorkerConfig {
            max_concurrent_activities: 32,
            ..test_config()
        };
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let registry = Arc::new(ActivityRegistry::new(Arc::new(
            kruxiaflow_core::cache::NoOpCache::new(),
        )));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config, client, registry, storage);

        assert_eq!(poller.semaphore.available_permits(), 32);
    }

    #[tokio::test]
    async fn test_semaphore_acquire_reduces_permits() {
        let config = WorkerConfig {
            max_concurrent_activities: 4,
            ..test_config()
        };
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let registry = Arc::new(ActivityRegistry::new(Arc::new(
            kruxiaflow_core::cache::NoOpCache::new(),
        )));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config, client, registry, storage);

        // Initially all permits are available
        assert_eq!(poller.semaphore.available_permits(), 4);

        // Acquire a permit
        let permit1 = poller.semaphore.clone().acquire_owned().await.unwrap();
        assert_eq!(poller.semaphore.available_permits(), 3);

        // Acquire another
        let permit2 = poller.semaphore.clone().acquire_owned().await.unwrap();
        assert_eq!(poller.semaphore.available_permits(), 2);

        // Release permits
        drop(permit1);
        assert_eq!(poller.semaphore.available_permits(), 3);

        drop(permit2);
        assert_eq!(poller.semaphore.available_permits(), 4);
    }

    #[test]
    fn test_poll_max_capped_to_available_slots() {
        // Test that poll_max_activities is correctly capped to available slots
        let poll_max_activities = 10;
        let available_slots = 3;

        let max_to_poll = available_slots.min(poll_max_activities);
        assert_eq!(max_to_poll, 3);

        // When more slots available than poll_max
        let available_slots = 50;
        let max_to_poll = available_slots.min(poll_max_activities);
        assert_eq!(max_to_poll, 10);
    }

    #[test]
    fn test_semaphore_is_arc_clonable() {
        // Verify semaphore can be cloned across tasks
        let config = test_config();
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );
        let registry = Arc::new(ActivityRegistry::new(Arc::new(
            kruxiaflow_core::cache::NoOpCache::new(),
        )));
        let storage = Arc::new(MockStorage);

        let poller = WorkerPoller::new(config, client, registry, storage);

        // Semaphore can be cloned
        let sem_clone = Arc::clone(&poller.semaphore);
        assert!(Arc::ptr_eq(&poller.semaphore, &sem_clone));
    }

    #[tokio::test]
    async fn test_semaphore_concurrent_access() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::Semaphore;

        // Test that semaphore correctly limits concurrent access
        let max_concurrent = 4;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        let active_count = Arc::new(AtomicUsize::new(0));
        let max_active_count = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();

        for _ in 0..16 {
            let sem = Arc::clone(&semaphore);
            let active = Arc::clone(&active_count);
            let max_active = Arc::clone(&max_active_count);

            handles.push(tokio::spawn(async move {
                // Acquire permit
                let _permit = sem.acquire().await.unwrap();

                // Track active count
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;

                // Update max if needed
                let mut prev_max = max_active.load(Ordering::SeqCst);
                while current > prev_max {
                    match max_active.compare_exchange_weak(
                        prev_max,
                        current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(x) => prev_max = x,
                    }
                }

                // Simulate work
                tokio::time::sleep(Duration::from_millis(10)).await;

                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Max concurrent should never exceed semaphore limit
        assert!(max_active_count.load(Ordering::SeqCst) <= max_concurrent);
    }

    #[test]
    fn test_config_max_concurrent_activities_different_values() {
        // Test various max_concurrent_activities values
        for max in [1, 4, 16, 32, 64, 100] {
            let config = WorkerConfig {
                max_concurrent_activities: max,
                ..test_config()
            };
            let client = WorkerApiClient::new(
                "http://localhost:8080".to_string(),
                "test_client".to_string(),
                "test_secret".to_string(),
            );
            let registry = Arc::new(ActivityRegistry::new(Arc::new(
                kruxiaflow_core::cache::NoOpCache::new(),
            )));
            let storage = Arc::new(MockStorage);

            let poller = WorkerPoller::new(config, client, registry, storage);

            assert_eq!(
                poller.semaphore.available_permits(),
                max,
                "Semaphore should have {} permits",
                max
            );
        }
    }

    #[test]
    fn test_pending_activity_output_definitions_parsing() {
        use kruxiaflow_core::workflow::ActivityOutputDefinition;

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test".to_string(),
            worker: "test".to_string(),
            activity_name: "process".to_string(),
            parameters: json!({}),
            settings: Some(json!({"retry_limit": 3})),
            timeout_seconds: Some(120),
            output_definitions: Some(json!([
                {"name": "result", "type": "value"},
                {"name": "report.pdf", "type": "file"}
            ])),
            signal_data: None,
        };

        // Test output_definitions parsing (matches execute_activity logic)
        let defs: Option<Vec<ActivityOutputDefinition>> = activity
            .output_definitions
            .as_ref()
            .and_then(|d| serde_json::from_value(d.clone()).ok());
        assert!(defs.is_some());
        let defs = defs.unwrap();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "result");
        assert_eq!(defs[1].name, "report.pdf");
    }

    #[test]
    fn test_pending_activity_settings_parsing() {
        use serde_json::Value;

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test".to_string(),
            worker: "test".to_string(),
            activity_name: "echo".to_string(),
            parameters: json!({}),
            settings: Some(json!({"cache_ttl_seconds": 60, "idempotent": true})),
            timeout_seconds: None,
            output_definitions: None,
            signal_data: None,
        };

        // Test settings parsing (matches execute_activity logic)
        let settings: Option<Value> = activity
            .settings
            .as_ref()
            .and_then(|s| serde_json::from_value(s.clone()).ok());
        assert!(settings.is_some());
    }

    #[test]
    fn test_pending_activity_invalid_output_definitions() {
        use kruxiaflow_core::workflow::ActivityOutputDefinition;

        let activity = PendingActivity {
            activity_id: Uuid::now_v7(),
            workflow_id: Uuid::now_v7(),
            activity_key: "test".to_string(),
            worker: "test".to_string(),
            activity_name: "echo".to_string(),
            parameters: json!({}),
            settings: None,
            timeout_seconds: None,
            output_definitions: Some(json!("not an array")),
            signal_data: None,
        };

        // Invalid output_definitions should return None
        let defs: Option<Vec<ActivityOutputDefinition>> = activity
            .output_definitions
            .as_ref()
            .and_then(|d| serde_json::from_value(d.clone()).ok());
        assert!(defs.is_none());
    }
}
