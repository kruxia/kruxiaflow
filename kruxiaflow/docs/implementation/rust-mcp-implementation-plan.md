# Rust MCP Server Implementation Plan

## Overview

This document provides a detailed technical plan for implementing the MCP server in Rust within the main Kruxia Flow binary, using the `rust-mcp-sdk` from https://github.com/rust-mcp-stack/rust-mcp-sdk.

## Architecture

### Module Structure

```
kruxiaflow/src/mcp/
├── mod.rs                      # Module exports and public API
├── config.rs                   # Configuration structs and parsing
├── server.rs                   # MCP server initialization and lifecycle
├── handler.rs                  # ServerHandler trait implementation
├── middleware.rs               # Rate limiting, auth, metrics
├── transport/
│   ├── mod.rs
│   ├── stdio.rs               # Stdio transport implementation
│   └── http.rs                # HTTP transport implementation
├── tools/
│   ├── mod.rs                 # Tool registry and routing
│   ├── discovery.rs           # Discovery tools (4 tools)
│   ├── execution.rs           # Execution tools (3 tools)
│   ├── observability.rs       # Observability tools (5 tools)
│   ├── visualization.rs       # Visualization tools (2 tools)
│   └── control.rs             # Control tools (2 tools)
└── utils/
    ├── mod.rs
    ├── mermaid.rs             # Mermaid diagram generation
    └── cost.rs                # Cost estimation utilities
```

### Dependencies

Add to `kruxiaflow/Cargo.toml`:

```toml
[dependencies]
# Existing dependencies...

# MCP Server (optional - excluded by default)
rust-mcp-sdk = { version = "0.8.2", optional = true }

# Already have these, but ensure versions are compatible:
# tokio, serde, serde_json, async-trait, anyhow, thiserror

[features]
default = []  # MCP NOT included by default (opt-in for smaller edge binaries)
mcp-server = ["dep:rust-mcp-sdk"]  # Optional MCP server feature
```

### Compile-Time Feature Flag (Opt-In)

The MCP server is **opt-in at compile time** to keep the default binary lean for edge deployments:

**Standard build (MCP excluded - default):**
```bash
cargo build --release
# Binary is ~2-5MB smaller, no MCP dependencies
```

**MCP-enabled build (opt-in):**
```bash
cargo build --release --features mcp-server
```

**Why opt-in?**
- ✅ **Smaller default binary** for edge deployments
- ✅ **Removes MCP dependencies** by default (rust-mcp-sdk and transitive deps)
- ✅ **Zero runtime overhead** when not needed
- ✅ **Explicit opt-in** for deployments that need AI agent integration

## Configuration Design (Simplified)

### Core Principles
- **Essential options only** - Avoid configuration complexity
- **Sensible defaults** - Works out of the box for common use cases
- **Environment variables only** - No config files, follows Kruxia Flow patterns
- **Progressive disclosure** - Basic use is simple, advanced use is possible

### Config Struct

