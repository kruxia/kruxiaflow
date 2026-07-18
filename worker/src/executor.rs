//! Std-worker activity execution behind the SDK's poll loop.
//!
//! The SDK crate (`kruxiaflow-worker`) owns polling, concurrency,
//! heartbeats, and completion reporting. This executor plugs the std
//! worker's extra machinery — cache-aware registry dispatch, streaming, and
//! file staging — in between.

use crate::file_executor::FileExecutor;
use crate::registry::{ActivityContext, ActivityRegistry};
use crate::streaming::HttpStreamSender;
use async_trait::async_trait;
use kruxiaflow_core::storage::WorkflowStorage;
use kruxiaflow_core::workflow::{ActivityOutputDefinition, ActivitySettings};
use kruxiaflow_worker::{
    ActivityContext as SdkActivityContext, ActivityError, ActivityExecutor, ActivityResult,
    PendingActivity, WorkerApiClient,
};
use std::sync::Arc;
use std::time::Duration;

/// Executes std-worker activities: cache-aware registry dispatch with
/// streaming and file-output support.
pub struct StdActivityExecutor {
    registry: Arc<ActivityRegistry>,
    storage: Arc<dyn WorkflowStorage>,
    client: WorkerApiClient,
}

impl StdActivityExecutor {
    pub fn new(
        registry: Arc<ActivityRegistry>,
        storage: Arc<dyn WorkflowStorage>,
        client: WorkerApiClient,
    ) -> Self {
        Self {
            registry,
            storage,
            client,
        }
    }
}

#[async_trait]
impl ActivityExecutor for StdActivityExecutor {
    async fn execute(
        &self,
        activity: &PendingActivity,
        _ctx: &SdkActivityContext,
        timeout: Duration,
    ) -> Result<ActivityResult, ActivityError> {
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
        let mut parameters = activity.parameters.clone();
        if let Some(executor) = &file_executor
            && let Some(obj) = parameters.as_object_mut()
        {
            obj.insert(
                "_kruxiaflow_temp_dir".to_string(),
                serde_json::Value::String(executor.temp_dir().display().to_string()),
            );
        }

        // Parse activity settings
        let settings: Option<ActivitySettings> = activity
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

        // Check if streaming is enabled and a streaming implementation exists
        let streaming_enabled = settings
            .as_ref()
            .map(|s| s.streaming.is_enabled())
            .unwrap_or(false);

        let streaming_impl = if streaming_enabled {
            self.registry
                .get_streaming(&activity.worker, &activity.activity_name)
                .cloned()
        } else {
            None
        };

        // Execute activity — streaming path or normal path
        let result = if let Some(streaming_activity) = streaming_impl {
            // Streaming path: create HttpStreamSender, check for subscribers
            let auth_token = self.client.get_token().await.ok().flatten();
            let sender = HttpStreamSender::new(
                self.client.api_url().to_string(),
                activity.activity_id,
                auth_token,
            );

            let has_subscribers = sender.has_subscribers().await.unwrap_or(false);

            if has_subscribers {
                tracing::info!(
                    activity_id = %activity.activity_id,
                    "Streaming dispatch: subscribers connected"
                );
                match tokio::time::timeout(
                    timeout,
                    streaming_activity.execute_streaming(
                        activity.activity_id,
                        parameters,
                        Box::new(sender),
                    ),
                )
                .await
                {
                    Ok(inner) => inner,
                    Err(_) => Err(anyhow::anyhow!(
                        "Activity execution timed out after {:?}",
                        timeout
                    )),
                }
            } else {
                tracing::debug!(
                    activity_id = %activity.activity_id,
                    "Streaming enabled but no subscribers, falling back to normal execution"
                );
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
            }
        } else {
            // Normal (non-streaming) execution path
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

        let result = match result {
            Ok(mut activity_result) => {
                // Process file outputs if we have a file executor
                if let (Some(executor), Some(defs)) = (&file_executor, &output_definitions) {
                    tracing::debug!("Processing file outputs");

                    let output_value = activity_result.to_json_value();
                    match executor.process_file_outputs(defs, output_value).await {
                        Ok(file_outputs) => {
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
                Ok(activity_result)
            }
            // anyhow errors from built-in activities stay retryable
            // EXECUTION_ERROR failures, matching the pre-SDK poller
            Err(err) => Err(ActivityError::from(err)),
        };

        // Cleanup file executor temp directory
        if let Some(executor) = file_executor
            && let Err(err) = executor.cleanup().await
        {
            tracing::warn!("Failed to cleanup file executor: {:?}", err);
        }

        result
    }
}
