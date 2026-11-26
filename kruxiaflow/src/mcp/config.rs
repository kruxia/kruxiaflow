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
/// **For stdio-only MCP clients:**
/// Use a thin stdio→HTTP proxy (e.g. `mcp-remote`) pointed at the HTTP endpoint.
/// Modern clients (Claude Code, Cursor) speak HTTP MCP natively.
///
/// **This HTTP-only MCP server is ideal for:**
/// - Production deployments with monitoring/logging
/// - Multi-client AI agent access
/// - Network-accessible MCP endpoints
use super::error::{McpError, Result};

/// MCP server configuration
///
/// This MCP server uses Streamable HTTP transport exclusively.
/// Stdio transport is not supported because the `serve` command runs multiple
/// services that log to stdout/stderr, which would corrupt the MCP protocol.
/// For stdio-only clients, use a stdio→HTTP proxy.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Enable MCP server
    pub enabled: bool,

    /// HTTP port for the Streamable HTTP server
    pub http_port: Option<u16>,

    /// HTTP bind address
    pub http_bind: Option<String>,

    /// The `run_code` contract of the kruxiaflow-monty add-on worker, read
    /// from the JSON file `kruxiaflow-monty contract` writes. When set, the
    /// discovery tools list `run_code` with it; when not, `run_code` is
    /// hidden. See [`McpConfig::with_run_code_contract`].
    pub run_code_contract: Option<serde_json::Value>,
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
                Connect over Streamable HTTP instead (default: http://localhost:8081/mcp), \
                or use a stdio→HTTP proxy for stdio-only clients."
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

        let config = Self {
            enabled,
            http_port,
            http_bind,
            run_code_contract: None,
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
            run_code_contract: None,
        }
    }

    /// Read the kruxiaflow-monty add-on's `run_code` contract from `path`
    /// (`--mcp-run-code-contract` / `KRUXIAFLOW_MCP_RUN_CODE_CONTRACT`), so
    /// the discovery tools advertise `run_code`. `None` leaves it hidden.
    ///
    /// The file must be a JSON object naming the add-on's `worker` and its
    /// `capabilities`; anything else is a configuration error, reported at
    /// start-up rather than as a confusing catalog later.
    pub fn with_run_code_contract(mut self, path: Option<&std::path::Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(self);
        };
        let text = std::fs::read_to_string(path).map_err(|e| {
            McpError::ConfigError(format!(
                "Cannot read the run_code contract {}: {e}",
                path.display()
            ))
        })?;
        let contract: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            McpError::ConfigError(format!(
                "The run_code contract {} is not JSON: {e}",
                path.display()
            ))
        })?;
        if !contract["worker"].is_string() || !contract["capabilities"].is_array() {
            return Err(McpError::ConfigError(format!(
                "The run_code contract {} has no `worker` or `capabilities`: write it with \
                 `kruxiaflow-monty contract`",
                path.display()
            )));
        }
        self.run_code_contract = Some(contract);
        Ok(self)
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
        match &self.run_code_contract {
            Some(contract) => tracing::info!(
                "  run_code: advertised for worker {}",
                contract["worker"].as_str().unwrap_or_default()
            ),
            None => tracing::info!("  run_code: not advertised (no add-on contract configured)"),
        }
        // Auth is handled by the AuthenticationService passed to spawn_mcp_server.
        // When an auth service is provided, MCP uses the same RS256 tokens as the REST API.
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
        assert!(err.contains("Streamable HTTP"));
    }

    #[test]
    #[serial]
    fn test_config_http_defaults() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "true");
        }

        let config = McpConfig::new(None, None, None).unwrap();
        assert_eq!(config.http_port, Some(8081));
        assert_eq!(config.http_bind, Some("0.0.0.0".to_string()));
    }

    #[test]
    #[serial]
    fn test_cli_overrides_env() {
        let _guard = EnvGuard::new(MCP_ENV_VARS);
        clean_mcp_env();

        unsafe {
            std::env::set_var("KRUXIAFLOW_MCP_ENABLED", "false");
            std::env::set_var("KRUXIAFLOW_MCP_HTTP_PORT", "8888");
        }

        let config = McpConfig::new(Some(true), Some(9090), Some("127.0.0.1".to_string())).unwrap();

        assert!(config.enabled);
        assert_eq!(config.http_port, Some(9090));
        assert_eq!(config.http_bind, Some("127.0.0.1".to_string()));
    }

    /// A contract file with `text`, removed when dropped
    struct ContractFile(std::path::PathBuf);

    impl ContractFile {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for ContractFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn write_contract(text: &str) -> ContractFile {
        let path =
            std::env::temp_dir().join(format!("run-code-contract-{}.json", uuid::Uuid::now_v7()));
        std::fs::write(&path, text).unwrap();
        ContractFile(path)
    }

    #[test]
    fn test_run_code_contract_absent_hides_run_code() {
        let config = McpConfig::disabled().with_run_code_contract(None).unwrap();
        assert!(config.run_code_contract.is_none());
    }

    #[test]
    fn test_run_code_contract_read_from_file() {
        let file = write_contract(r#"{"worker": "monty", "capabilities": [{"name": "log"}]}"#);
        let config = McpConfig::disabled()
            .with_run_code_contract(Some(file.path()))
            .unwrap();
        assert_eq!(config.run_code_contract.unwrap()["worker"], "monty");
    }

    #[test]
    fn test_run_code_contract_errors_name_the_file() {
        let missing = std::path::Path::new("/nonexistent/run-code-contract.json");
        let err = McpConfig::disabled()
            .with_run_code_contract(Some(missing))
            .unwrap_err()
            .to_string();
        assert!(err.contains("/nonexistent/run-code-contract.json"), "{err}");

        for text in [
            "not json",
            r#"{"capabilities": []}"#,
            r#"{"worker": "monty"}"#,
        ] {
            let file = write_contract(text);
            let err = McpConfig::disabled()
                .with_run_code_contract(Some(file.path()))
                .unwrap_err()
                .to_string();
            assert!(err.contains("run_code contract"), "{text}: {err}");
        }
    }
}
