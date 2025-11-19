use anyhow::Result;
use clap::{Parser, Subcommand};

// Use jemalloc for memory profiling support
#[cfg(all(feature = "profiling", not(target_env = "msvc")))]
use tikv_jemallocator::Jemalloc;

mod commands;
mod config;
mod llm_catalog;
mod logging;
mod signals;

#[cfg(all(feature = "profiling", not(target_env = "msvc")))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(all(feature = "profiling", not(target_env = "msvc")))]
#[unsafe(export_name = "_rjem_malloc_conf")]
#[allow(non_upper_case_globals)]
pub static _rjem_malloc_conf: &[u8] = b"prof:true\0";

/// StreamFlow - High-performance workflow orchestration
#[derive(Parser)]
#[command(
    name = "streamflow",
    version,
    about = "StreamFlow workflow orchestration platform",
    long_about = "StreamFlow is a lightweight, high-performance workflow orchestration \
platform designed for edge-to-cloud deployment. Built as a single binary \
with PostgreSQL as the only required dependency.\n\n\
EXAMPLES:\n  \
  streamflow api --port 8080\n  \
  streamflow version --format json\n  \
  streamflow --help\n\n\
ENVIRONMENT VARIABLES:\n  \
  DATABASE_URL               PostgreSQL connection string (required for most commands)\n  \
  STREAMFLOW_LOG_LEVEL       Logging verbosity (default: info)\n  \
  STREAMFLOW_LOG_FORMAT      Log output format (default: text)\n  \
  STREAMFLOW_API_PORT        API server port (default: 8080)\n  \
  STREAMFLOW_API_BIND        API server bind address (default: 0.0.0.0)"
)]
struct Cli {
    /// Database connection URL
    #[arg(
        long,
        env = "DATABASE_URL",
        global = true,
        help = "PostgreSQL connection URL (postgres://user:pass@host:port/db)",
        long_help = "PostgreSQL connection URL\n\n\
Example: postgres://user:pass@localhost:5432/streamflow\n\
Required for all commands except 'version'"
    )]
    database_url: Option<String>,

    /// Log level
    #[arg(
        long,
        env = "STREAMFLOW_LOG_LEVEL",
        default_value = "info",
        global = true,
        help = "Log level (trace, debug, info, warn, error)",
        long_help = "Log level for structured logging\n\n\
Options: trace, debug, info, warn, error\n\
Default: info\n\
Example: --log-level debug"
    )]
    log_level: String,

    /// Log format
    #[arg(
        long,
        env = "STREAMFLOW_LOG_FORMAT",
        default_value = "text",
        global = true,
        help = "Log format (text, json)",
        long_help = "Log output format\n\n\
Options: text (human-readable), json (machine-readable)\n\
Default: text\n\
Example: --log-format json for production logging"
    )]
    log_format: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch API server
    #[command(
        about = "Launch the API server on the specified port",
        long_about = "Launch the HTTP API server\n\n\
The API server provides RESTful endpoints for workflow management, \
authentication, and monitoring.\n\n\
EXAMPLES:\n  \
  streamflow api\n  \
  streamflow api --port 9090 --bind 127.0.0.1\n  \
  DATABASE_URL=postgres://localhost/db streamflow api\n\n\
ENDPOINTS:\n  \
  GET  /health              - Liveness probe\n  \
  GET  /health/ready        - Readiness probe\n  \
  GET  /api/v1/info         - Service information\n  \
  POST /api/v1/auth/token   - Authentication\n  \
  See /api/v1/openapi.json for full API documentation"
    )]
    Api(commands::api::ApiCommand),

    /// Launch all services together
    #[command(
        about = "Launch orchestrator, API server, and workers together",
        long_about = "Launch all StreamFlow services in a single process\n\n\
This is the recommended mode for development, testing, and single-node production.\n\n\
EXAMPLES:\n  \
  streamflow serve\n  \
  streamflow serve --port 8080 --workers 4\n  \
  streamflow serve --bind 127.0.0.1 --workers 2\n\n\
SERVICES STARTED:\n  \
  - Orchestrator: Evaluates workflows and schedules activities\n  \
  - API Server: HTTP/REST endpoints\n  \
  - Workers: Built-in activity execution (configurable count)"
    )]
    Serve(commands::serve::ServeCommand),

    /// Show version information
    #[command(
        about = "Display version and build information",
        long_about = "Display version and build information\n\n\
Shows StreamFlow version, build timestamp, git commit, and platform details.\n\n\
EXAMPLES:\n  \
  streamflow version\n  \
  streamflow version --format json"
    )]
    Version(commands::version::VersionCommand),

    /// Seed LLM model catalog from YAML
    #[command(
        about = "Load LLM model catalog and pricing from YAML file",
        long_about = "Load LLM model catalog and pricing from YAML file\n\n\
Loads provider and model information into the database from a YAML configuration file.\n\
Supports upsert - updates existing providers/models with new pricing.\n\n\
EXAMPLES:\n  \
  streamflow seed-llm config/llm_models.yaml\n  \
  streamflow seed-llm /path/to/custom_models.yaml"
    )]
    SeedLlm(commands::seed_llm::SeedLlmCommand),
    // Future commands (Epic 1C):
    // Orchestrator(commands::orchestrator::OrchestratorCommand),
    // Worker(commands::worker::WorkerCommand),
    // Migrate(commands::migrate::MigrateCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging (skip for version command)
    if !matches!(cli.command, Commands::Version(_)) {
        logging::init(&cli.log_level, &cli.log_format)?;
    }

    // Validate database_url for commands that need it
    let database_url = match &cli.command {
        Commands::Version(_) => None,
        _ => {
            let url = cli.database_url.ok_or_else(|| {
                anyhow::anyhow!(
                    "Database URL is required\n\n\
                    Set via:\n  \
                      --database-url postgres://user:pass@host:port/db\n  \
                      export DATABASE_URL=postgres://user:pass@host:port/db"
                )
            })?;
            Some(url)
        }
    };

    // Route to command handler
    match cli.command {
        Commands::Api(cmd) => commands::api::execute(cmd, database_url.clone()).await,
        Commands::Serve(cmd) => commands::serve::execute(cmd, database_url.unwrap()).await,
        Commands::Version(cmd) => commands::version::execute(cmd),
        Commands::SeedLlm(cmd) => commands::seed_llm::execute(cmd, database_url.unwrap()).await,
    }
}
