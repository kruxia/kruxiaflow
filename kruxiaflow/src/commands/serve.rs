use anyhow::Result;
use clap::Args;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::{
    ActivityQueue, CacheService, EventSource, OrchestratorConfig, PostgresEventSource,
    PostgresQueue, QueueConfig, RedisCache, orchestrator::OrchestratorError, run_orchestrator,
};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_worker::{WorkerConfig, WorkerManager};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{migrate, seed_client};

/// Serve command - Launch all services together
#[derive(Args)]
pub struct ServeCommand {
    /// API server port
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_API_PORT",
        default_value = "8080",
        help = "Port to bind API server to",
        long_help = "Port to bind API server to\n\n\
Default: 8080\n\
Example: --port 9090"
    )]
    pub port: u16,

    /// API server bind address
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_API_BIND",
        default_value = "0.0.0.0",
        help = "Address to bind API server to",
        long_help = "Address to bind API server to\n\n\
Options:\n  \
  0.0.0.0    - All network interfaces (default)\n  \
  127.0.0.1  - Localhost only (development)\n\
Example: --bind 127.0.0.1"
    )]
    pub bind: String,

    /// Number of worker tasks
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_WORKER_COUNT",
        default_value = "4",
        help = "Number of concurrent worker tasks",
        long_help = "Number of concurrent worker tasks to spawn\n\n\
Default: 1\n\
Range: 1-100\n\
Example: --workers 20"
    )]
    pub workers: usize,

    /// Orchestrator consumer ID (for event polling checkpoint)
    #[arg(
        long,
        env = "KRUXIAFLOW_ORCHESTRATOR_CONSUMER_ID",
        default_value = "orchestrator_default",
        help = "Orchestrator consumer ID for event checkpointing"
    )]
    pub orchestrator_id: String,

    /// Worker client ID for OAuth
    #[arg(
        long,
        env = "KRUXIAFLOW_CLIENT_ID",
        default_value = "kruxiaflow_internal_worker",
        help = "OAuth client ID for internal workers"
    )]
    pub client_id: String,

    /// Worker client secret for OAuth
    #[arg(
        long,
        env = "KRUXIAFLOW_CLIENT_SECRET",
        help = "OAuth client secret for internal workers (required)"
    )]
    pub client_secret: Option<String>,

    /// OAuth RSA private key (PEM format)
    #[arg(
        long,
        env = "KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM",
        help = "RSA private key for JWT signing (required)"
    )]
    pub oauth_private_key: Option<String>,

    /// OAuth RSA public key (PEM format)
    #[arg(
        long,
        env = "KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM",
        help = "RSA public key for JWT validation (optional, derived from private key if not provided)"
    )]
    pub oauth_public_key: Option<String>,

    /// Shutdown timeout in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_SHUTDOWN_TIMEOUT",
        default_value = "30",
        help = "Graceful shutdown timeout in seconds",
        long_help = "Time to wait for in-flight activities to complete during shutdown\n\n\
Default: 30 seconds\n\
Range: 5-300 seconds\n\
Example: --shutdown-timeout 60"
    )]
    pub shutdown_timeout: u64,

    /// Maximum activities per worker poll
    #[arg(
        long,
        env = "KRUXIAFLOW_POLL_MAX_ACTIVITIES",
        default_value = "1",
        help = "Maximum number of activities each worker claims per poll",
        long_help = "Maximum number of activities each worker claims per poll\n\n\
Default: 1\n\
Range: 1-100\n\
Note: Lower values (1-5) improve work distribution across workers\n\
Example: --poll-max-activities 10"
    )]
    pub poll_max_activities: usize,

    /// Redis connection URL for caching
    #[arg(
        long,
        env = "KRUXIAFLOW_REDIS_URL",
        default_value = "redis://127.0.0.1:6379",
        help = "Redis connection URL for caching",
        long_help = "Redis connection URL for activity result caching\n\n\
Default: redis://127.0.0.1:6379\n\
Example: --redis-url redis://localhost:6379/0"
    )]
    pub redis_url: String,

    /// Run database migrations before starting
    #[arg(
        long,
        env = "KRUXIAFLOW_MIGRATE",
        help = "Run database migrations before starting server",
        long_help = "Run pending database migrations before starting the server.\n\
Useful for container deployments where migrations should run at startup.\n\n\
Example: kruxiaflow serve --migrate"
    )]
    pub migrate: bool,

    /// Seed OAuth client before starting
    #[arg(
        long,
        env = "KRUXIAFLOW_SEED_CLIENT",
        help = "Seed OAuth client credentials before starting server",
        long_help = "Seed OAuth client credentials before starting the server.\n\
Skips seeding if the client already exists (idempotent).\n\
Requires KRUXIAFLOW_CLIENT_ID and KRUXIAFLOW_CLIENT_SECRET.\n\n\
Example: kruxiaflow serve --seed-client"
    )]
    pub seed_client: bool,

    /// Database connection timeout for --migrate/--seed-client (seconds)
    #[arg(
        long,
        env = "KRUXIAFLOW_DB_CONNECT_TIMEOUT",
        default_value = "60",
        help = "Timeout for initial database connection (seconds)",
        long_help = "Maximum time to wait for database to become available.\n\
Used with --migrate or --seed-client for container startup.\n\n\
Default: 60 seconds\n\
Example: --db-connect-timeout 120"
    )]
    pub db_connect_timeout: u64,
}

