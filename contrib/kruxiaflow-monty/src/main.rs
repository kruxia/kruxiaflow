//! `kruxiaflow-monty`: the add-on worker that runs Kruxia Flow's sandboxed
//! `run_code` steps.
//!
//! ```text
//! kruxiaflow-monty            # run the worker
//! kruxiaflow-monty contract   # print the contract for the engine's MCP server
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use kruxiaflow_monty::cli::{Cli, Command};
use kruxiaflow_monty::{RunCodeActivity, contract};
use kruxiaflow_worker::{Worker, WorkerConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Run) {
        Command::Contract => {
            let config = cli.monty.to_config(1, false)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&contract(&config, &cli.monty.worker))?
            );
            Ok(())
        }
        Command::Run => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .init();

            // The server connection and concurrency come from the SDK's
            // KRUXIAFLOW_* variables; the pool defaults to that concurrency
            let mut worker_config =
                WorkerConfig::from_env().context("worker configuration (KRUXIAFLOW_*)")?;
            worker_config.worker = cli.monty.worker.clone();
            let config = cli
                .monty
                .to_config(worker_config.max_concurrent_activities, true)?;

            // Checks the configuration and the binary's version, so a
            // misconfigured worker refuses to start instead of failing every
            // run_code step later. The pool itself starts on first use.
            let activity = RunCodeActivity::new(config, &cli.monty.worker).await?;
            tracing::info!(
                worker = %cli.monty.worker,
                capabilities = ?activity.capability_names(),
                "run_code worker starting (Monty sandbox)"
            );

            let worker = Worker::builder()
                .config(worker_config)
                .register(activity)
                .build()?;
            worker.run_until_shutdown().await;
            Ok(())
        }
    }
}
