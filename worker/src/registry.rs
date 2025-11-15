use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Activity implementation trait
///
/// All activity implementations must implement this trait.
#[async_trait]
pub trait ActivityImpl: Send + Sync {
    /// Execute the activity
    ///
    /// # Arguments
    /// * `parameters` - Activity input parameters
    ///
    /// # Returns
    /// * `Ok(output)` - Activity output on success
    /// * `Err(error)` - Activity error on failure
    async fn execute(&self, parameters: Value) -> Result<Value>;

    /// Get activity name
    fn name(&self) -> &str;

    /// Get activity worker
    fn worker(&self) -> &str;
}

/// Activity registry
///
/// Manages activity implementations and executes them.
pub struct ActivityRegistry {
    implementations: HashMap<String, Arc<dyn ActivityImpl>>,
}

impl ActivityRegistry {
    pub fn new() -> Self {
        Self {
            implementations: HashMap::new(),
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

    /// Execute an activity
    ///
    /// Returns activity output or error.
    pub async fn execute(
        &self,
        worker: &str,
        activity_name: &str,
        parameters: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let key = format!("{}.{}", worker, activity_name);

        let implementation = self
            .implementations
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("Activity implementation not found: {}", key))?;

        // Execute with timeout
        let result = tokio::time::timeout(timeout, implementation.execute(parameters)).await;

        match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(anyhow::anyhow!(
                "Activity execution timed out after {:?}",
                timeout
            )),
        }
    }
}

impl Default for ActivityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod registry_test;