```rust
// kruxiaflow/src/mcp/config.rs

use anyhow::Result;
use std::time::Duration;

/// MCP server transport type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    Stdio,
    Http,
}

/// MCP server configuration (simplified)
#[derive(Debug, Clone)]
pub struct McpConfig {
    // Core settings
    pub enabled: bool,
    pub transport: McpTransport,

    // HTTP settings (only if transport=http)
    pub http_port: Option<u16>,
    pub http_bind: Option<String>,

    // Security (HTTP only)
    pub auth_required: bool,
    pub jwt_secret: Option<String>,

    // Resource limits (simple)
    pub max_concurrent_requests: usize,
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
            .or_else(|| std::env::var("KRUXIAFLOW_MCP_ENABLED")
                .ok()
                .and_then(|s| s.parse().ok()))
            .unwrap_or(false);

        // If not enabled, return minimal config
        if !enabled {
            return Ok(Self::disabled());
        }

        // Transport: CLI > Env > Default (stdio)
        let transport_str = transport_cli
            .or_else(|| std::env::var("KRUXIAFLOW_MCP_TRANSPORT").ok())
            .unwrap_or_else(|| "stdio".to_string())
            .to_lowercase();

        let transport = match transport_str.as_str() {
            "stdio" => McpTransport::Stdio,
            "http" => McpTransport::Http,
            other => anyhow::bail!("Invalid MCP transport: {} (use 'stdio' or 'http')", other),
        };

        // HTTP settings (only if transport=http)
        let http_port = if transport == McpTransport::Http {
            Some(
                http_port_cli
                    .or_else(|| std::env::var("KRUXIAFLOW_MCP_HTTP_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok()))
                    .unwrap_or(8081),
            )
        } else {
            None
        };

        let http_bind = if transport == McpTransport::Http {
            Some(
                http_bind_cli
                    .or_else(|| std::env::var("KRUXIAFLOW_MCP_HTTP_BIND").ok())
                    .unwrap_or_else(|| "0.0.0.0".to_string()),
            )
        } else {
            None
        };

        // Authentication (HTTP only)
        let auth_required = std::env::var("KRUXIAFLOW_MCP_AUTH_REQUIRED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(transport == McpTransport::Http); // Default: false for stdio, true for http

        let jwt_secret = if auth_required {
            std::env::var("KRUXIAFLOW_MCP_JWT_SECRET").ok()
        } else {
            None
        };

        // Resource limits (simple)
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
            transport: McpTransport::Stdio,
            http_port: None,
            http_bind: None,
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

        // HTTP transport requires port
        if self.transport == McpTransport::Http && self.http_port.is_none() {
            anyhow::bail!(
                "MCP HTTP transport requires port. Set KRUXIAFLOW_MCP_HTTP_PORT"
            );
        }

        // Warn about insecure HTTP without auth
        if self.transport == McpTransport::Http && !self.auth_required {
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

    /// Log configuration (simple)
    pub fn log_config(&self) {
        if !self.enabled {
            tracing::info!("MCP server: disabled");
            return;
        }

        tracing::info!("MCP Server Configuration:");
        tracing::info!("  Transport: {:?}", self.transport);

        if self.transport == McpTransport::Http {
            if let Some(port) = self.http_port {
                tracing::info!("  HTTP Port: {}", port);
            }
            if let Some(ref bind) = self.http_bind {
                tracing::info!("  HTTP Bind: {}", bind);
            }
            tracing::info!("  Auth required: {}", self.auth_required);
        }

        tracing::info!("  Max concurrent requests: {}", self.max_concurrent_requests);
        tracing::info!("  Request timeout: {:?}", self.request_timeout);
    }
}
```

### Simplified Environment Variables

**Core Settings:**
- `KRUXIAFLOW_MCP_ENABLED` - Enable MCP server (default: `false`)
- `KRUXIAFLOW_MCP_TRANSPORT` - Transport type: `stdio` or `http` (default: `stdio`)

**HTTP Settings (only if transport=http):**
- `KRUXIAFLOW_MCP_HTTP_PORT` - HTTP port (default: `8081`)
- `KRUXIAFLOW_MCP_HTTP_BIND` - Bind address (default: `0.0.0.0`)

**Security (HTTP only):**
- `KRUXIAFLOW_MCP_AUTH_REQUIRED` - Require JWT auth (default: `false` for stdio, `true` for http)
- `KRUXIAFLOW_MCP_JWT_SECRET` - JWT secret key (required if auth enabled)

**Resource Limits:**
- `KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS` - Max concurrent requests (default: `10`)
- `KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS` - Request timeout in seconds (default: `30`)

**That's it!** Just 8 environment variables for the complete configuration.

## Server Handler Design (Simplified)

