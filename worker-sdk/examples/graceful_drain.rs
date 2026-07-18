//! Graceful shutdown: drain in-flight activities, re-queue the rest.
//!
//! Start this worker, submit a workflow using the `slow` activity, then hit
//! Ctrl-C (or send SIGTERM, as a Kubernetes rolling deploy does) while it is
//! executing:
//!
//! - polling stops immediately;
//! - the in-flight `slow` activity gets up to `shutdown_timeout` (10s here)
//!   to finish and report;
//! - anything still running at the deadline is failed as retryable
//!   (`WORKER_SHUTDOWN`) so the server re-queues it for another worker —
//!   nothing is lost or double-completed.
//!
//! ```sh
//! kruxiaflow serve --insecure-dev
//! KRUXIAFLOW_API_URL=http://localhost:8080 cargo run --example graceful_drain
//! ```

use kruxiaflow_worker::{ActivityContext, ActivityResult, Worker, WorkerConfig};
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = WorkerConfig {
        shutdown_timeout: Duration::from_secs(10),
        ..WorkerConfig::from_env()?
    };

    let worker = Worker::builder()
        .config(config)
        .worker("demo")
        .register_fn("slow", |_params: Value, ctx: ActivityContext| async move {
            for i in 1..=8 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                tracing::info!(step = i, activity_id = %ctx.activity_id, "still working");
            }
            Ok(ActivityResult::value("finished", json!(true)))
        })
        .build()?;

    tracing::info!("Running; Ctrl-C during a `slow` activity to watch the drain");
    worker.run_until_shutdown().await;
    Ok(())
}