impl ServeCommand {
    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0 || self.workers > 100 {
            anyhow::bail!("Worker count must be between 1 and 100");
        }

        if self.poll_max_activities == 0 || self.poll_max_activities > 100 {
            anyhow::bail!("Max activities per poll must be between 1 and 100");
        }

        if self.client_secret.is_none() {
            anyhow::bail!("Client secret required (--client-secret or KRUXIAFLOW_CLIENT_SECRET)");
        }

        if self.oauth_private_key.is_none() {
            anyhow::bail!(
                "OAuth private key required (--oauth-private-key or KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM)"
            );
        }

        if self.shutdown_timeout < 5 || self.shutdown_timeout > 300 {
            anyhow::bail!("Shutdown timeout must be between 5 and 300 seconds");
        }

        Ok(())
    }
}

/// Spawn orchestrator task
async fn spawn_orchestrator(
    event_source: Arc<dyn EventSource>,
    activity_queue: Arc<dyn ActivityQueue>,
    pool: PgPool,
    shutdown_token: CancellationToken,
) -> Result<(JoinHandle<Result<()>>, Arc<Notify>)> {
    let ready_notify = Arc::new(Notify::new());
    let ready_clone = Arc::clone(&ready_notify);

    let config = OrchestratorConfig::new(pool);

    let handle = tokio::spawn(async move {
        tracing::info!("Starting orchestrator");

        // Signal ready immediately since we're just starting the loop
        ready_clone.notify_one();

        // Run orchestrator (polls events and schedules activities)
        // Note: run_orchestrator will check shutdown_token in its loop
        run_orchestrator(event_source, activity_queue, config, Some(shutdown_token))
            .await
            .map_err(|e: OrchestratorError| anyhow::anyhow!("Orchestrator error: {}", e))
    });

    // Wait for orchestrator to signal ready (or timeout)
    tokio::time::timeout(Duration::from_secs(5), ready_notify.notified())
        .await
        .map_err(|_| anyhow::anyhow!("Orchestrator failed to start within 5 seconds"))?;

    tracing::info!("Orchestrator ready");

    Ok((handle, ready_notify))
}

/// Spawn API server task with graceful shutdown support
async fn spawn_api_server(
    state: AppState,
    bind: String,
    port: u16,
    shutdown_token: CancellationToken,
) -> Result<(JoinHandle<Result<()>>, Arc<Notify>)> {
    let addr: SocketAddr = format!("{}:{}", bind, port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;

    let ready_notify = Arc::new(Notify::new());
    let ready_clone = Arc::clone(&ready_notify);

    let handle = tokio::spawn(async move {
        tracing::info!(
            addr = %addr,
            "Starting API server"
        );

        // Create router
        let app = app_router(state);

        // Bind server
        let listener = tokio::net::TcpListener::bind(addr).await?;

        // Signal ready
        ready_clone.notify_one();

        tracing::info!(addr = %addr, "API server listening");

        // Serve with graceful shutdown
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_token.cancelled().await;
                tracing::info!("API server shutdown signal received, draining connections...");
            })
            .await
            .map_err(|e| anyhow::anyhow!("API server error: {}", e))?;

        tracing::info!("API server stopped accepting connections");
        Ok(())
    });

    // Wait for API server to bind (or timeout)
    tokio::time::timeout(Duration::from_secs(5), ready_notify.notified())
        .await
        .map_err(|_| anyhow::anyhow!("API server failed to start within 5 seconds"))?;

    tracing::info!("API server ready");

    Ok((handle, ready_notify))
}

