//! High-level worker: builder, run loop, graceful shutdown.

use crate::client::WorkerApiClient;
use crate::config::WorkerConfig;
use crate::error::ConfigError;
use crate::poller::WorkerPoller;
use crate::registry::{ActivityExecutor, ActivityImpl, ActivityRegistry, TypedActivity};
use crate::result::ActivityResult;
use crate::{ActivityContext, ActivityError};
use serde::de::DeserializeOwned;
use std::future::Future;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// A configured worker, ready to run.
///
/// ```no_run
/// use kruxiaflow_worker::{ActivityContext, ActivityResult, Worker};
/// use serde_json::json;
///
/// #[derive(serde::Deserialize)]
/// struct EchoParams {
///     message: String,
/// }
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let worker = Worker::builder()
///         .worker("demo")
///         .register_fn("echo", |params: EchoParams, _ctx: ActivityContext| async move {
///             Ok(ActivityResult::value("echoed", json!(params.message)))
///         })
///         .build()?; // config from KRUXIAFLOW_* environment variables
///
///     worker.run_until_shutdown().await;
///     Ok(())
/// }
/// ```
pub struct Worker {
    poller: WorkerPoller,
    shutdown: CancellationToken,
}

/// Clonable handle that triggers graceful shutdown programmatically.
#[derive(Clone)]
pub struct WorkerHandle {
    token: CancellationToken,
}

impl WorkerHandle {
    /// Stop polling and drain in-flight activities (up to the configured
    /// `shutdown_timeout`), failing whatever remains as retryable so it
    /// re-queues.
    pub fn shutdown(&self) {
        self.token.cancel();
    }
}

impl Worker {
    /// Start building a worker.
    pub fn builder() -> WorkerBuilder {
        WorkerBuilder {
            config: None,
            worker: None,
            registry: ActivityRegistry::new(),
            deferred: Vec::new(),
            executor: None,
        }
    }

    /// Handle for triggering graceful shutdown from elsewhere.
    pub fn handle(&self) -> WorkerHandle {
        WorkerHandle {
            token: self.shutdown.clone(),
        }
    }

    /// Run until the shutdown handle is triggered, then drain gracefully.
    pub async fn run(&self) {
        self.poller.run().await;
    }

    /// Run until SIGINT/SIGTERM (or the shutdown handle), then drain
    /// gracefully. Convenience for standalone worker binaries.
    pub async fn run_until_shutdown(&self) {
        let token = self.shutdown.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            tracing::info!("Shutdown signal received");
            token.cancel();
        });
        self.run().await;
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

type DeferredRegistration = Box<dyn FnOnce(&mut ActivityRegistry, &str)>;

/// Builder for [`Worker`].
pub struct WorkerBuilder {
    config: Option<WorkerConfig>,
    worker: Option<String>,
    registry: ActivityRegistry,
    /// register_fn calls deferred until the poll worker name is resolved
    deferred: Vec<(String, DeferredRegistration)>,
    executor: Option<Arc<dyn ActivityExecutor>>,
}

