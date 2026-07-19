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

// MCP server module (optional - only compiled with mcp-server feature)
#[cfg(feature = "mcp-server")]
mod mcp;

#[cfg(all(feature = "profiling", not(target_env = "msvc")))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(all(feature = "profiling", not(target_env = "msvc")))]
#[unsafe(export_name = "_rjem_malloc_conf")]
#[allow(non_upper_case_globals)]
pub static _rjem_malloc_conf: &[u8] = b"prof:true\0";

/// Kruxia Flow - High-performance workflow orchestration
#[derive(Parser)]
#[command(
    name = "kruxiaflow",
    version,
    about = "Kruxia Flow workflow orchestration platform",
    long_about = "Kruxia Flow is a lightweight, high-performance workflow orchestration \
platform designed for edge-to-cloud deployment. Built as a single binary \
with PostgreSQL as the only required dependency.\n\n\
EXAMPLES:\n  \
  kruxiaflow api --port 8080\n  \
  kruxiaflow version --format json\n  \
  kruxiaflow --help\n\n\
ENVIRONMENT VARIABLES:\n  \
  DATABASE_URL               PostgreSQL connection string (required for most commands)\n  \
  KRUXIAFLOW_LOG_LEVEL       Logging verbosity (default: info)\n  \
  KRUXIAFLOW_LOG_FORMAT      Log output format (default: text)\n  \
  KRUXIAFLOW_API_PORT        API server port (default: 8080)\n  \
  KRUXIAFLOW_API_BIND        API server bind address (default: 0.0.0.0)"
)]
struct Cli {
    /// Database connection URL
    #[arg(
        long,
        env = "DATABASE_URL",
        global = true,
        help = "PostgreSQL connection URL (postgres://user:pass@host:port/db)",
        long_help = "PostgreSQL connection URL\n\n\
Example: postgres://user:pass@localhost:5432/kruxiaflow\n\
Required for all commands except 'version'"
    )]
    database_url: Option<String>,

    /// Log level
    #[arg(
        long,
        env = "KRUXIAFLOW_LOG_LEVEL",
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
        env = "KRUXIAFLOW_LOG_FORMAT",
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
  kruxiaflow api\n  \
  kruxiaflow api --port 9090 --bind 127.0.0.1\n  \
  DATABASE_URL=postgres://localhost/db kruxiaflow api\n\n\
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
        long_about = "Launch all Kruxia Flow services in a single process\n\n\
This is the recommended mode for development, testing, and single-node production.\n\n\
EXAMPLES:\n  \
  kruxiaflow serve\n  \
  kruxiaflow serve --port 8080 --workers 4\n  \
  kruxiaflow serve --bind 127.0.0.1 --workers 2\n\n\
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
Shows Kruxia Flow version, build timestamp, git commit, and platform details.\n\n\
EXAMPLES:\n  \
  kruxiaflow version\n  \
  kruxiaflow version --format json"
    )]
    Version(commands::version::VersionCommand),

    /// Seed LLM model catalog from YAML
    #[command(
        about = "Load LLM model catalog and pricing from YAML file",
        long_about = "Load LLM model catalog and pricing from YAML file\n\n\
Loads provider and model information into the database from a YAML configuration file.\n\
Supports upsert - updates existing providers/models with new pricing.\n\n\
EXAMPLES:\n  \
  kruxiaflow seed-llm config/llm_models.yaml\n  \
  kruxiaflow seed-llm /path/to/custom_models.yaml"
    )]
    SeedLlm(commands::seed_llm::SeedLlmCommand),

    /// Database migration management
    #[command(
        about = "Run database migrations",
        long_about = "Manage database migrations with embedded SQL files\n\n\
Migrations are embedded at compile time and can be run, previewed, or status-checked.\n\n\
EXAMPLES:\n  \
  kruxiaflow migrate              # Run pending migrations\n  \
  kruxiaflow migrate --status     # Show migration status\n  \
  kruxiaflow migrate --dry-run    # Preview without applying"
    )]
    Migrate(commands::migrate::MigrateCommand),

    /// Seed OAuth client credentials
    #[command(
        name = "seed-client",
        about = "Seed OAuth client credentials in database",
        long_about = "Seed OAuth client credentials for authentication\n\n\
By default, skips seeding if the client already exists (idempotent).\n\
Use --force to delete and re-create an existing client.\n\n\
EXAMPLES:\n  \
  kruxiaflow seed-client                              # Seed with env vars\n  \
  kruxiaflow seed-client --client-id my-app           # Override client ID\n  \
  kruxiaflow seed-client --force                      # Re-seed even if exists"
    )]
    SeedClient(commands::seed_client::SeedClientCommand),

    /// Check service health
    #[command(
        about = "Check health of Kruxia Flow services",
        long_about = "Check health of Kruxia Flow services\n\n\
Performs health checks on database, API server, and orchestrator.\n\
Exit code: 0 (healthy), 1 (unhealthy).\n\n\
EXAMPLES:\n  \
  kruxiaflow health                    # Check all services\n  \
  kruxiaflow health --service api      # Check API only\n  \
  kruxiaflow health --format json      # JSON output\n  \
  kruxiaflow health --timeout 10       # 10 second timeout\n\n\
USE IN SCRIPTS:\n  \
  if kruxiaflow health; then\n  \
    echo 'Kruxia Flow is healthy'\n  \
  fi"
    )]
    Health(commands::health::HealthCommand),

    /// Show detailed service status
    #[command(
        about = "Show detailed status of Kruxia Flow services",
        long_about = "Show detailed status of Kruxia Flow services\n\n\
Displays version, uptime, and configuration for all services.\n\n\
EXAMPLES:\n  \
  kruxiaflow status                # Show status\n  \
  kruxiaflow status --format json  # JSON output"
    )]
    Status(commands::status::StatusCommand),

    /// Cost reporting and analytics
    #[command(
        about = "Cost reporting and analytics (wraps the REST cost API)",
        long_about = "Cost reporting and analytics via the REST API\n\n\
Answers: what did we spend, on which providers/models/definitions, which\n\
workflows were most expensive, and did anything hit its budget?\n\n\
Authenticates with OAuth2 client credentials; no credentials are needed\n\
against a server running with --insecure-dev.\n\n\
EXAMPLES:\n  \
  kruxiaflow cost workflow <workflow_id>\n  \
  kruxiaflow cost workflow <workflow_id> --detailed\n  \
  kruxiaflow cost analytics --since 7d --group-by provider\n  \
  kruxiaflow cost top --limit 10 --since 30d\n  \
  kruxiaflow cost export --since 30d --output costs.csv\n\n\
REQUIRES:\n  \
  - KRUXIAFLOW_API_URL: API server URL (default http://127.0.0.1:8080)\n  \
  - KRUXIAFLOW_CLIENT_ID / KRUXIAFLOW_CLIENT_SECRET: OAuth2 credentials\n    \
(omit both against --insecure-dev servers)"
    )]
    Cost(commands::cost::CostCommand),

    /// Launch orchestrator only (for distributed deployment)
    #[command(
        about = "Launch orchestrator service for distributed deployment",
        long_about = "Launch the orchestrator service independently\n\n\
The orchestrator polls for workflow events and schedules activities.\n\
Use this for distributed deployments where services run on separate hosts.\n\n\
EXAMPLES:\n  \
  kruxiaflow orchestrator\n  \
  kruxiaflow orchestrator --consumer-id orch_prod_1\n\n\
REQUIRES:\n  \
  - DATABASE_URL: PostgreSQL connection string"
    )]
    Orchestrator(commands::orchestrator::OrchestratorCommand),

    /// Launch worker only (for distributed deployment)
    #[command(
        about = "Launch worker service for distributed deployment",
        long_about = "Launch the built-in worker service independently\n\n\
Workers poll the API server for activities and execute them.\n\
Use this for distributed deployments or to scale workers.\n\n\
EXAMPLES:\n  \
  kruxiaflow worker --api-url http://api.example.com:8080\n  \
  kruxiaflow worker --workers 20 --worker-id worker_payments_1\n\n\
REQUIRES:\n  \
  - KRUXIAFLOW_API_URL: API server URL\n  \
  - KRUXIAFLOW_CLIENT_SECRET: OAuth client secret\n  \
  - DATABASE_URL: For artifact storage access"
    )]
    Worker(commands::worker::WorkerCommand),

    /// PostgreSQL performance profiling
    #[command(
        about = "Profile PostgreSQL query performance",
        long_about = "Profile PostgreSQL query performance\n\n\
Queries pg_stat_statements and system views to analyze database performance.\n\
Shows slow queries, index usage, table statistics, and lock contention.\n\n\
EXAMPLES:\n  \
  kruxiaflow profile                    # Full profiling report\n  \
  kruxiaflow profile --explain          # Include EXPLAIN ANALYZE\n  \
  kruxiaflow profile --reset            # Reset query statistics\n  \
  kruxiaflow profile --format json      # JSON output\n  \
  kruxiaflow profile -v                 # Verbose with table stats\n\n\
REQUIRES:\n  \
  - DATABASE_URL: PostgreSQL connection string\n  \
  - pg_stat_statements extension (for query stats)"
    )]
    Profile(commands::profile::ProfileCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging (skip for client commands whose stdout is the
    // deliverable — version info, cost tables/JSON/CSV)
    if !matches!(cli.command, Commands::Version(_) | Commands::Cost(_)) {
        logging::init(&cli.log_level, &cli.log_format)?;
    }

    // Validate database_url for commands that need it
    // Health and Status only use the API server (no direct database access)
    let database_url = match &cli.command {
        Commands::Version(_) | Commands::Health(_) | Commands::Status(_) | Commands::Cost(_) => {
            None
        }
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
        Commands::Migrate(cmd) => commands::migrate::execute(cmd, database_url.unwrap()).await,
        Commands::SeedClient(cmd) => {
            commands::seed_client::execute(cmd, database_url.unwrap()).await
        }
        Commands::Health(cmd) => commands::health::execute(cmd).await,
        Commands::Status(cmd) => commands::status::execute(cmd).await,
        Commands::Cost(cmd) => commands::cost::execute(cmd).await,
        Commands::Orchestrator(cmd) => {
            commands::orchestrator::execute(cmd, database_url.unwrap()).await
        }
        Commands::Worker(cmd) => commands::worker::execute(cmd, database_url.unwrap()).await,
        Commands::Profile(cmd) => commands::profile::execute(cmd, database_url.unwrap()).await,
    }
}