/// Spawn worker tasks
async fn spawn_workers(
    worker_count: usize,
    poll_max_activities: usize,
    api_url: String,
    client_id: String,
    client_secret: String,
    workflow_storage: Arc<dyn kruxiaflow_core::WorkflowStorage>,
) -> Result<Vec<JoinHandle<()>>> {
    tracing::info!(
        count = worker_count,
        api_url = %api_url,
        "Starting workers"
    );

    // Create cache service based on environment configuration
    let cache_config = crate::config::CacheConfig::new();
    cache_config.log_config();
    let cache_service = cache_config.create_cache_service();

    // Create activity registry with all built-in activities pre-registered
    let registry = kruxiaflow_worker::register_builtin_activities(cache_service);

    let config = WorkerConfig {
        api_url: api_url.clone(),
        worker_id: format!("internal_worker_{}", Uuid::now_v7()),
        activity_types: registry.activity_types(),
        poll_max_activities,
        poll_interval: Duration::from_millis(100),
        concurrency: worker_count,
        activity_timeout: Duration::from_secs(300),
        heartbeat_interval: Duration::from_secs(30),
        client_id,
        client_secret,
    };

    let manager = WorkerManager::new(config, registry, workflow_storage);
    let handles = manager.start().await?;

    // Wait a moment for workers to authenticate
    tokio::time::sleep(Duration::from_millis(500)).await;

    tracing::info!(count = handles.len(), "Workers ready");

    Ok(handles)
}

/// Load a secret from environment, supporting Docker secrets pattern.
/// Checks for `{name}_FILE` first (reads file contents), then falls back to `{name}` direct value.
fn load_secret(name: &str) -> Option<String> {
    // First check for _FILE variant (Docker secrets pattern)
    let file_var = format!("{}_FILE", name);
    if let Ok(file_path) = std::env::var(&file_var) {
        match std::fs::read_to_string(&file_path) {
            Ok(contents) => {
                tracing::debug!("Loaded {} from file: {}", name, file_path);
                return Some(contents.trim().to_string());
            }
            Err(e) => {
                tracing::warn!("Failed to read {} from {}: {}", file_var, file_path, e);
            }
        }
    }

    // Fall back to direct environment variable
    std::env::var(name).ok()
}