impl WorkerBuilder {
    /// Use this configuration instead of loading `KRUXIAFLOW_*` environment
    /// variables at build time.
    pub fn config(mut self, config: WorkerConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the worker type to poll for (the workflow definition's `worker:`
    /// field). Overrides the config; when neither sets it, it is inferred
    /// from registered activities if they all declare the same worker.
    pub fn worker(mut self, worker: impl Into<String>) -> Self {
        self.worker = Some(worker.into());
        self
    }

    /// Register an [`ActivityImpl`] handler.
    pub fn register(mut self, activity: impl ActivityImpl + 'static) -> Self {
        self.registry.register(Arc::new(activity));
        self
    }

    /// Register a [`TypedActivity`] handler.
    pub fn register_typed(mut self, activity: impl TypedActivity + 'static) -> Self {
        self.registry.register_typed(activity);
        self
    }

    /// Register an async closure under the builder's worker type (set
    /// [`worker`](Self::worker) or the config's `worker` field). The
    /// parameter type may be any `serde::Deserialize` struct or
    /// `serde_json::Value`; deserialization failures are reported as
    /// non-retryable `INVALID_PARAMETERS` failures.
    pub fn register_fn<P, F, Fut>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        P: DeserializeOwned + Send + 'static,
        F: Fn(P, ActivityContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ActivityResult, ActivityError>> + Send + 'static,
    {
        let name = name.into();
        self.deferred.push((
            name.clone(),
            Box::new(move |registry, worker| registry.register_fn(worker, name, handler)),
        ));
        self
    }

    /// Use a custom [`ActivityExecutor`] instead of the built-in registry
    /// dispatch. Advanced: for layering caching, streaming, or file staging
    /// between the poll loop and handlers. Requires an explicit worker name.
    pub fn executor(mut self, executor: Arc<dyn ActivityExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Validate and construct the [`Worker`].
    ///
    /// When no config was provided, loads it from `KRUXIAFLOW_*` environment
    /// variables.
    pub fn build(mut self) -> Result<Worker, ConfigError> {
        let mut config = match self.config.take() {
            Some(config) => config,
            None => WorkerConfig::from_env()?,
        };

        if let Some(worker) = self.worker {
            config.worker = worker;
        }
        if config.worker.is_empty() {
            let mut names = self.registry.worker_names();
            match names.len() {
                0 => return Err(ConfigError::MissingWorker),
                1 => config.worker = names.remove(0),
                _ => return Err(ConfigError::AmbiguousWorker(names)),
            }
        }

        let mut registry = self.registry;
        for (name, register) in self.deferred {
            tracing::debug!(worker = %config.worker, name, "Registering deferred activity");
            register(&mut registry, &config.worker);
        }

        let executor: Arc<dyn ActivityExecutor> = match self.executor {
            Some(executor) => executor,
            None => Arc::new(registry),
        };

        let client = match (&config.client_id, &config.client_secret) {
            (Some(id), Some(secret)) => {
                WorkerApiClient::with_credentials(&config.api_url, id, secret)
            }
            _ => WorkerApiClient::new(&config.api_url),
        };

        let poller = WorkerPoller::new(config, client, executor);
        let shutdown = poller.shutdown_token();
        Ok(Worker { poller, shutdown })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn base_config() -> WorkerConfig {
        WorkerConfig {
            api_url: "http://localhost:8080".to_string(),
            ..WorkerConfig::default()
        }
    }

    #[test]
    fn build_infers_worker_from_registrations() {
        let worker = Worker::builder()
            .config(base_config())
            .register_fn("echo", |params: Value, _ctx| async move {
                Ok(ActivityResult::value("echoed", params))
            })
            .worker("demo")
            .build()
            .unwrap();
        drop(worker);
    }

    #[test]
    fn build_without_worker_name_fails() {
        let result = Worker::builder()
            .config(base_config())
            .register_fn("echo", |params: Value, _ctx| async move {
                Ok(ActivityResult::value("echoed", params))
            })
            .build();
        assert!(matches!(result.err(), Some(ConfigError::MissingWorker)));
    }

    #[test]
    fn build_infers_single_worker_from_trait_impls() {
        struct Act;

        #[async_trait::async_trait]
        impl ActivityImpl for Act {
            async fn execute(
                &self,
                _parameters: Value,
                _ctx: &ActivityContext,
            ) -> Result<ActivityResult, ActivityError> {
                Ok(ActivityResult::value("ok", json!(true)))
            }

            fn name(&self) -> &str {
                "act"
            }

            fn worker(&self) -> &str {
                "inferred"
            }
        }

        // Worker name comes from the registered activity
        let worker = Worker::builder()
            .config(base_config())
            .register(Act)
            .build()
            .unwrap();
        drop(worker);
    }

    #[tokio::test]
    async fn handle_shutdown_stops_run() {
        let worker = Worker::builder()
            .config(base_config())
            .worker("demo")
            .register_fn("echo", |params: Value, _ctx| async move {
                Ok(ActivityResult::value("echoed", params))
            })
            .build()
            .unwrap();

        worker.handle().shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(5), worker.run())
            .await
            .expect("run() should return after shutdown");
    }
}
