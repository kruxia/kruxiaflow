//! Activity handler registration and dispatch.

use crate::context::ActivityContext;
use crate::error::ActivityError;
use crate::result::ActivityResult;
use crate::types::PendingActivity;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// An activity handler.
///
/// Activities are matched to workflow definitions by the `(worker, name)`
/// pair — the definition's `worker:` and `name:` fields.
///
/// For handlers with a typed parameter struct, implement [`TypedActivity`]
/// instead; for one-off closures, use
/// [`ActivityRegistry::register_fn`].
#[async_trait]
pub trait ActivityImpl: Send + Sync {
    /// Execute the activity.
    async fn execute(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult, ActivityError>;

    /// Activity name (the workflow definition's `name:` field).
    fn name(&self) -> &str;

    /// Worker type (the workflow definition's `worker:` field).
    fn worker(&self) -> &str;
}

/// An activity handler with a typed, `serde`-deserialized parameter struct.
///
/// The queued activity's JSON parameters are deserialized into
/// [`Self::Params`]; a deserialization failure is reported as a
/// **non-retryable** `INVALID_PARAMETERS` failure (bad input will not improve
/// on retry). Register with [`ActivityRegistry::register_typed`].
///
/// ```
/// use kruxiaflow_worker::{ActivityContext, ActivityError, ActivityResult, TypedActivity};
/// use serde_json::json;
///
/// #[derive(serde::Deserialize)]
/// struct GreetParams {
///     name: String,
/// }
///
/// struct Greet;
///
/// #[async_trait::async_trait]
/// impl TypedActivity for Greet {
///     type Params = GreetParams;
///
///     fn worker(&self) -> &str {
///         "demo"
///     }
///
///     fn name(&self) -> &str {
///         "greet"
///     }
///
///     async fn execute(
///         &self,
///         params: GreetParams,
///         _ctx: &ActivityContext,
///     ) -> Result<ActivityResult, ActivityError> {
///         Ok(ActivityResult::value("greeting", json!(format!("Hello, {}!", params.name))))
///     }
/// }
/// ```
#[async_trait]
pub trait TypedActivity: Send + Sync {
    /// Parameter struct the activity's JSON parameters deserialize into.
    type Params: DeserializeOwned + Send;

    /// Worker type (the workflow definition's `worker:` field).
    fn worker(&self) -> &str;

    /// Activity name (the workflow definition's `name:` field).
    fn name(&self) -> &str;

