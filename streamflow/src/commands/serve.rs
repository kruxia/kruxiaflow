use anyhow::Result;
use clap::Args;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use streamflow_api::{AppState, app_router};
use streamflow_core::{
    ActivityQueue, EventSource, OrchestratorConfig, PostgresEventSource, PostgresQueue,
    QueueConfig, orchestrator::OrchestratorError, run_orchestrator,
};
use streamflow_oauth::{AuthConfig, PostgresAuthService};
use streamflow_worker::{ActivityRegistry, WorkerConfig, WorkerManager};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Serve command - Launch all services together
#[derive(Args)]
pub struct ServeCommand {
    /// API server port
    #[arg(
        short,
        long,
        env = "STREAMFLOW_API_PORT",
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
        env = "STREAMFLOW_API_BIND",
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
        env = "STREAMFLOW_WORKER_COUNT",
        default_value = "1",
        help = "Number of concurrent worker tasks",
        long_help = "Number of concurrent worker tasks to spawn\n\n\
Default: 1\n\
Range: 1-100\n\
Example: --workers 4"
    )]
    pub workers: usize,

    /// Orchestrator consumer ID (for event polling checkpoint)
    #[arg(
        long,
        env = "STREAMFLOW_ORCHESTRATOR_CONSUMER_ID",
        default_value = "orchestrator_default",
        help = "Orchestrator consumer ID for event checkpointing"
    )]
    pub orchestrator_id: String,

    /// Worker client ID for OAuth
    #[arg(
        long,
        env = "STREAMFLOW_CLIENT_ID",
        default_value = "streamflow_internal_worker",
        help = "OAuth client ID for internal workers"
    )]
    pub client_id: String,

    /// Worker client secret for OAuth
    #[arg(
        long,
        env = "STREAMFLOW_CLIENT_SECRET",
        help = "OAuth client secret for internal workers (required)"
    )]
    pub client_secret: Option<String>,

    /// OAuth RSA private key (PEM format)
    #[arg(
        long,
        env = "STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM",
        help = "RSA private key for JWT signing (required)"
    )]
    pub oauth_private_key: Option<String>,

    /// OAuth RSA public key (PEM format)
    #[arg(
        long,
        env = "STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM",
        help = "RSA public key for JWT validation (optional, derived from private key if not provided)"
    )]
    pub oauth_public_key: Option<String>,
}

impl ServeCommand {
    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0 || self.workers > 100 {
            anyhow::bail!("Worker count must be between 1 and 100");
        }

        if self.client_secret.is_none() {
            anyhow::bail!("Client secret required (--client-secret or STREAMFLOW_CLIENT_SECRET)");
        }

        if self.oauth_private_key.is_none() {
            anyhow::bail!(
                "OAuth private key required (--oauth-private-key or STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM)"
            );
        }

        Ok(())
    }
}

