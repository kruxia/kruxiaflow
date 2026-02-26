/// MCP server configuration (simplified)
///
/// Configuration follows Kruxia Flow patterns:
/// - CLI flags > Environment variables > Defaults
/// - Only essential options to avoid complexity
/// - Sensible defaults for common use cases
///
/// # Transport: HTTP Only
///
/// This MCP server ONLY supports HTTP transport, not stdio transport.
///
/// **Why no stdio support?**
/// - The `serve` command runs multiple services (API server, orchestrator, workers, MCP server)
/// - All services log to stdout/stderr for observability
/// - MCP stdio transport requires CLEAN stdin/stdout (no logging output)
/// - Running stdio MCP alongside logging services would corrupt the MCP protocol
///
/// **For stdio MCP support:**
/// Use a separate process that runs ONLY the MCP server:
/// - Python MCP server: `kruxiaflow-mcp/` (already available)
/// - Standalone Rust MCP server: Create a dedicated binary with no logging to stdout
///
/// **This HTTP-only MCP server is ideal for:**
/// - Production deployments with monitoring/logging
/// - Multi-client AI agent access
/// - Network-accessible MCP endpoints
use std::time::Duration;

use super::error::{McpError, Result};

/// MCP server configuration
///
/// This MCP server uses Streamable HTTP transport exclusively.
/// Stdio transport is not supported because the `serve` command runs multiple
/// services that log to stdout/stderr, which would corrupt the MCP protocol.
/// For stdio MCP, use a separate process (e.g., the Python MCP server).
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Enable MCP server
    pub enabled: bool,

    /// HTTP port for the Streamable HTTP server
    pub http_port: Option<u16>,

    /// HTTP bind address
    pub http_bind: Option<String>,

    /// Require authentication (default: true)
    pub auth_required: bool,

    /// JWT secret for authentication (required if auth_required=true)
    pub jwt_secret: Option<String>,

    /// Maximum concurrent requests
    pub max_concurrent_requests: usize,

    /// Request timeout
    pub request_timeout: Duration,
}

