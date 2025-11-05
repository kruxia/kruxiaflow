use crate::config::ApiConfig;
use crate::signals;
use anyhow::{Context, Result};
use clap::Args;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use streamflow_oauth::{AuthConfig, PostgresAuthService};

#[derive(Args)]
pub struct ApiCommand {
    /// Port to bind to
    #[arg(
        short,
        long,
        env = "STREAMFLOW_API_PORT",
        help = "Port to bind API server to"
    )]
    port: Option<u16>,

    /// Address to bind to
    #[arg(
        short,
        long,
        env = "STREAMFLOW_API_BIND",
        help = "Address to bind API server to (e.g., 0.0.0.0, 127.0.0.1)"
    )]
    bind: Option<String>,
}

pub async fn execute(cmd: ApiCommand, database_url_global: Option<String>) -> Result<()> {
    // Build configuration from CLI args, env vars, and defaults
    let config = ApiConfig::new(database_url_global, cmd.port, cmd.bind)?;

    // Log effective configuration (redacts secrets)
    config.log_config();

    // Initialize database connection pool
    tracing::info!("Connecting to database...");
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("Failed to connect to database")?;

    tracing::info!("Database connection established");

    // Test database connectivity
    sqlx::query("SELECT 1")
        .fetch_one(&db_pool)
        .await
        .context("Database connectivity test failed")?;

    tracing::info!("Database connectivity verified");

    // Load RSA keys for JWT signing/verification from environment
    let rsa_private_key_pem = std::env::var("STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM").context(
        "STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM environment variable is required. \
             Generate keys with: openssl genrsa -out private.pem 2048 && \
             openssl rsa -in private.pem -pubout -out public.pem",
    )?;

    let rsa_public_key_pem = std::env::var("STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM").ok();

    tracing::info!("RSA keys loaded for JWT signing/verification");

    // Configure authentication service
    let auth_config = AuthConfig {
        rsa_private_key_pem,
        rsa_public_key_pem,
        jwt_issuer: std::env::var("STREAMFLOW_OAUTH_JWT_ISSUER")
            .unwrap_or_else(|_| "streamflow".to_string()),
        jwt_audience: std::env::var("STREAMFLOW_OAUTH_JWT_AUDIENCE")
            .unwrap_or_else(|_| "streamflow-api".to_string()),
        token_ttl: std::env::var("STREAMFLOW_OAUTH_TOKEN_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400), // 24 hours
    };

    // Initialize authentication service
    let auth_service = PostgresAuthService::new(db_pool.clone(), auth_config)
        .context("Failed to initialize authentication service")?;

    tracing::info!("Authentication service initialized");

    // Create application state
    let app_state = streamflow_api::AppState::new(db_pool, Arc::new(auth_service));

    // Create Axum router
    let app = streamflow_api::app_router(app_state);

    // Bind to address and port
    let bind_addr = config.bind_address();
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .context(format!("Failed to bind to {}", bind_addr))?;

    tracing::info!("API Server starting on http://{}", bind_addr);
    tracing::info!("Health check: http://{}/health", bind_addr);
    tracing::info!("Readiness check: http://{}/health/ready", bind_addr);
    tracing::info!("Service info: http://{}/api/v1/info", bind_addr);

    // Serve with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(signals::shutdown_signal())
        .await
        .context("API server error")?;

    tracing::info!("API Server stopped");

    Ok(())
}