```rust
// kruxiaflow/src/mcp/handler.rs

use async_trait::async_trait;
use rust_mcp_sdk::prelude::*;
use std::sync::Arc;

use crate::mcp::config::McpConfig;
use crate::mcp::tools;
use sqlx::PgPool;

/// Main MCP server handler
pub struct KruxiaFlowMcpHandler {
    config: Arc<McpConfig>,
    pool: PgPool,
}

impl KruxiaFlowMcpHandler {
    pub fn new(config: Arc<McpConfig>, pool: PgPool) -> Self {
        Self { config, pool }
    }
}

#[async_trait]
impl ServerHandler for KruxiaFlowMcpHandler {
    async fn handle_list_tools_request(
        &self,
        _request: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        // Simple: expose all 13 tools when MCP is enabled
        let mut tools = Vec::new();

        tools.extend(tools::discovery::list_tools());
        tools.extend(tools::execution::list_tools());
        tools.extend(tools::observability::list_tools());
        tools.extend(tools::visualization::list_tools());
        tools.extend(tools::control::list_tools());

        Ok(ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        // Simple routing: all tools available when MCP is enabled
        match params.name.as_str() {
            // Discovery tools
            "list_workflow_definitions" => {
                tools::discovery::list_workflow_definitions(&self.pool, params).await
            }
            "get_workflow_definition" => {
                tools::discovery::get_workflow_definition(&self.pool, params).await
            }
            "list_activities" => {
                tools::discovery::list_activities(params).await
            }
            "get_workflow_authoring_guide" => {
                tools::discovery::get_workflow_authoring_guide(params).await
            }

            // Execution tools
            "validate_workflow" => {
                tools::execution::validate_workflow(params).await
            }
            "submit_workflow" => {
                tools::execution::submit_workflow(&self.pool, params).await
            }
            "cancel_workflow" => {
                tools::execution::cancel_workflow(&self.pool, params).await
            }

            // Observability tools
            "get_workflow_status" => {
                tools::observability::get_workflow_status(&self.pool, params).await
            }
            "list_workflows" => {
                tools::observability::list_workflows(&self.pool, params).await
            }
            "get_activity_output" => {
                tools::observability::get_activity_output(&self.pool, params).await
            }
            "get_workflow_cost" => {
                tools::observability::get_workflow_cost(&self.pool, params).await
            }
            "estimate_workflow_cost" => {
                tools::observability::estimate_workflow_cost(&self.pool, params).await
            }

            // Visualization tools
            "render_workflow_diagram" => {
                tools::visualization::render_workflow_diagram(&self.pool, params).await
            }
            "render_cost_diagram" => {
                tools::visualization::render_cost_diagram(&self.pool, params).await
            }

            // Control tools
            "send_workflow_signal" => {
                tools::control::send_workflow_signal(&self.pool, params).await
            }
            "list_waiting_workflows" => {
                tools::control::list_waiting_workflows(&self.pool, params).await
            }

            _ => Err(CallToolError::unknown_tool(params.name)),
        }
    }
}
```

## Integration with Serve Command

