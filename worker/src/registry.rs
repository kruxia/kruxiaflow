use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use kruxiaflow_core::cache::{CacheService, CachedResult, key_generator};
use kruxiaflow_core::storage::WorkflowStorage;
use kruxiaflow_core::workflow::{ActivityOutput, ActivitySettings};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::activity_result::ActivityResult;

/// Context available to activities during execution
///
/// Provides activities with workflow metadata and optional storage access
/// for streaming large outputs.
#[derive(Clone)]
pub struct ActivityContext {
    /// Unique identifier for the workflow instance
    pub workflow_id: Uuid,
    /// Unique identifier for this activity execution
    pub activity_id: Uuid,
    /// Activity key from workflow definition
    pub activity_key: String,
    /// Optional workflow storage for streaming large outputs
    pub storage: Option<Arc<dyn WorkflowStorage>>,
}

impl ActivityContext {
    /// Create a new activity context
    pub fn new(
        workflow_id: Uuid,
        activity_id: Uuid,
        activity_key: String,
        storage: Option<Arc<dyn WorkflowStorage>>,
    ) -> Self {
        Self {
            workflow_id,
            activity_id,
            activity_key,
            storage,
        }
    }
}

/// Activity implementation trait
///
/// All activity implementations must implement this trait.
#[async_trait]
pub trait ActivityImpl: Send + Sync {
    /// Execute the activity with full context
    ///
    /// Override this method to access workflow storage for streaming large outputs.
    /// Default implementation delegates to the simple `execute` method.
    ///
    /// # Arguments
    /// * `parameters` - Activity input parameters
    /// * `ctx` - Activity context with workflow_id and optional storage
    ///
    /// # Returns
    /// * `Ok(ActivityResult)` - Activity result with outputs on success
    /// * `Err(error)` - Activity error on failure
    async fn execute_with_context(
        &self,
        parameters: Value,
        _ctx: &ActivityContext,
    ) -> Result<ActivityResult> {
        // Default: delegate to simple execute (backwards compatible)
        self.execute(parameters).await
    }

    /// Execute the activity (simple form)
    ///
    /// # Arguments
    /// * `parameters` - Activity input parameters
    ///
    /// # Returns
    /// * `Ok(ActivityResult)` - Activity result with outputs on success
    /// * `Err(error)` - Activity error on failure
    async fn execute(&self, parameters: Value) -> Result<ActivityResult>;

    /// Get activity name
    fn name(&self) -> &str;

    /// Get activity worker
    fn worker(&self) -> &str;
}

/// Activity registry
///
/// Manages activity implementations and executes them.
/// Includes caching support for all activity types.
pub struct ActivityRegistry {
    implementations: HashMap<String, Arc<dyn ActivityImpl>>,
    cache_service: Arc<dyn CacheService>,
}

impl ActivityRegistry {
    /// Create a new ActivityRegistry with a cache service
    pub fn new(cache_service: Arc<dyn CacheService>) -> Self {
        Self {
            implementations: HashMap::new(),
            cache_service,
        }
    }

    /// Register an activity implementation
    pub fn register(&mut self, implementation: Arc<dyn ActivityImpl>) {
        let key = format!("{}.{}", implementation.worker(), implementation.name());
        tracing::info!("Registering activity: {}", key);
        self.implementations.insert(key, implementation);
    }

    /// Get all registered activity types
    pub fn activity_types(&self) -> Vec<String> {
        self.implementations.keys().cloned().collect()
    }

    /// Execute an activity with caching support
    ///
    /// This method transparently handles caching for all activity types:
    /// - Checks cache before execution if caching is enabled
    /// - Returns cached result with cost_usd = 0.0 on cache hit
    /// - Stores result in cache after successful execution
    ///
    /// Returns activity result or error.
    pub async fn execute(
        &self,
        worker: &str,
        activity_name: &str,
        parameters: Value,
        settings: Option<ActivitySettings>,
        timeout: Duration,
    ) -> Result<ActivityResult> {
        // For backwards compatibility, create a minimal context
        let ctx = ActivityContext::new(
            Uuid::nil(),
            Uuid::nil(),
            String::new(),
            None,
        );
        self.execute_with_context(worker, activity_name, parameters, settings, timeout, &ctx)
            .await
    }