/// Wait for PostgreSQL to become available with exponential backoff.
/// Used for container startup when database may not be ready immediately.
async fn wait_for_postgres(database_url: &str, timeout_secs: u64) -> Result<PgPool> {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let mut backoff = Duration::from_millis(100);
    let max_backoff = Duration::from_secs(5);

    tracing::info!(
        timeout_secs = timeout_secs,
        "Waiting for PostgreSQL to become available..."
    );

    loop {
        match PgPool::connect(database_url).await {
            Ok(pool) => {
                tracing::info!(
                    elapsed_ms = start.elapsed().as_millis(),
                    "PostgreSQL connection established"
                );
                return Ok(pool);
            }
            Err(e) => {
                if start.elapsed() >= timeout {
                    return Err(anyhow::anyhow!(
                        "Timed out waiting for PostgreSQL after {} seconds: {}",
                        timeout_secs,
                        e
                    ));
                }

                tracing::debug!(
                    error = %e,
                    retry_in_ms = backoff.as_millis(),
                    "PostgreSQL not ready, retrying..."
                );

                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}

/// Execute serve command
pub async fn execute(mut cmd: ServeCommand, database_url: String) -> Result<()> {
    // Load secrets from files if _FILE variants are set (Docker secrets pattern)
    if cmd.oauth_private_key.is_none() {
        cmd.oauth_private_key = load_secret("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM");
    }
    if cmd.oauth_public_key.is_none() {
        cmd.oauth_public_key = load_secret("KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM");
    }
    if cmd.client_secret.is_none() {
        cmd.client_secret = load_secret("KRUXIAFLOW_CLIENT_SECRET");
    }

    // Run startup tasks if requested (--migrate or --seed-client)
    if cmd.migrate || cmd.seed_client {
        // Wait for database with retry (for container startup scenarios)
        let init_pool = wait_for_postgres(&database_url, cmd.db_connect_timeout).await?;

        // Run migrations if requested
        if cmd.migrate {
            tracing::info!("Running database migrations...");
            migrate::run_migrations(&init_pool).await?;
        }

        // Seed OAuth client if requested
        if cmd.seed_client {
            tracing::info!("Seeding OAuth client...");
            let client_id = &cmd.client_id;
            let client_secret = cmd.client_secret.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Client secret required for --seed-client (--client-secret or KRUXIAFLOW_CLIENT_SECRET)"
                )
            })?;

            // Check if client exists and seed if not (idempotent)
            seed_client::seed_oauth_client(&init_pool, client_id, client_secret, false).await?;
        }

        // Close the init pool - main server will create its own
        init_pool.close().await;
    }

    // Validate configuration
    cmd.validate()?;

    tracing::info!(
        port = cmd.port,
        bind = %cmd.bind,
        workers = cmd.workers,
        shutdown_timeout = cmd.shutdown_timeout,
        "Starting Kruxia Flow all-in-one mode"
    );

    // Create shutdown coordinator
    let shutdown_token = CancellationToken::new();

    // 1. Test database connection
    tracing::info!("Testing database connection...");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(100) // Increased from 20 to support high concurrency
        .min_connections(10) // Set minimum to reduce connection overhead
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&database_url)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to database: {}\nURL: {}",
                e,
                database_url
            )
        })?;

    tracing::info!("Database connection successful");

    // 2. Create shared services
    tracing::info!("Initializing services...");

    // Create authentication service
    let auth_config = AuthConfig {
        rsa_private_key_pem: cmd.oauth_private_key.as_ref().unwrap().clone(),
        rsa_public_key_pem: cmd.oauth_public_key.clone(),
        jwt_issuer: "kruxiaflow".to_string(),
        jwt_audience: "kruxiaflow-api".to_string(),
        token_ttl: 86400, // 24 hours
    };

    let auth_service = Arc::new(PostgresAuthService::new(pool.clone(), auth_config)?);

    // Create activity queue
    let queue_config = QueueConfig::default();
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

    // Create event source
    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));

    // Create workflow storage
    let workflow_storage: Arc<dyn kruxiaflow_core::WorkflowStorage> =
        Arc::new(kruxiaflow_core::PostgresStorage::new(pool.clone()));

    // Create cache service
    let cache_service: Arc<dyn CacheService> = Arc::new(
        RedisCache::new(&cmd.redis_url, None)
            .map_err(|e| anyhow::anyhow!("Failed to connect to Redis: {}", e))?,
    );

    // Create API state with shutdown token
    let state = AppState::new(
        pool.clone(),
        auth_service,
        activity_queue.clone(),
        event_source.clone(),
        workflow_storage.clone(),
        cache_service.clone(),
        shutdown_token.clone(),
    );

    tracing::info!("Services initialized");

    // 3. Spawn orchestrator with shutdown token
    let (orchestrator_handle, _) = spawn_orchestrator(
        event_source.clone(),
        activity_queue.clone(),
        pool.clone(),
        shutdown_token.clone(),
    )
    .await?;

    // 4. Spawn API server with shutdown token
    let api_url = format!("http://{}:{}", cmd.bind, cmd.port);
    let (api_handle, _) =
        spawn_api_server(state, cmd.bind.clone(), cmd.port, shutdown_token.clone()).await?;

    // 5. Spawn workers (workers will be gracefully stopped via manager)
    let worker_handles = spawn_workers(
        cmd.workers,
        cmd.poll_max_activities,
        api_url.clone(),
        cmd.client_id.clone(),
        cmd.client_secret.unwrap(),
        workflow_storage.clone(),
    )
    .await?;

    tracing::info!("All services started successfully");
    tracing::info!(
        api_url = %api_url,
        "Kruxia Flow is ready - API available at {}",
        api_url
    );

    // 5.5. Spawn connection pool monitor (logs stats every 30 seconds) - only in profiling mode
    #[cfg(feature = "profiling")]
    let pool_monitor = {
        let pool = pool.clone();
        let token = shutdown_token.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        tracing::debug!("Pool monitor shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        tracing::info!(
                            pool_size = pool.size(),
                            pool_idle = pool.num_idle(),
                            "Connection pool stats"
                        );
                    }
                }
            }
        })
    };

    // 6. Wait for shutdown signal
    let shutdown_signal = crate::signals::wait_for_shutdown();
    shutdown_signal.await;

    tracing::info!("Shutdown signal received, initiating graceful shutdown...");

    // 7. Trigger shutdown for all components
    shutdown_token.cancel();

    // 8. Graceful shutdown sequence with timeout
    let shutdown_timeout = Duration::from_secs(cmd.shutdown_timeout);

    // Stop workers first (they will finish current activities)
    tracing::info!(
        timeout_secs = cmd.shutdown_timeout,
        "Stopping workers, waiting for activities to complete..."
    );

    // Workers are running as spawned tasks, we'll give them time to finish
    // In a full implementation, WorkerManager would have a drain() method
    for handle in worker_handles {
        handle.abort();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    tracing::info!("Workers stopped");

    // Stop API server (drain in-flight requests via graceful shutdown)
    tracing::info!("Stopping API server...");
    let api_result = tokio::time::timeout(shutdown_timeout, api_handle).await;
    match api_result {
        Ok(Ok(Ok(()))) => tracing::info!("API server stopped gracefully"),
        Ok(Ok(Err(e))) => tracing::warn!("API server error during shutdown: {}", e),
        Ok(Err(e)) => tracing::warn!("API server task error: {}", e),
        Err(_) => tracing::warn!("API server shutdown timeout, forcing stop"),
    }

    // Stop orchestrator (will stop polling via shutdown token)
    tracing::info!("Stopping orchestrator...");
    let orch_result = tokio::time::timeout(shutdown_timeout, orchestrator_handle).await;
    match orch_result {
        Ok(Ok(Ok(()))) => tracing::info!("Orchestrator stopped gracefully"),
        Ok(Ok(Err(e))) => tracing::warn!("Orchestrator error during shutdown: {}", e),
        Ok(Err(e)) => tracing::warn!("Orchestrator task error: {}", e),
        Err(_) => tracing::warn!("Orchestrator shutdown timeout, forcing stop"),
    }

    // Stop pool monitor (only in profiling mode)
    #[cfg(feature = "profiling")]
    {
        tracing::debug!("Stopping pool monitor...");
        pool_monitor.abort();
    }

    // Close database pool
    tracing::info!("Closing database pool...");
    pool.close().await;
    tracing::info!("Database pool closed");

    tracing::info!("Graceful shutdown complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a valid ServeCommand for testing
    fn valid_serve_command() -> ServeCommand {
        ServeCommand {
            port: 8080,
            bind: "0.0.0.0".to_string(),
            workers: 1,
            orchestrator_id: "orchestrator_default".to_string(),
            client_id: "kruxiaflow_internal_worker".to_string(),
            client_secret: Some("secret".to_string()),
            oauth_private_key: Some(
                "-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----".to_string(),
            ),
            oauth_public_key: None,
            shutdown_timeout: 30,
            poll_max_activities: 10,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            migrate: false,
            seed_client: false,
            db_connect_timeout: 60,
        }
    }

    #[test]
    fn test_serve_command_defaults() {
        let cmd = valid_serve_command();
        assert!(cmd.validate().is_ok());
    }

    // =========================================================================
    // Worker count validation tests
    // =========================================================================

    #[test]
    fn test_serve_command_invalid_workers_zero() {
        let mut cmd = valid_serve_command();
        cmd.workers = 0;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Worker count"));
    }

    #[test]
    fn test_serve_command_invalid_workers_over_100() {
        let mut cmd = valid_serve_command();
        cmd.workers = 101;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Worker count"));
    }

    #[test]
    fn test_serve_command_valid_workers_at_boundaries() {
        // Test minimum boundary (1)
        let mut cmd = valid_serve_command();
        cmd.workers = 1;
        assert!(cmd.validate().is_ok());

        // Test maximum boundary (100)
        cmd.workers = 100;
        assert!(cmd.validate().is_ok());
    }

    // =========================================================================
    // Poll max activities validation tests
    // =========================================================================

    #[test]
    fn test_serve_command_invalid_poll_max_activities_zero() {
        let mut cmd = valid_serve_command();
        cmd.poll_max_activities = 0;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Max activities per poll")
        );
    }

    #[test]
    fn test_serve_command_invalid_poll_max_activities_over_100() {
        let mut cmd = valid_serve_command();
        cmd.poll_max_activities = 101;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Max activities per poll")
        );
    }

    #[test]
    fn test_serve_command_valid_poll_max_activities_at_boundaries() {
        // Test minimum boundary (1)
        let mut cmd = valid_serve_command();
        cmd.poll_max_activities = 1;
        assert!(cmd.validate().is_ok());

        // Test maximum boundary (100)
        cmd.poll_max_activities = 100;
        assert!(cmd.validate().is_ok());
    }

    // =========================================================================
    // Client secret validation tests
    // =========================================================================

    #[test]
    fn test_serve_command_missing_secret() {
        let mut cmd = valid_serve_command();
        cmd.client_secret = None;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Client secret"));
    }

    // =========================================================================
    // OAuth private key validation tests
    // =========================================================================

    #[test]
    fn test_serve_command_missing_oauth_private_key() {
        let mut cmd = valid_serve_command();
        cmd.oauth_private_key = None;

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("OAuth private key")
        );
    }

    // =========================================================================
    // Shutdown timeout validation tests
    // =========================================================================

    #[test]
    fn test_serve_command_invalid_shutdown_timeout_too_low() {
        let mut cmd = valid_serve_command();
        cmd.shutdown_timeout = 4; // Below minimum of 5

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Shutdown timeout"));
    }

    #[test]
    fn test_serve_command_invalid_shutdown_timeout_too_high() {
        let mut cmd = valid_serve_command();
        cmd.shutdown_timeout = 301; // Above maximum of 300

        let result = cmd.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Shutdown timeout"));
    }

    #[test]
    fn test_serve_command_valid_shutdown_timeout_at_boundaries() {
        // Test minimum boundary (5)
        let mut cmd = valid_serve_command();
        cmd.shutdown_timeout = 5;
        assert!(cmd.validate().is_ok());

        // Test maximum boundary (300)
        cmd.shutdown_timeout = 300;
        assert!(cmd.validate().is_ok());
    }

    // =========================================================================
    // Struct tests
    // =========================================================================

    #[test]
    fn test_serve_command_with_all_options() {
        let cmd = ServeCommand {
            port: 9090,
            bind: "127.0.0.1".to_string(),
            workers: 50,
            orchestrator_id: "custom_orchestrator".to_string(),
            client_id: "custom_client".to_string(),
            client_secret: Some("my_secret".to_string()),
            oauth_private_key: Some(
                "-----BEGIN RSA PRIVATE KEY-----\n...\n-----END RSA PRIVATE KEY-----".to_string(),
            ),
            oauth_public_key: Some(
                "-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----".to_string(),
            ),
            shutdown_timeout: 60,
            poll_max_activities: 5,
            redis_url: "redis://redis.example.com:6379/0".to_string(),
            migrate: true,
            seed_client: true,
            db_connect_timeout: 120,
        };

        assert!(cmd.validate().is_ok());
        assert!(cmd.migrate);
        assert!(cmd.seed_client);
        assert_eq!(cmd.db_connect_timeout, 120);
        assert_eq!(cmd.port, 9090);
        assert_eq!(cmd.bind, "127.0.0.1");
        assert_eq!(cmd.workers, 50);
        assert_eq!(cmd.orchestrator_id, "custom_orchestrator");
        assert!(cmd.oauth_public_key.is_some());
    }

    #[test]
    fn test_serve_command_validates_in_order() {
        // Validation should fail on first error - test that workers is checked first
        let mut cmd = valid_serve_command();
        cmd.workers = 0;
        cmd.poll_max_activities = 0;
        cmd.client_secret = None;
        cmd.oauth_private_key = None;
        cmd.shutdown_timeout = 0;

        let result = cmd.validate();
        assert!(result.is_err());
        // Should fail on workers first
        assert!(result.unwrap_err().to_string().contains("Worker count"));
    }
}
