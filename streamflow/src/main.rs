use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod config;
mod logging;
mod signals;

/// StreamFlow - High-performance workflow orchestration
#[derive(Parser)]
#[command(
    name = "streamflow",
    version,
    about = "StreamFlow workflow orchestration platform",
    long_about = None
)]
struct Cli {
    /// Database connection URL
    #[arg(
        long,
        env = "DATABASE_URL",
        global = true,
        help = "PostgreSQL connection URL (postgres://user:pass@host:port/db)"
    )]
    database_url: Option<String>,

    /// Log level
    #[arg(
        long,
        env = "STREAMFLOW_LOG_LEVEL",
        default_value = "info",
        global = true,
        help = "Log level (trace, debug, info, warn, error)"
    )]
    log_level: String,

    /// Log format
    #[arg(
        long,
        env = "STREAMFLOW_LOG_FORMAT",
        default_value = "text",
        global = true,
        help = "Log format (text, json)"
    )]
    log_format: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch API server
    Api(commands::api::ApiCommand),
    // Future commands (Epic 1C):
    // Serve(commands::serve::ServeCommand),
    // Orchestrator(commands::orchestrator::OrchestratorCommand),
    // Worker(commands::worker::WorkerCommand),
    // Migrate(commands::migrate::MigrateCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    logging::init(&cli.log_level, &cli.log_format)?;

    // Route to command handler
    match cli.command {
        Commands::Api(cmd) => commands::api::execute(cmd, cli.database_url).await,
    }
}