    /// Execute an activity with full context and caching support
    ///
    /// This method transparently handles caching for all activity types:
    /// - Checks cache before execution if caching is enabled
    /// - Returns cached result with cost_usd = 0.0 on cache hit
    /// - Stores result in cache after successful execution
    ///
    /// Returns activity result or error.
    pub async fn execute_with_context(
        &self,
        worker: &str,
        activity_name: &str,
        parameters: Value,
        settings: Option<ActivitySettings>,
        timeout: Duration,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult> {
        let key = format!("{}.{}", worker, activity_name);

        // Check if caching is enabled for this activity
        let cache_enabled = settings
            .as_ref()
            .and_then(|s| s.cache.then_some(true))
            .unwrap_or(false);

        // --- CACHE CHECK ---
        if cache_enabled && self.cache_service.is_available() {
            // Generate deterministic cache key from activity name + parameters
            let cache_key = key_generator::generate_cache_key(&key, &parameters)?;

            // Check cache for existing result
            if let Ok(Some(cached)) = self.cache_service.get(&cache_key).await {
                tracing::info!(
                    activity = %key,
                    cache_key = %cache_key,
                    "Cache hit - returning cached result"
                );

                // Convert cached JSON value to Vec<ActivityOutput>
                let outputs = if let Value::Object(map) = cached.output {
                    map.into_iter()
                        .map(|(k, v)| ActivityOutput::value(k, v))
                        .collect()
                } else {
                    vec![ActivityOutput::value("result", cached.output)]
                };

                // Return cached result with cost_usd = 0.0
                return Ok(ActivityResult {
                    outputs,
                    cost_usd: Some(Decimal::ZERO), // Cache hit = zero cost
                    metadata: Some(json!({
                        "cached": true,
                        "cache_key": cache_key,
                        "cached_at": cached.cached_at,
                        "original_cost_usd": cached.original_cost_usd,
                    })),
                });
            }

            tracing::debug!(
                activity = %key,
                cache_key = %cache_key,
                "Cache miss - executing activity"
            );
        }

        // --- EXECUTE ACTIVITY ---
        let implementation = self
            .implementations
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("Activity implementation not found: {}", key))?;

        // Execute with timeout using context-aware method
        let result = tokio::time::timeout(
            timeout,
            implementation.execute_with_context(parameters.clone(), ctx),
        )
        .await;

        let mut activity_result = match result {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Activity execution timed out after {:?}",
                    timeout
                ));
            }
        };

        // --- CACHE STORAGE ---
        if cache_enabled && self.cache_service.is_available() {
            let cache_key = key_generator::generate_cache_key(&key, &parameters)?;
            let cache_ttl = settings.as_ref().and_then(|s| s.cache_ttl).unwrap_or(3600); // Default 1 hour

            // Convert outputs to JSON value for caching
            let output_value = activity_result.to_json_value();

            let cached_result = CachedResult {
                output: output_value,
                cached_at: Utc::now(),
                original_cost_usd: activity_result.cost_usd,
            };

            match self
                .cache_service
                .set(&cache_key, &cached_result, Duration::from_secs(cache_ttl))
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        activity = %key,
                        cache_key = %cache_key,
                        ttl_seconds = cache_ttl,
                        "Stored result in cache"
                    );

                    // Add cache_key to result metadata
                    let cache_metadata = json!({
                        "cache_key": cache_key,
                        "cached": false,
                    });

                    activity_result.metadata = match activity_result.metadata {
                        Some(Value::Object(mut map)) => {
                            map.insert("cache_key".to_string(), json!(cache_key));
                            map.insert("cached".to_string(), json!(false));
                            Some(Value::Object(map))
                        }
                        _ => Some(cache_metadata),
                    };
                }
                Err(err) => {
                    // Log cache storage error but don't fail the activity
                    tracing::warn!(
                        activity = %key,
                        error = %err,
                        "Failed to store result in cache"
                    );
                }
            }
        }

        Ok(activity_result)
    }
}

// Note: Default implementation removed - ActivityRegistry requires explicit cache service injection

#[cfg(test)]
#[path = "registry_test.rs"]
mod registry_test;