impl McpConfig {
    /// Create McpConfig with precedence: CLI flags > Environment variables > Defaults
    pub fn new(
        enabled_cli: Option<bool>,
        http_port_cli: Option<u16>,
        http_bind_cli: Option<String>,
    ) -> Result<Self> {
        // Enabled: CLI > Env > Default (false)
        let enabled = enabled_cli
            .or_else(|| {
                std::env::var("KRUXIAFLOW_MCP_ENABLED")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(false);

        // If not enabled, return minimal config
        if !enabled {
            return Ok(Self::disabled());
        }

        // Reject stdio if someone explicitly requests it via env var
        if let Ok(transport) = std::env::var("KRUXIAFLOW_MCP_TRANSPORT")
            && transport.eq_ignore_ascii_case("stdio")
        {
            return Err(McpError::UnsupportedTransport(
                "The 'serve' command runs multiple services with logging to stdout/stderr, \
                which corrupts the MCP stdio protocol (requires clean stdin/stdout). \
                For stdio MCP support, use a separate process: \
                Python MCP server (kruxiaflow-mcp/) or a standalone Rust binary."
                    .to_string(),
            ));
        }

        // HTTP settings (Streamable HTTP is the only transport)
        let http_port = Some(
            http_port_cli
                .or_else(|| {
                    std::env::var("KRUXIAFLOW_MCP_HTTP_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(8081),
        );

        let http_bind = Some(
            http_bind_cli
                .or_else(|| std::env::var("KRUXIAFLOW_MCP_HTTP_BIND").ok())
                .unwrap_or_else(|| "0.0.0.0".to_string()),
        );

        // Authentication (required by default for HTTP)
        let auth_required = std::env::var("KRUXIAFLOW_MCP_AUTH_REQUIRED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(true); // Default: true for security

        let jwt_secret = if auth_required {
            std::env::var("KRUXIAFLOW_MCP_JWT_SECRET").ok()
        } else {
            None
        };

        // Resource limits
        let max_concurrent_requests = std::env::var("KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        let request_timeout = std::env::var("KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(30));

        let config = Self {
            enabled,
            http_port,
            http_bind,
            auth_required,
            jwt_secret,
            max_concurrent_requests,
            request_timeout,
        };

        config.validate()?;
        Ok(config)
    }

    /// Create a disabled configuration
    fn disabled() -> Self {
        Self {
            enabled: false,
            http_port: Some(8081),
            http_bind: Some("0.0.0.0".to_string()),
            auth_required: false,
            jwt_secret: None,
            max_concurrent_requests: 10,
            request_timeout: Duration::from_secs(30),
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // HTTP port is always required (HTTP is the only transport)
        if self.http_port.is_none() {
            return Err(McpError::ConfigError(
                "MCP HTTP transport requires port. Set KRUXIAFLOW_MCP_HTTP_PORT".to_string(),
            ));
        }

        // Warn about insecure HTTP without auth
        if !self.auth_required {
            tracing::warn!(
                "MCP HTTP transport without authentication is insecure! \
                Set KRUXIAFLOW_MCP_AUTH_REQUIRED=true"
            );
        }

        // Auth requires JWT secret
        if self.auth_required && self.jwt_secret.is_none() {
            return Err(McpError::ConfigError(
                "Authentication enabled but no JWT secret provided. \
                Set KRUXIAFLOW_MCP_JWT_SECRET"
                    .to_string(),
            ));
        }

        // Resource limits
        if self.max_concurrent_requests == 0 {
            return Err(McpError::ConfigError(
                "max_concurrent_requests must be > 0".to_string(),
            ));
        }

        Ok(())
    }

    /// Log configuration
    pub fn log_config(&self) {
        if !self.enabled {
            tracing::info!("MCP server: disabled");
            return;
        }

        tracing::info!("MCP Server Configuration:");
        tracing::info!("  Transport: Streamable HTTP");

        if let Some(port) = self.http_port {
            tracing::info!("  HTTP Port: {}", port);
        }
        if let Some(ref bind) = self.http_bind {
            tracing::info!("  HTTP Bind: {}", bind);
        }
        tracing::info!("  Auth required: {}", self.auth_required);

        tracing::info!(
            "  Max concurrent requests: {}",
            self.max_concurrent_requests
        );
        tracing::info!("  Request timeout: {:?}", self.request_timeout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// RAII guard that restores environment variables on drop (even on panic).
    struct EnvGuard {
        vars: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        /// Capture current values of the given env vars and return a guard.
        fn new(names: &[&str]) -> Self {
            let vars = names
                .iter()
                .map(|&name| (name.to_string(), std::env::var(name).ok()))
                .collect();
            Self { vars }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, original) in &self.vars {
                match original {
                    Some(val) => unsafe { std::env::set_var(name, val) },
                    None => unsafe { std::env::remove_var(name) },
                }
            }
        }
    }

    /// All MCP env var names used in tests.
    const MCP_ENV_VARS: &[&str] = &[
        "KRUXIAFLOW_MCP_ENABLED",
        "KRUXIAFLOW_MCP_TRANSPORT",
        "KRUXIAFLOW_MCP_HTTP_PORT",
        "KRUXIAFLOW_MCP_HTTP_BIND",
        "KRUXIAFLOW_MCP_JWT_SECRET",
        "KRUXIAFLOW_MCP_AUTH_REQUIRED",
        "KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS",
        "KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS",
    ];

    /// Helper: clean all MCP env vars for a fresh test.
    fn clean_mcp_env() {
        for var in MCP_ENV_VARS {
            unsafe { std::env::remove_var(var) };
        }
    }

    #[test]
    #[serial]
    fn test_config_disabled_by_default() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        let config = McpConfig::new(None, None, None).unwrap();
        assert!(!config.enabled);
    }

    #[test]
    #[serial]
    fn test_config_stdio_transport_rejected() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
            std::env::set_var("KRUXIAFLOW_MCP_TRANSPORT", "stdio");
        }

        let result = McpConfig::new(None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("stdio"));
        assert!(err.contains("separate process"));
    }

    #[test]
    #[serial]
    fn test_config_http_defaults() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
            std::env::set_var("KRUXIAFLOW_MCP_JWT_SECRET", "test-secret");
        }

        let config = McpConfig::new(None, None, None).unwrap();
        assert_eq!(config.http_port, Some(8081));
        assert_eq!(config.http_bind, Some("0.0.0.0".to_string()));
        assert!(config.auth_required);
    }

    #[test]
    #[serial]
    fn test_config_auth_required_by_default() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
            std::env::set_var("KRUXIAFLOW_MCP_JWT_SECRET", "test-secret");
        }

        let config = McpConfig::new(None, None, None).unwrap();
        assert!(config.auth_required);
        assert_eq!(config.jwt_secret, Some("test-secret".to_string()));
    }

    #[test]
    #[serial]
    fn test_cli_overrides_env() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "false");
            std::env::set_var("KRUXIAFLOW_MCP_HTTP_PORT", "8888");
            std::env::set_var("KRUXIAFLOW_MCP_JWT_SECRET", "test-secret");
        }

        let config = McpConfig::new(
            Some(true),
            Some(9090),
            Some("127.0.0.1".to_string()),
        )
        .unwrap();

        assert!(config.enabled);
        assert_eq!(config.http_port, Some(9090));
        assert_eq!(config.http_bind, Some("127.0.0.1".to_string()));
    }
}
