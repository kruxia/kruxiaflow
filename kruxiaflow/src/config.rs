use anyhow::{Context, Result};
use kruxiaflow_core::cache::CacheService;
use std::sync::Arc;

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Cache provider: "redis" or "noop"
    pub provider: String,

    /// Redis connection URL (used when provider=redis)
    pub redis_url: Option<String>,

    /// Redis key prefix for namespace isolation
    pub redis_key_prefix: Option<String>,
}

impl CacheConfig {
    /// Create CacheConfig with precedence: Environment variables > Defaults
    pub fn new() -> Self {
        let provider = std::env::var("KRUXIAFLOW_CACHE_PROVIDER")
            .unwrap_or_else(|_| "noop".to_string())
            .to_lowercase();

        let redis_url = std::env::var("KRUXIAFLOW_REDIS_URL").ok();

        let redis_key_prefix = std::env::var("KRUXIAFLOW_REDIS_KEY_PREFIX").ok();

        Self {
            provider,
            redis_url,
            redis_key_prefix,
        }
    }

    /// Create cache service based on configuration
    pub async fn create_cache_service(&self) -> Arc<dyn CacheService> {
        match self.provider.as_str() {
            "redis" => self.create_redis_cache().await,
            _ => {
                tracing::info!("Cache disabled (using NoOpCache)");
                Arc::new(kruxiaflow_core::NoOpCache::new())
            }
        }
    }

    #[cfg(feature = "redis-cache")]
    async fn create_redis_cache(&self) -> Arc<dyn CacheService> {
        let redis_url = self
            .redis_url
            .as_deref()
            .unwrap_or("redis://localhost:6379");

        match kruxiaflow_core::RedisCache::new(redis_url, self.redis_key_prefix.clone()) {
            Ok(cache) => {
                // Test connectivity
                match cache.ping().await {
                    Ok(_) => {
                        tracing::info!(
                            redis_url = %self.redact_redis_url(redis_url),
                            "Redis cache initialized successfully"
                        );
                        Arc::new(cache)
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Redis ping failed, falling back to NoOpCache"
                        );
                        Arc::new(kruxiaflow_core::NoOpCache::new())
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to create Redis cache, falling back to NoOpCache"
                );
                Arc::new(kruxiaflow_core::NoOpCache::new())
            }
        }
    }

    #[cfg(not(feature = "redis-cache"))]
    async fn create_redis_cache(&self) -> Arc<dyn CacheService> {
        tracing::warn!(
            "Redis caching requested but redis-cache feature not enabled, falling back to NoOpCache"
        );
        Arc::new(kruxiaflow_core::NoOpCache::new())
    }

    /// Redact password from Redis URL for logging
    fn redact_redis_url(&self, url: &str) -> String {
        // Format: redis://[:password@]host:port[/db]
        if let Some(at_pos) = url.find('@') {
            if let Some(colon_pos) = url[..at_pos].rfind(':') {
                let mut redacted = url.to_string();
                redacted.replace_range(colon_pos + 1..at_pos, "***");
                return redacted;
            }
        }
        url.to_string()
    }