```rust
// kruxiaflow/src/commands/serve.rs

#[derive(Parser)]
pub struct ServeCommand {
    // ... existing fields ...

    /// Enable MCP server (requires mcp-server feature at compile time)
    #[cfg(feature = "mcp-server")]
    #[arg(long, env = "KRUXIAFLOW_MCP_ENABLED")]
    pub mcp_enabled: Option<bool>,

    /// MCP transport (stdio, http)
    #[cfg(feature = "mcp-server")]
    #[arg(long, env = "KRUXIAFLOW_MCP_TRANSPORT")]
    pub mcp_transport: Option<String>,

    /// MCP HTTP port (required if transport=http)
    #[cfg(feature = "mcp-server")]
    #[arg(long, env = "KRUXIAFLOW_MCP_HTTP_PORT")]
    pub mcp_http_port: Option<u16>,

    /// MCP HTTP bind address
    #[cfg(feature = "mcp-server")]
    #[arg(long, env = "KRUXIAFLOW_MCP_HTTP_BIND")]
    pub mcp_http_bind: Option<String>,
}

pub async fn execute(cmd: ServeCommand, database_url: String) -> Result<()> {
    // ... existing setup ...

    // Initialize MCP server if feature is compiled in and enabled
    #[cfg(feature = "mcp-server")]
    {
        let mcp_config = crate::mcp::config::McpConfig::new(
            cmd.mcp_enabled,
            cmd.mcp_transport,
            cmd.mcp_http_port,
            cmd.mcp_http_bind,
        )?;

        if mcp_config.enabled {
            mcp_config.log_config();

            let mcp_server = crate::mcp::server::create_mcp_server(
                Arc::new(mcp_config),
                pool.clone(),
            ).await?;

            // Spawn MCP server task
            let mcp_handle = tokio::spawn(async move {
                mcp_server.start().await
            });

            // Add to service handles
            handles.push(mcp_handle);
        }
    }

    // Log if someone tries to use MCP without the feature
    #[cfg(not(feature = "mcp-server"))]
    {
        if std::env::var("KRUXIAFLOW_MCP_ENABLED").ok().and_then(|s| s.parse().ok()).unwrap_or(false) {
            tracing::warn!(
                "MCP server requested but not compiled in. \
                Rebuild with --features mcp-server to enable MCP support."
            );
        }
    }

    // ... rest of serve command ...
}
```

### Conditional Compilation in Module Structure

```rust
// kruxiaflow/src/lib.rs or kruxiaflow/src/main.rs

// Only compile MCP module if feature is enabled
#[cfg(feature = "mcp-server")]
pub mod mcp;
```

This ensures:
- **Zero code** compiled when feature disabled
- **Zero dependencies** included in binary
- **Helpful warning** if user tries to enable MCP at runtime without compile-time support
- **Smaller binary** for edge deployments (estimated 2-5MB reduction)

## Tool Implementation Pattern

Each tool follows this pattern:

```rust
// Example: kruxiaflow/src/mcp/tools/discovery.rs

use rust_mcp_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// List available workflow definitions
pub async fn list_workflow_definitions(
    pool: &PgPool,
    params: CallToolRequestParams,
) -> Result<CallToolResult, CallToolError> {
    // Parse parameters
    let args: ListWorkflowDefinitionsArgs = serde_json::from_value(params.arguments)
        .map_err(|e| CallToolError::invalid_params(e.to_string()))?;

    // Query database
    let definitions = query_workflow_definitions(pool, &args).await
        .map_err(|e| CallToolError::internal_error(e.to_string()))?;

    // Format response
    let response = serde_json::json!({
        "definitions": definitions,
        "total": definitions.len(),
        "limit": args.limit,
        "offset": args.offset,
    });

    Ok(CallToolResult::text_content(vec![
        serde_json::to_string_pretty(&response).unwrap()
    ]))
}

#[derive(Debug, Deserialize, Serialize)]
struct ListWorkflowDefinitionsArgs {
    namespace: Option<String>,
    limit: usize,
    offset: usize,
}

async fn query_workflow_definitions(
    pool: &PgPool,
    args: &ListWorkflowDefinitionsArgs,
) -> anyhow::Result<Vec<WorkflowDefinitionSummary>> {
    // Implementation using existing Kruxia Flow database queries
    todo!()
}
```

## Next Steps

This plan will be executed in phases:

1. **Phase 1 (Prompt 2)**: Implement core infrastructure (config, server, basic stdio)
2. **Phase 2 (Prompt 3)**: Discovery tools
3. **Phase 3 (Prompt 4)**: Execution tools
4. **Phase 4 (Prompt 5)**: Observability tools
5. **Phase 5 (Prompt 6)**: Visualization & control tools
6. **Phase 6 (Prompt 7)**: Production hardening (rate limiting, auth, HTTP transport, metrics)

## References

- [rust-mcp-sdk Documentation](https://github.com/rust-mcp-stack/rust-mcp-sdk)
- [MCP Protocol Spec](https://modelcontextprotocol.io/)
- Python MCP implementation at `kruxiaflow-mcp/` for reference
