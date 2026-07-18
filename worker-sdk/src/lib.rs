//! Worker SDK for [Kruxia Flow](https://kruxiaflow.com) — run Rust
//! activities in budgeted workflows with engine-enforced cost limits.
//!
//! A Kruxia Flow *worker* polls the server for queued activities, executes
//! registered handlers, and reports results (including LLM usage, so
//! external activities count against workflow budgets with full fidelity).
//! This crate provides the complete worker loop: registration, polling,
//! bounded concurrency, heartbeats, timeout and panic containment,
//! completion/failure reporting, OAuth2 client-credentials auth, and
//! graceful drain on shutdown.
//!
//! # Quickstart
//!
//! Run a server locally (no auth needed in dev mode):
//!
//! ```text
//! kruxiaflow serve --insecure-dev
//! ```
//!
//! Then a minimal worker:
//!
//! ```no_run
//! use kruxiaflow_worker::{ActivityContext, ActivityResult, Worker};
//! use serde_json::json;
//!
//! #[derive(serde::Deserialize)]
//! struct EchoParams {
//!     message: String,
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let worker = Worker::builder()
//!         .worker("demo") // the workflow definition's `worker:` field
//!         .register_fn("echo", |params: EchoParams, _ctx: ActivityContext| async move {
//!             Ok(ActivityResult::value("echoed", json!(params.message)))
//!         })
//!         .build()?; // config from KRUXIAFLOW_* environment variables
//!
//!     worker.run_until_shutdown().await;
//!     Ok(())
//! }
//! ```
//!
//! A workflow activity with `worker: demo` and `name: echo` now executes on
//! this worker.
//!
//! # Reporting cost and usage
//!
//! Activities that call LLMs themselves report per-call usage so the engine
//! records it exactly like built-in LLM activities — visible in cost
//! history/analytics and counted against workflow budgets:
//!
//! ```no_run
//! use kruxiaflow_worker::{ActivityResult, UsageEntry};
//! use serde_json::json;
//!
//! # fn example() -> ActivityResult {
//! ActivityResult::value("summary", json!("..."))
//!     .push_usage(
//!         UsageEntry::new("anthropic", "claude-sonnet-5")
//!             .input_tokens(12034)
//!             .output_tokens(512)
//!             .cache_read_tokens(9800),
//!     )
//! # }
//! ```
//!
//! The server prices entries from its model catalog unless an entry carries
//! an explicit `cost_usd`. Failures spend money too: attach usage to
//! [`ActivityError`] so failed attempts are budget-counted.
//!
//! # Failure semantics
//!
//! Handlers return `Result<ActivityResult, ActivityError>`. Construct errors
//! with [`ActivityError::retryable`] (transient: the orchestrator re-queues
//! up to the activity's retry limit) or [`ActivityError::terminal`] (bad
//! input, business rejection: no retry). Panics and timeouts are caught and
//! reported as retryable; parameter deserialization failures are terminal.
//!
//! # Shutdown
//!
//! [`Worker::run_until_shutdown`] listens for SIGINT/SIGTERM;
//! [`Worker::handle`] gives a programmatic trigger. Shutdown stops polling
//! and drains in-flight activities up to the configured `shutdown_timeout`,
//! then fails the remainder as retryable so they re-queue — nothing is lost
//! or double-completed.

pub mod client;
pub mod config;
pub mod context;
pub mod error;
pub mod poller;
pub mod registry;
pub mod result;
pub mod types;
pub mod worker;

pub use client::{ReportAck, WorkerApiClient};
pub use config::{WorkerConfig, WorkerConfigBuilder};
pub use context::ActivityContext;
pub use error::{ActivityError, ClientError, ConfigError};
pub use poller::WorkerPoller;
pub use registry::{ActivityExecutor, ActivityImpl, ActivityRegistry, TypedActivity};
pub use result::ActivityResult;
pub use types::{ActivityOutput, OutputType, PendingActivity, PollActivitiesResponse, UsageEntry};
pub use worker::{Worker, WorkerBuilder, WorkerHandle};

// Implementing [`ActivityImpl`]/[`TypedActivity`] requires the same
// `async_trait` attribute the traits are declared with; re-exported so
// consumers don't need the direct dependency.
pub use async_trait::async_trait;
