/// MCP server initialization and lifecycle management
///
/// # HTTP-Only Transport
///
/// This MCP server ONLY supports HTTP transport, not stdio transport.
///
/// **Why no stdio support?**
/// - The `serve` command runs multiple services (API server, orchestrator, workers, MCP server)
/// - All services log to stdout/stderr for observability and debugging
/// - MCP stdio transport requires CLEAN stdin/stdout with no logging output
/// - Running stdio MCP alongside logging services would corrupt the MCP protocol
///
/// **For stdio MCP support:**
/// Use a separate process that runs ONLY the MCP server with no stdout logging:
/// - Python MCP server: `kruxiaflow-mcp/` (already available)
/// - Standalone Rust MCP server: Create a dedicated binary with logging to file/syslog
///
/// **This HTTP-only MCP server is designed for:**
/// - Production deployments with full observability
/// - Multi-client AI agent access over network
/// - Coexistence with API server and other services

use anyhow::Result;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::task::JoinHandle;

use crate::mcp::config::McpConfig;
use crate::mcp::handler::KruxiaFlowMcpHandler;

/// MCP server instance
pub struct McpServer {
    config: Arc<McpConfig>,
    _pool: PgPool,
}

impl McpServer {
    /// Create a new MCP server instance
    pub fn new(config: Arc<McpConfig>, pool: PgPool) -> Self {
        Self {
            config,
            _pool: pool,
        }
    }

    /// Start the MCP server (HTTP transport only)
    pub async fn start(self) -> Result<()> {
        tracing::info!("Starting MCP HTTP server...");
        self.start_http().await
    }

    /// Start MCP server with HTTP transport
    async fn start_http(self) -> Result<()> {
        let port = self
            .config
            .http_port
            .expect("HTTP transport requires port");
        let bind = self
            .config
            .http_bind
            .clone()
            .expect("HTTP transport requires bind address");

        tracing::info!("MCP server running on HTTP transport at {}:{}", bind, port);

        // TODO: Implement HTTP transport using rust-mcp-sdk
        // For now, this is a placeholder

        // In actual implementation:
        // 1. Create HTTP server with hyper_server::create_server()
        // 2. Set up authentication middleware if auth_required
        // 3. Start server

        tracing::warn!("MCP HTTP transport not yet implemented (placeholder)");

        // Keep task alive for now
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
        }
    }
}

/// Create and configure MCP server
///
/// This function creates an MCP server instance but does not start it.
/// Call `start()` on the returned server to begin serving MCP requests.
pub fn create_mcp_server(config: Arc<McpConfig>, pool: PgPool) -> McpServer {
    McpServer::new(config, pool)
}

/// Spawn MCP server in a background task
///
/// This is the recommended way to run the MCP server alongside other services.
pub fn spawn_mcp_server(config: Arc<McpConfig>, pool: PgPool) -> JoinHandle<Result<()>> {
    let server = create_mcp_server(config, pool);
    tokio::spawn(async move { server.start().await })
}
