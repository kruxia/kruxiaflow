//! Minimal worker: one typed activity handler.
//!
//! Run a server in dev mode, then this worker:
//!
//! ```sh
//! kruxiaflow serve --insecure-dev
//! KRUXIAFLOW_API_URL=http://localhost:8080 cargo run --example echo_worker
//! ```
//!
//! Any workflow activity with `worker: demo` / `name: echo` executes here:
//!
//! ```yaml
//! name: hello-sdk
//! activities:
//!   greet:
//!     worker: demo
//!     name: echo
//!     parameters:
//!       message: "hello from Rust"
//! ```

use kruxiaflow_worker::{ActivityContext, ActivityResult, Worker};
use serde_json::json;

#[derive(serde::Deserialize)]
struct EchoParams {
    message: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let worker = Worker::builder()
        .worker("demo")
        .register_fn(
            "echo",
            |params: EchoParams, _ctx: ActivityContext| async move {
                Ok(ActivityResult::value("echoed", json!(params.message)))
            },
        )
        .build()?;

    worker.run_until_shutdown().await;
    Ok(())
}
