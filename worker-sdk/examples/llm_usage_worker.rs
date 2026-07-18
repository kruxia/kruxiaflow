//! Reporting LLM usage from a custom activity.
//!
//! An activity that calls an LLM itself attaches one [`UsageEntry`] per
//! call. The server prices entries from its model catalog and records them
//! with the same fidelity as built-in LLM activities: they appear in
//! `/cost/history` and `/cost/analytics`, and count against the workflow's
//! budget. This is what keeps external activities from being a budget
//! bypass.
//!
//! ```sh
//! kruxiaflow serve --insecure-dev
//! KRUXIAFLOW_API_URL=http://localhost:8080 cargo run --example llm_usage_worker
//! ```

use kruxiaflow_worker::{ActivityContext, ActivityError, ActivityResult, UsageEntry, Worker};
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;

#[derive(serde::Deserialize)]
struct SummarizeParams {
    text: String,
}

/// Stand-in for a real LLM client call; returns (completion, usage).
async fn call_llm(prompt: &str) -> Result<(String, UsageEntry), ActivityError> {
    // A real implementation calls its provider SDK here and copies the token
    // counts from the provider's response.
    let usage = UsageEntry::new("anthropic", "claude-sonnet-5")
        .input_tokens(prompt.len() as u32 / 4)
        .output_tokens(128);
    Ok((format!("summary of {} chars", prompt.len()), usage))
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
            "summarize",
            |params: SummarizeParams, _ctx: ActivityContext| async move {
                let (summary, usage) = call_llm(&params.text).await?;

                Ok(ActivityResult::value("summary", json!(summary))
                    // One entry per LLM call; the server computes the cost
                    // from its pricing catalog.
                    .push_usage(usage)
                    // Spend NOT covered by usage entries (a paid non-LLM
                    // API, time-based cache storage, ...). Never repeat
                    // entry costs here.
                    .with_cost(Decimal::from_str("0.0004").unwrap()))
            },
        )
        .build()?;

    worker.run_until_shutdown().await;
    Ok(())
}