/// Spawn orchestrator task
async fn spawn_orchestrator(
    event_source: Arc<dyn EventSource>,
    activity_queue: Arc<dyn ActivityQueue>,
    pool: PgPool,
) -> Result<(JoinHandle<Result<()>>, Arc<Notify>)> {
    let ready_notify = Arc::new(Notify::new());
    let ready_clone = Arc::clone(&ready_notify);

    let config = OrchestratorConfig::new(pool);

    let handle = tokio::spawn(async move {
        tracing::info!("Starting orchestrator");

        // Signal ready immediately since we're just starting the loop
        ready_clone.notify_one();

        // Run orchestrator (polls events and schedules activities)
        run_orchestrator(event_source, activity_queue, config)
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

/// Spawn API server task
async fn spawn_api_server(
    state: AppState,
    bind: String,
    port: u16,
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

        // Serve
        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow::anyhow!("API server error: {}", e))?;

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
    api_url: String,
    client_id: String,
    client_secret: String,
) -> Result<Vec<JoinHandle<()>>> {
    tracing::info!(
        count = worker_count,
        api_url = %api_url,
        "Starting workers"
    );

    // Create activity registry with built-in activities
    let mut registry = ActivityRegistry::new();
    registry.register(Arc::new(streamflow_worker::activities::EchoActivity));

    // TODO: Register more built-in activities here
    // registry.register(Arc::new(HttpRequestActivity));
    // registry.register(Arc::new(LlmCompleteActivity));

    let config = WorkerConfig {
        api_url: api_url.clone(),
        worker_id: format!("internal_worker_{}", Uuid::now_v7()),
        activity_types: registry.activity_types(),
        max_activities_per_poll: 10,
        poll_interval: Duration::from_millis(100),
        concurrency: worker_count,
        activity_timeout: Duration::from_secs(300),
        heartbeat_interval: Duration::from_secs(30),
        client_id,
        client_secret,
    };

    let manager = WorkerManager::new(config, registry);
    let handles = manager.start().await?;

    // Wait a moment for workers to authenticate
    tokio::time::sleep(Duration::from_millis(500)).await;

    tracing::info!(count = handles.len(), "Workers ready");

    Ok(handles)
}

/// Execute serve command
pub async fn execute(cmd: ServeCommand, database_url: String) -> Result<()> {
    // Validate configuration
    cmd.validate()?;

    tracing::info!(
        port = cmd.port,
        bind = %cmd.bind,
        workers = cmd.workers,
        "Starting StreamFlow all-in-one mode"
    );

    // 1. Test database connection
    tracing::info!("Testing database connection...");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
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
        jwt_issuer: "streamflow".to_string(),
        jwt_audience: "streamflow-api".to_string(),
        token_ttl: 86400, // 24 hours
    };

    let auth_service = Arc::new(PostgresAuthService::new(pool.clone(), auth_config)?);

    // Create activity queue
    let queue_config = QueueConfig::default();
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

    // Create event source (consumer_id is not used in the constructor, will be used in polling)
    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));

    // Create API state
    let state = AppState::new(
        pool.clone(),
        auth_service,
        activity_queue.clone(),
        event_source.clone(),
    );

    tracing::info!("Services initialized");

    // 3. Spawn orchestrator
    let (orchestrator_handle, _) =
        spawn_orchestrator(event_source.clone(), activity_queue.clone(), pool.clone()).await?;

    // 4. Spawn API server
    let api_url = format!("http://{}:{}", cmd.bind, cmd.port);
    let (api_handle, _) = spawn_api_server(state, cmd.bind.clone(), cmd.port).await?;

    // 5. Spawn workers
    let worker_handles = spawn_workers(
        cmd.workers,
        api_url.clone(),
        cmd.client_id.clone(),
        cmd.client_secret.unwrap(),
    )
    .await?;

    tracing::info!("All services started successfully");
    tracing::info!(
        api_url = %api_url,
        "StreamFlow is ready - API available at {}",
        api_url
    );

    // 6. Setup signal handlers for graceful shutdown
    let shutdown_signal = crate::signals::wait_for_shutdown();

    // 7. Wait for shutdown signal
    shutdown_signal.await;

    tracing::info!("Shutdown signal received, stopping services...");

    // 8. Graceful shutdown sequence

    // Stop workers first (drain in-flight activities)
    tracing::info!("Stopping workers...");
    for handle in worker_handles {
        handle.abort();
    }
    // Give workers time to finish in-flight activities
    tokio::time::sleep(Duration::from_secs(2)).await;
    tracing::info!("Workers stopped");

    // Stop API server (close connections)
    tracing::info!("Stopping API server...");
    api_handle.abort();
    let _ = api_handle.await;
    tracing::info!("API server stopped");

    // Stop orchestrator (flush events)
    tracing::info!("Stopping orchestrator...");
    orchestrator_handle.abort();
    let _ = orchestrator_handle.await;
    tracing::info!("Orchestrator stopped");

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

    #[test]
    fn test_serve_command_defaults() {
        let cmd = ServeCommand {
            port: 8080,
            bind: "0.0.0.0".to_string(),
            workers: 1,
            orchestrator_id: "orchestrator_default".to_string(),
            client_id: "streamflow_internal_worker".to_string(),
            client_secret: Some("secret".to_string()),
            oauth_private_key: Some(
                "-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----".to_string(),
            ),
            oauth_public_key: None,
        };

        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn test_serve_command_invalid_workers() {
        let cmd = ServeCommand {
            port: 8080,
            bind: "0.0.0.0".to_string(),
            workers: 0,
            orchestrator_id: "orchestrator_default".to_string(),
            client_id: "streamflow_internal_worker".to_string(),
            client_secret: Some("secret".to_string()),
            oauth_private_key: Some("key".to_string()),
            oauth_public_key: None,
        };

        assert!(cmd.validate().is_err());
    }

    #[test]
    fn test_serve_command_missing_secret() {
        let cmd = ServeCommand {
            port: 8080,
            bind: "0.0.0.0".to_string(),
            workers: 1,
            orchestrator_id: "orchestrator_default".to_string(),
            client_id: "streamflow_internal_worker".to_string(),
            client_secret: None,
            oauth_private_key: Some("key".to_string()),
            oauth_public_key: None,
        };

        assert!(cmd.validate().is_err());
    }
}
