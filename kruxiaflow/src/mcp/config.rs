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

use anyhow::Result;
use std::time::Duration;

/// MCP server transport type
///
/// Only HTTP transport is supported in the integrated MCP server.
/// For stdio transport, use a separate process (e.g., the Python MCP server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    /// HTTP transport with SSE (for network access, multi-client)
    /// This is the ONLY supported transport in the integrated MCP server.
    Http,
}

/// MCP server configuration
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Enable MCP server
    pub enabled: bool,

    /// Transport type (stdio or http)
    pub transport: McpTransport,

    /// HTTP port (only if transport=http)
    pub http_port: Option<u16>,

    /// HTTP bind address (only if transport=http)
    pub http_bind: Option<String>,

    /// Require authentication (default: false for stdio, true for http)
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
        transport_cli: Option<String>,
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

        // Transport: CLI > Env > Default (http)
        // Note: Only HTTP transport is supported (stdio requires separate process)
        let transport_str = transport_cli
            .or_else(|| std::env::var("KRUXIAFLOW_MCP_TRANSPORT").ok())
            .unwrap_or_else(|| "http".to_string())
            .to_lowercase();

        let transport = match transport_str.as_str() {
            "http" => McpTransport::Http,
            "stdio" => anyhow::bail!(
                "MCP stdio transport is not supported in the integrated MCP server.\n\
                Reason: The 'serve' command runs multiple services with logging to stdout/stderr,\n\
                which corrupts the MCP stdio protocol (requires clean stdin/stdout).\n\n\
                For stdio MCP support, use a separate process:\n\
                - Python MCP server: kruxiaflow-mcp/\n\
                - Standalone Rust MCP server: Create a dedicated binary"
            ),
            other => anyhow::bail!(
                "Invalid MCP transport: {} (only 'http' is supported)",
                other
            ),
        };

        // HTTP settings (always required since HTTP is the only transport)
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
        let max_concurrent_requests =
            std::env::var("KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS")
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
            transport,
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
            transport: McpTransport::Http,
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
            anyhow::bail!("MCP HTTP transport requires port. Set KRUXIAFLOW_MCP_HTTP_PORT");
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
            anyhow::bail!(
                "Authentication enabled but no JWT secret provided. \
                Set KRUXIAFLOW_MCP_JWT_SECRET"
            );
        }

        // Resource limits
        if self.max_concurrent_requests == 0 {
            anyhow::bail!("max_concurrent_requests must be > 0");
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
        tracing::info!("  Transport: HTTP (only supported transport)");

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

    #[test]
    #[serial]
    fn test_config_disabled_by_default() {
        // Clean environment
        unsafe {
            std::env::remove_var("KRUXIAFLOW_MCP_ENABLED");
        }

        let config = McpConfig::new(None, None, None, None).unwrap();
        assert!(!config.enabled);
        assert_eq!(config.transport, McpTransport::Http);
    }

    #[test]
    #[serial]
    fn test_config_stdio_transport_rejected() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
            std::env::set_var("KRUXIAFLOW_MCP_TRANSPORT", "stdio");
        }

        // Should fail with clear error message about stdio not being supported
        let result = McpConfig::new(None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("stdio transport is not supported"));
        assert!(err.contains("separate process"));

        unsafe {
            std::env::remove_var("KRUXIAFLOW_MCP_ENABLED");
            std::env::remove_var("KRUXIAFLOW_MCP_TRANSPORT");
        }
    }

    #[test]
    #[serial]
    fn test_config_http_defaults() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
            std::env::set_var("KRUXIAFLOW_MCP_JWT_SECRET", "test-secret");
            std::env::remove_var("KRUXIAFLOW_MCP_TRANSPORT");
            std::env::remove_var("KRUXIAFLOW_MCP_HTTP_PORT");
        }

        let config = McpConfig::new(None, None, None, None).unwrap();
        assert_eq!(config.transport, McpTransport::Http);
        assert_eq!(config.http_port, Some(8081)); // Default port
        assert_eq!(config.http_bind, Some("0.0.0.0".to_string())); // Default bind
        assert!(config.auth_required); // Default true for security

        unsafe {
            std::env::remove_var("KRUXIAFLOW_MCP_ENABLED");
            std::env::remove_var("KRUXIAFLOW_MCP_JWT_SECRET");
        }
    }

    #[test]
    #[serial]
    fn test_config_http_auth_required_by_default() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
            std::env::set_var("KRUXIAFLOW_MCP_TRANSPORT", "http");
            std::env::set_var("KRUXIAFLOW_MCP_JWT_SECRET", "test-secret");
        }

        let config = McpConfig::new(None, None, None, None).unwrap();
        assert_eq!(config.transport, McpTransport::Http);
        assert!(config.auth_required); // Default for HTTP
        assert_eq!(config.jwt_secret, Some("test-secret".to_string()));

        unsafe {
            std::env::remove_var("KRUXIAFLOW_MCP_ENABLED");
            std::env::remove_var("KRUXIAFLOW_MCP_TRANSPORT");
            std::env::remove_var("KRUXIAFLOW_MCP_JWT_SECRET");
        }
    }

    #[test]
    #[serial]
    fn test_cli_overrides_env() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "false");
            std::env::set_var("KRUXIAFLOW_MCP_HTTP_PORT", "8888");
            std::env::set_var("KRUXIAFLOW_MCP_JWT_SECRET", "test-secret");
        }

        let config = McpConfig::new(
            Some(true),                    // enabled (overrides env false)
            Some("http".to_string()),      // transport
            Some(9090),                    // port (overrides env 8888)
            Some("127.0.0.1".to_string()), // bind
        )
        .unwrap();

        assert!(config.enabled);
        assert_eq!(config.transport, McpTransport::Http);
        assert_eq!(config.http_port, Some(9090));
        assert_eq!(config.http_bind, Some("127.0.0.1".to_string()));

        unsafe {
            std::env::remove_var("KRUXIAFLOW_MCP_ENABLED");
            std::env::remove_var("KRUXIAFLOW_MCP_HTTP_PORT");
            std::env::remove_var("KRUXIAFLOW_MCP_JWT_SECRET");
        }
    }
}