    /// Execute the activity with deserialized parameters.
    async fn execute(
        &self,
        params: Self::Params,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult, ActivityError>;
}

fn invalid_parameters(key: &str, err: serde_json::Error) -> ActivityError {
    ActivityError::terminal(
        "INVALID_PARAMETERS",
        format!("Failed to deserialize parameters for {key}: {err}"),
    )
}

struct TypedAdapter<T>(T);

#[async_trait]
impl<T: TypedActivity> ActivityImpl for TypedAdapter<T> {
    async fn execute(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult, ActivityError> {
        let params: T::Params = serde_json::from_value(parameters).map_err(|err| {
            invalid_parameters(&format!("{}.{}", self.worker(), self.name()), err)
        })?;
        self.0.execute(params, ctx).await
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn worker(&self) -> &str {
        self.0.worker()
    }
}

type BoxedHandler = Box<
    dyn Fn(
            Value,
            ActivityContext,
        ) -> Pin<Box<dyn Future<Output = Result<ActivityResult, ActivityError>> + Send>>
        + Send
        + Sync,
>;

struct FnActivity {
    worker: String,
    name: String,
    handler: BoxedHandler,
}

#[async_trait]
impl ActivityImpl for FnActivity {
    async fn execute(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> Result<ActivityResult, ActivityError> {
        (self.handler)(parameters, ctx.clone()).await
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn worker(&self) -> &str {
        &self.worker
    }
}

/// Registry of activity handlers, keyed by `worker.name`.
#[derive(Default)]
pub struct ActivityRegistry {
    implementations: HashMap<String, Arc<dyn ActivityImpl>>,
}

impl ActivityRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an [`ActivityImpl`], keyed by its `worker()` and `name()`.
    pub fn register(&mut self, implementation: Arc<dyn ActivityImpl>) {
        let key = format!("{}.{}", implementation.worker(), implementation.name());
        tracing::info!(activity = %key, "Registering activity");
        self.implementations.insert(key, implementation);
    }

    /// Register a [`TypedActivity`].
    pub fn register_typed<T: TypedActivity + 'static>(&mut self, activity: T) {
        self.register(Arc::new(TypedAdapter(activity)));
    }

    /// Register an async closure as an activity handler.
    ///
    /// The parameter type `P` may be any `serde::Deserialize` struct (typed
    /// parameters) or `serde_json::Value` (raw parameters). Deserialization
    /// failures are reported as non-retryable `INVALID_PARAMETERS` failures.
    ///
    /// ```
    /// use kruxiaflow_worker::{ActivityContext, ActivityRegistry, ActivityResult};
    /// use serde_json::json;
    ///
    /// #[derive(serde::Deserialize)]
    /// struct EchoParams {
    ///     message: String,
    /// }
    ///
    /// let mut registry = ActivityRegistry::new();
    /// registry.register_fn("demo", "echo", |params: EchoParams, _ctx: ActivityContext| async move {
    ///     Ok(ActivityResult::value("echoed", json!(params.message)))
    /// });
    /// ```
    pub fn register_fn<P, F, Fut>(
        &mut self,
        worker: impl Into<String>,
        name: impl Into<String>,
        handler: F,
    ) where
        P: DeserializeOwned + Send + 'static,
        F: Fn(P, ActivityContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ActivityResult, ActivityError>> + Send + 'static,
    {
        let worker = worker.into();
        let name = name.into();
        let key = format!("{worker}.{name}");
        let handler: BoxedHandler = Box::new(move |parameters, ctx| {
            let key = key.clone();
            match serde_json::from_value::<P>(parameters) {
                Ok(params) => Box::pin(handler(params, ctx)),
                Err(err) => Box::pin(async move { Err(invalid_parameters(&key, err)) }),
            }
        });
        self.register(Arc::new(FnActivity {
            worker,
            name,
            handler,
        }));
    }

    /// Get a handler by worker and name.
    pub fn get(&self, worker: &str, name: &str) -> Option<&Arc<dyn ActivityImpl>> {
        self.implementations.get(&format!("{worker}.{name}"))
    }

    /// All registered `worker.name` activity types.
    pub fn activity_types(&self) -> Vec<String> {
        self.implementations.keys().cloned().collect()
    }

    /// Distinct worker names across registered activities.
    pub fn worker_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .implementations
            .values()
            .map(|i| i.worker().to_string())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Whether the registry has no handlers.
    pub fn is_empty(&self) -> bool {
        self.implementations.is_empty()
    }
}

/// Executes claimed activities. The poller drives this for every activity it
/// claims.
///
/// [`ActivityRegistry`] implements this by dispatching to registered
/// handlers; implement it directly to put custom machinery (caching,
/// streaming, file staging) between the poll loop and your handlers.
///
/// The poller enforces `timeout` around the whole call and catches panics;
/// implementations receive it for their own bookkeeping (e.g., inner
/// deadlines) and need not enforce it.
#[async_trait]
pub trait ActivityExecutor: Send + Sync {
    /// Execute one claimed activity.
    async fn execute(
        &self,
        activity: &PendingActivity,
        ctx: &ActivityContext,
        timeout: Duration,
    ) -> Result<ActivityResult, ActivityError>;
}

#[async_trait]
impl ActivityExecutor for ActivityRegistry {
    async fn execute(
        &self,
        activity: &PendingActivity,
        ctx: &ActivityContext,
        _timeout: Duration,
    ) -> Result<ActivityResult, ActivityError> {
        let implementation = self
            .get(&activity.worker, &activity.activity_name)
            .ok_or_else(|| {
                ActivityError::retryable(
                    "ACTIVITY_NOT_FOUND",
                    format!(
                        "Activity implementation not found: {}.{}",
                        activity.worker, activity.activity_name
                    ),
                )
            })?;

        implementation
            .execute(activity.parameters.clone(), ctx)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn test_ctx() -> ActivityContext {
        ActivityContext::new(Uuid::now_v7(), Uuid::now_v7(), "step")
    }

    fn pending(worker: &str, name: &str, parameters: Value) -> PendingActivity {
        serde_json::from_value(json!({
            "activity_id": Uuid::now_v7(),
            "workflow_id": Uuid::now_v7(),
            "activity_key": "step",
            "worker": worker,
            "activity_name": name,
            "parameters": parameters,
        }))
        .unwrap()
    }

    #[derive(serde::Deserialize)]
    struct EchoParams {
        message: String,
    }

    #[tokio::test]
    async fn register_fn_typed_params() {
        let mut registry = ActivityRegistry::new();
        registry.register_fn("demo", "echo", |params: EchoParams, _ctx| async move {
            Ok(ActivityResult::value("echoed", json!(params.message)))
        });

        let result = ActivityExecutor::execute(
            &registry,
            &pending("demo", "echo", json!({"message": "hi"})),
            &test_ctx(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(result.get_output("echoed").unwrap().value, json!("hi"));
    }

    #[tokio::test]
    async fn register_fn_bad_params_is_terminal() {
        let mut registry = ActivityRegistry::new();
        registry.register_fn("demo", "echo", |params: EchoParams, _ctx| async move {
            Ok(ActivityResult::value("echoed", json!(params.message)))
        });

        let err = ActivityExecutor::execute(
            &registry,
            &pending("demo", "echo", json!({"wrong_field": 1})),
            &test_ctx(),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID_PARAMETERS");
        assert!(!err.retryable);
    }

    #[tokio::test]
    async fn register_fn_raw_value_params() {
        let mut registry = ActivityRegistry::new();
        registry.register_fn("demo", "raw", |params: Value, _ctx| async move {
            Ok(ActivityResult::value("raw", params))
        });

        let result = ActivityExecutor::execute(
            &registry,
            &pending("demo", "raw", json!([1, 2, 3])),
            &test_ctx(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(result.get_output("raw").unwrap().value, json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn missing_activity_is_retryable() {
        let registry = ActivityRegistry::new();
        let err = ActivityExecutor::execute(
            &registry,
            &pending("demo", "nope", json!({})),
            &test_ctx(),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "ACTIVITY_NOT_FOUND");
        assert!(err.retryable);
    }

    struct Greet;

    #[async_trait]
    impl TypedActivity for Greet {
        type Params = EchoParams;

        fn worker(&self) -> &str {
            "demo"
        }

        fn name(&self) -> &str {
            "greet"
        }

        async fn execute(
            &self,
            params: EchoParams,
            _ctx: &ActivityContext,
        ) -> Result<ActivityResult, ActivityError> {
            Ok(ActivityResult::value(
                "greeting",
                json!(format!("Hello, {}!", params.message)),
            ))
        }
    }

    #[tokio::test]
    async fn register_typed_trait() {
        let mut registry = ActivityRegistry::new();
        registry.register_typed(Greet);
        assert_eq!(registry.activity_types(), vec!["demo.greet".to_string()]);
        assert_eq!(registry.worker_names(), vec!["demo".to_string()]);

        let result = ActivityExecutor::execute(
            &registry,
            &pending("demo", "greet", json!({"message": "world"})),
            &test_ctx(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(
            result.get_output("greeting").unwrap().value,
            json!("Hello, world!")
        );
    }
}