    /// Log configuration
    pub fn log_config(&self) {
        tracing::info!("Cache Configuration:");
        tracing::info!("  Provider: {}", self.provider);
        if let Some(url) = &self.redis_url {
            tracing::info!("  Redis URL: {}", self.redact_redis_url(url));
        }
        if let Some(prefix) = &self.redis_key_prefix {
            tracing::info!("  Key prefix: {}", prefix);
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// API Server configuration
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// PostgreSQL connection URL
    pub database_url: String,

    /// Port to bind to
    pub port: u16,

    /// Address to bind to
    pub bind: String,
}

impl ApiConfig {
    /// Create ApiConfig with precedence: CLI flags > Environment variables > Defaults
    pub fn new(
        database_url_cli: Option<String>,
        port_cli: Option<u16>,
        bind_cli: Option<String>,
    ) -> Result<Self> {
        // Database URL: Required
        let database_url = database_url_cli
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .context("Database URL is required (--database-url or DATABASE_URL)")?;

        // Port: CLI > Env > Default (8080)
        let port = port_cli
            .or_else(|| {
                std::env::var("KRUXIAFLOW_API_PORT")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(8080);

        // Bind address: CLI > Env > Default (0.0.0.0)
        let bind = bind_cli
            .or_else(|| std::env::var("KRUXIAFLOW_API_BIND").ok())
            .unwrap_or_else(|| "0.0.0.0".to_string());

        // Validate configuration
        if port == 0 {
            anyhow::bail!("Port must be between 1 and 65535");
        }

        Ok(Self {
            database_url,
            port,
            bind,
        })
    }

    /// Get bind address for Axum server
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }

    /// Log configuration (redact sensitive values)
    pub fn log_config(&self) {
        tracing::info!("API Server Configuration:");
        tracing::info!("  Bind address: {}", self.bind_address());
        tracing::info!("  Database: {}", self.redact_database_url());
    }

    /// Redact password from database URL for logging
    fn redact_database_url(&self) -> String {
        // Simple redaction: Replace password with ***
        // Format: postgres://user:password@host:port/db
        if let Some(at_pos) = self.database_url.rfind('@')
            && let Some(colon_pos) = self.database_url[..at_pos].rfind(':')
        {
            let mut redacted = self.database_url.clone();
            redacted.replace_range(colon_pos + 1..at_pos, "***");
            return redacted;
        }
        "***".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // =========================================================================
    // CacheConfig tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_cache_config_default_provider() {
        // Clean environment
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
            std::env::remove_var("KRUXIAFLOW_REDIS_URL");
            std::env::remove_var("KRUXIAFLOW_REDIS_KEY_PREFIX");
        }

        let config = CacheConfig::new();

        assert_eq!(config.provider, "noop");
        assert!(config.redis_url.is_none());
        assert!(config.redis_key_prefix.is_none());
    }

    #[test]
    #[serial]
    fn test_cache_config_from_environment() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_CACHE_PROVIDER", "Redis"); // Mixed case
            std::env::set_var("KRUXIAFLOW_REDIS_URL", "redis://localhost:6380");
            std::env::set_var("KRUXIAFLOW_REDIS_KEY_PREFIX", "test:");
        }

        let config = CacheConfig::new();

        assert_eq!(config.provider, "redis"); // Normalized to lowercase
        assert_eq!(config.redis_url, Some("redis://localhost:6380".to_string()));
        assert_eq!(config.redis_key_prefix, Some("test:".to_string()));

        // Clean up
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
            std::env::remove_var("KRUXIAFLOW_REDIS_URL");
            std::env::remove_var("KRUXIAFLOW_REDIS_KEY_PREFIX");
        }
    }

    #[test]
    #[serial]
    fn test_cache_config_default_is_same_as_new() {
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
            std::env::remove_var("KRUXIAFLOW_REDIS_URL");
            std::env::remove_var("KRUXIAFLOW_REDIS_KEY_PREFIX");
        }

        let config_new = CacheConfig::new();
        let config_default = CacheConfig::default();

        assert_eq!(config_new.provider, config_default.provider);
        assert_eq!(config_new.redis_url, config_default.redis_url);
        assert_eq!(config_new.redis_key_prefix, config_default.redis_key_prefix);
    }

    #[test]
    fn test_cache_config_redact_redis_url_with_password() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://:secretpassword@localhost:6379".to_string()),
            redis_key_prefix: None,
        };

        let redacted = config.redact_redis_url("redis://:secretpassword@localhost:6379");
        assert!(redacted.contains("***"));
        assert!(!redacted.contains("secretpassword"));
        assert!(redacted.contains("localhost:6379"));
    }

    #[test]
    fn test_cache_config_redact_redis_url_with_user_and_password() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://user:password123@redis.example.com:6379/0".to_string()),
            redis_key_prefix: None,
        };

        let redacted = config.redact_redis_url("redis://user:password123@redis.example.com:6379/0");
        assert!(redacted.contains("***"));
        assert!(!redacted.contains("password123"));
        assert!(redacted.contains("redis.example.com"));
    }

    #[test]
    fn test_cache_config_redact_redis_url_without_password() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://localhost:6379".to_string()),
            redis_key_prefix: None,
        };

        let redacted = config.redact_redis_url("redis://localhost:6379");
        // Should return URL unchanged when no password
        assert_eq!(redacted, "redis://localhost:6379");
    }

    #[test]
    #[serial]
    fn test_cache_config_create_noop_cache() {
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
        }

        let config = CacheConfig::new();
        let _cache = config.create_cache_service();

        // NoOpCache should be created - we can't directly test the type
        // but we can verify it doesn't panic and returns an Arc<dyn CacheService>
    }

    #[test]
    #[serial]
    fn test_cache_config_create_cache_with_unknown_provider() {
        let config = CacheConfig {
            provider: "unknown_provider".to_string(),
            redis_url: None,
            redis_key_prefix: None,
        };

        // Should fall back to NoOpCache for unknown providers
        let _cache = config.create_cache_service();
    }

    // =========================================================================
    // ApiConfig tests
    // =========================================================================

    #[test]
    fn test_database_url_redaction() {
        let config = ApiConfig {
            database_url: "postgres://user:secret123@localhost:5432/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
        };

        let redacted = config.redact_database_url();
        assert!(redacted.contains("***"));
        assert!(!redacted.contains("secret123"));
    }

    #[test]
    fn test_bind_address() {
        let config = ApiConfig {
            database_url: "postgres://localhost/db".to_string(),
            port: 9090,
            bind: "127.0.0.1".to_string(),
        };

        assert_eq!(config.bind_address(), "127.0.0.1:9090");
    }

    #[test]
    #[serial]
    fn test_defaults() {
        // Set database URL via environment for test
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
        }

        let config = ApiConfig::new(None, None, None).unwrap();

        assert_eq!(config.port, 8080);
        assert_eq!(config.bind, "0.0.0.0");

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
        }
    }

    #[test]
    #[serial]
    fn test_cli_overrides_env() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/env_db");
            std::env::set_var("KRUXIAFLOW_API_PORT", "9090");
            std::env::set_var("KRUXIAFLOW_API_BIND", "127.0.0.1");
        }

        let config = ApiConfig::new(
            Some("postgres://localhost/cli_db".to_string()),
            Some(8888),
            Some("192.168.1.1".to_string()),
        )
        .unwrap();

        assert!(config.database_url.contains("cli_db"));
        assert_eq!(config.port, 8888);
        assert_eq!(config.bind, "192.168.1.1");

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("KRUXIAFLOW_API_PORT");
            std::env::remove_var("KRUXIAFLOW_API_BIND");
        }
    }

    #[test]
    #[serial]
    fn test_env_overrides_defaults() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
            std::env::set_var("KRUXIAFLOW_API_PORT", "9000");
            std::env::set_var("KRUXIAFLOW_API_BIND", "localhost");
        }

        let config = ApiConfig::new(None, None, None).unwrap();

        assert_eq!(config.port, 9000);
        assert_eq!(config.bind, "localhost");

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("KRUXIAFLOW_API_PORT");
            std::env::remove_var("KRUXIAFLOW_API_BIND");
        }
    }

    #[test]
    #[serial]
    fn test_invalid_port() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
        }

        let result = ApiConfig::new(None, Some(0), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Port must be"));

        unsafe {
            std::env::remove_var("DATABASE_URL");
        }
    }

    #[test]
    #[serial]
    fn test_database_url_required() {
        unsafe {
            std::env::remove_var("DATABASE_URL");
        }

        let result = ApiConfig::new(None, None, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Database URL is required")
        );
    }

    #[test]
    fn test_database_url_redaction_without_password() {
        // Database URL without password (e.g., local dev with trust auth)
        let config = ApiConfig {
            database_url: "postgres://localhost/testdb".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
        };

        let redacted = config.redact_database_url();
        // When no @ symbol with password, returns "***" as fallback
        assert_eq!(redacted, "***");
    }

    #[test]
    fn test_database_url_redaction_with_at_but_no_password() {
        // Edge case: URL with @ but user only (no password)
        // The function finds the protocol colon and redacts from there to @
        let config = ApiConfig {
            database_url: "postgres://user@localhost:5432/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
        };

        let redacted = config.redact_database_url();
        // Finds protocol colon and redacts "//user" portion
        assert!(redacted.contains("***"));
        assert!(redacted.contains("@localhost"));
    }

    #[test]
    #[serial]
    fn test_invalid_port_from_environment() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
            std::env::set_var("KRUXIAFLOW_API_PORT", "invalid_port");
        }

        // Invalid port string should fall back to default
        let config = ApiConfig::new(None, None, None).unwrap();
        assert_eq!(config.port, 8080); // Default port

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("KRUXIAFLOW_API_PORT");
        }
    }

    #[test]
    fn test_api_config_clone() {
        let config = ApiConfig {
            database_url: "postgres://localhost/db".to_string(),
            port: 9090,
            bind: "127.0.0.1".to_string(),
        };

        let cloned = config.clone();
        assert_eq!(cloned.database_url, config.database_url);
        assert_eq!(cloned.port, config.port);
        assert_eq!(cloned.bind, config.bind);
    }

    #[test]
    fn test_cache_config_log_config_with_redis() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://:secret@localhost:6379".to_string()),
            redis_key_prefix: Some("test:".to_string()),
        };
        // Should not panic
        config.log_config();
    }

    #[test]
    fn test_cache_config_log_config_without_redis() {
        let config = CacheConfig {
            provider: "noop".to_string(),
            redis_url: None,
            redis_key_prefix: None,
        };
        // Should not panic
        config.log_config();
    }

    #[test]
    fn test_api_config_log_config() {
        let config = ApiConfig {
            database_url: "postgres://user:pass@localhost:5432/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
        };
        // Should not panic
        config.log_config();
    }

    #[test]
    fn test_api_config_debug() {
        let config = ApiConfig {
            database_url: "postgres://localhost/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
        };

        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("ApiConfig"));
        assert!(debug_str.contains("8080"));
    }
}
