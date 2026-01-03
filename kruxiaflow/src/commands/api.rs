use crate::config::ApiConfig;
use crate::signals;
use anyhow::{Context, Result};
use clap::Args;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use kruxiaflow_core::cache::{CacheService, RedisCache};
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};

#[derive(Args)]
pub struct ApiCommand {
    /// Port to bind to
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_API_PORT",
        help = "Port to bind API server to",
        long_help = "Port to bind API server to\n\n\
Default: 8080\n\
Example: --port 9090"
    )]
    port: Option<u16>,

    /// Address to bind to
    #[arg(
        short,
        long,
        env = "KRUXIAFLOW_API_BIND",
        help = "Address to bind API server to (e.g., 0.0.0.0, 127.0.0.1)",
        long_help = "Address to bind API server to\n\n\
Options:\n  \
  0.0.0.0    - All network interfaces (default)\n  \
  127.0.0.1  - Localhost only (development)\n\
Example: --bind 127.0.0.1"
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
        .max_connections(200)
        .min_connections(20)
        .acquire_timeout(Duration::from_secs(10))
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
    let rsa_private_key_pem = std::env::var("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM").context(
        "KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM environment variable is required. \
             Generate keys with: openssl genrsa -out private.pem 2048 && \
             openssl rsa -in private.pem -pubout -out public.pem",
    )?;

    let rsa_public_key_pem = std::env::var("KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM").ok();

    tracing::info!("RSA keys loaded for JWT signing/verification");

    // Configure authentication service
    let auth_config = AuthConfig {
        rsa_private_key_pem,
        rsa_public_key_pem,
        jwt_issuer: std::env::var("KRUXIAFLOW_OAUTH_JWT_ISSUER")
            .unwrap_or_else(|_| "kruxiaflow".to_string()),
        jwt_audience: std::env::var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE")
            .unwrap_or_else(|_| "kruxiaflow-api".to_string()),
        token_ttl: std::env::var("KRUXIAFLOW_OAUTH_TOKEN_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400), // 24 hours
    };

    // Initialize authentication service
    let auth_service = PostgresAuthService::new(db_pool.clone(), auth_config)
        .context("Failed to initialize authentication service")?;

    tracing::info!("Authentication service initialized");

    // Initialize activity queue (PostgreSQL implementation for MVP)
    let activity_queue = Arc::new(PostgresQueue::new(db_pool.clone(), QueueConfig::default()));
    tracing::info!("Activity queue initialized (PostgreSQL)");

    // Initialize event source (PostgreSQL polling implementation for MVP)
    let event_source = Arc::new(PostgresEventSource::new(db_pool.clone()));
    tracing::info!("Event source initialized (PostgreSQL polling)");

    // Initialize workflow storage (PostgreSQL Large Objects for MVP)
    let workflow_storage = Arc::new(kruxiaflow_core::PostgresStorage::new(db_pool.clone()));
    tracing::info!("Workflow storage initialized (PostgreSQL Large Objects)");

    // Initialize cache service (Redis for MVP)
    let redis_url = std::env::var("KRUXIAFLOW_REDIS_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let cache_service: Arc<dyn CacheService> =
        Arc::new(RedisCache::new(&redis_url, None).context("Failed to connect to Redis")?);
    tracing::info!("Cache service initialized (Redis)");

    // Create application state with configured infrastructure services
    let shutdown_token = tokio_util::sync::CancellationToken::new();
    let app_state = kruxiaflow_api::AppState::new(
        db_pool,
        Arc::new(auth_service),
        activity_queue,
        event_source,
        workflow_storage,
        cache_service,
        shutdown_token,
    );

    // Create Axum router
    let app = kruxiaflow_api::app_router(app_state);

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

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_api_command_construction() {
        // Test that ApiCommand can be constructed with all fields
        let cmd = ApiCommand {
            port: Some(8080),
            bind: Some("127.0.0.1".to_string()),
        };
        assert_eq!(cmd.port, Some(8080));
        assert_eq!(cmd.bind, Some("127.0.0.1".to_string()));
    }

    #[test]
    fn test_api_command_with_none_values() {
        // Test that ApiCommand can be constructed with None values
        let cmd = ApiCommand {
            port: None,
            bind: None,
        };
        assert_eq!(cmd.port, None);
        assert_eq!(cmd.bind, None);
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_fails_without_database_url() {
        // Test that execute fails when no database URL is provided
        let cmd = ApiCommand {
            port: Some(8080),
            bind: Some("127.0.0.1".to_string()),
        };

        // Remove DATABASE_URL from environment
        unsafe {
            std::env::remove_var("DATABASE_URL");
        }

        let result = execute(cmd, None).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Database URL is required")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_fails_with_invalid_database_url() {
        // Test that execute fails with an invalid database URL
        let cmd = ApiCommand {
            port: Some(8080),
            bind: Some("127.0.0.1".to_string()),
        };

        let result = execute(cmd, Some("invalid://url".to_string())).await;
        assert!(result.is_err());
        // Should fail during database connection
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to connect to database") || err_msg.contains("invalid"));
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_fails_without_rsa_keys() {
        // Test that execute fails when RSA keys are not provided
        // This test requires a valid database URL but will fail at RSA key loading

        // Clean up RSA key environment variables
        unsafe {
            std::env::remove_var("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM");
            std::env::remove_var("KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM");
        }

        // Use default database URL for testing
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
        });

        let cmd = ApiCommand {
            port: Some(8181), // Use different port to avoid conflicts
            bind: Some("127.0.0.1".to_string()),
        };

        let result = execute(cmd, Some(database_url)).await;

        // Should fail either at database connection or RSA key loading
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM")
                || err_msg.contains("Failed to connect to database")
                || err_msg.contains("Database connectivity test failed")
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_with_invalid_rsa_key() {
        // Test that execute fails with invalid RSA key format

        // Set invalid RSA key
        unsafe {
            std::env::set_var("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM", "invalid-key");
        }

        // Use default database URL for testing
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
        });

        let cmd = ApiCommand {
            port: Some(8182),
            bind: Some("127.0.0.1".to_string()),
        };

        let result = execute(cmd, Some(database_url)).await;

        // Should fail either at database connection or auth service initialization
        assert!(result.is_err());

        // Clean up
        unsafe {
            std::env::remove_var("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM");
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_with_valid_config_fails_at_bind() {
        // Test that execute with valid database and RSA keys proceeds to bind attempt
        // Uses an invalid bind address to fail at binding rather than blocking

        // Load test RSA keys
        let private_key = include_str!("../../../oauth/tests/private.pem");
        let public_key = include_str!("../../../oauth/tests/public.pem");

        unsafe {
            std::env::set_var("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM", private_key);
            std::env::set_var("KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM", public_key);
            std::env::set_var("KRUXIAFLOW_OAUTH_JWT_ISSUER", "test-issuer");
            std::env::set_var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE", "test-audience");
            std::env::set_var("KRUXIAFLOW_OAUTH_TOKEN_TTL", "3600");
        }

        // Use default database URL for testing
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
        });

        let cmd = ApiCommand {
            port: Some(9999),
            bind: Some("256.256.256.256".to_string()), // Invalid IP to fail at bind
        };

        let result = execute(cmd, Some(database_url)).await;

        // Should fail either at database connection, auth initialization, or bind
        assert!(result.is_err());

        // Clean up
        unsafe {
            std::env::remove_var("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM");
            std::env::remove_var("KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM");
            std::env::remove_var("KRUXIAFLOW_OAUTH_JWT_ISSUER");
            std::env::remove_var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE");
            std::env::remove_var("KRUXIAFLOW_OAUTH_TOKEN_TTL");
        }
    }

    #[test]
    #[serial]
    fn test_auth_config_with_invalid_ttl() {
        // Test that invalid TTL values are handled gracefully
        unsafe {
            std::env::set_var("KRUXIAFLOW_OAUTH_TOKEN_TTL", "not-a-number");
        }

        let token_ttl: u64 = std::env::var("KRUXIAFLOW_OAUTH_TOKEN_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400);

        // Should fall back to default
        assert_eq!(token_ttl, 86400);

        // Clean up
        unsafe {
            std::env::remove_var("KRUXIAFLOW_OAUTH_TOKEN_TTL");
        }
    }

    #[test]
    #[serial]
    fn test_auth_config_defaults() {
        // Test that auth configuration uses sensible defaults
        unsafe {
            std::env::remove_var("KRUXIAFLOW_OAUTH_JWT_ISSUER");
            std::env::remove_var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE");
            std::env::remove_var("KRUXIAFLOW_OAUTH_TOKEN_TTL");
        }

        let issuer = std::env::var("KRUXIAFLOW_OAUTH_JWT_ISSUER")
            .unwrap_or_else(|_| "kruxiaflow".to_string());
        let audience = std::env::var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE")
            .unwrap_or_else(|_| "kruxiaflow-api".to_string());
        let token_ttl: u64 = std::env::var("KRUXIAFLOW_OAUTH_TOKEN_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400);

        assert_eq!(issuer, "kruxiaflow");
        assert_eq!(audience, "kruxiaflow-api");
        assert_eq!(token_ttl, 86400);
    }

    #[test]
    #[serial]
    fn test_auth_config_from_environment() {
        // Test that auth configuration can be set via environment variables
        unsafe {
            std::env::set_var("KRUXIAFLOW_OAUTH_JWT_ISSUER", "test-issuer");
            std::env::set_var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE", "test-audience");
            std::env::set_var("KRUXIAFLOW_OAUTH_TOKEN_TTL", "3600");
        }

        let issuer = std::env::var("KRUXIAFLOW_OAUTH_JWT_ISSUER")
            .unwrap_or_else(|_| "kruxiaflow".to_string());
        let audience = std::env::var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE")
            .unwrap_or_else(|_| "kruxiaflow-api".to_string());
        let token_ttl: u64 = std::env::var("KRUXIAFLOW_OAUTH_TOKEN_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400);

        assert_eq!(issuer, "test-issuer");
        assert_eq!(audience, "test-audience");
        assert_eq!(token_ttl, 3600);

        // Clean up
        unsafe {
            std::env::remove_var("KRUXIAFLOW_OAUTH_JWT_ISSUER");
            std::env::remove_var("KRUXIAFLOW_OAUTH_JWT_AUDIENCE");
            std::env::remove_var("KRUXIAFLOW_OAUTH_TOKEN_TTL");
        }
    }
}
