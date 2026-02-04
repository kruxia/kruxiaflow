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
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;
use tokio::task::JoinHandle;

use rust_mcp_sdk::{
    auth::{AuthInfo, AuthProvider, AuthenticationError, OauthEndpoint},
    mcp_http::{GenericBody, GenericBodyExt},
    mcp_server::{
        HyperServerOptions, McpAppState, ToMcpServerHandler, error::TransportServerError,
        hyper_server,
    },
    schema::{
        Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
        ServerCapabilitiesTools,
    },
};

use crate::mcp::config::McpConfig;

/// MCP server instance
pub struct McpServer {
    config: Arc<McpConfig>,
    pool: PgPool,
}

impl McpServer {
    /// Create a new MCP server instance
    pub fn new(config: Arc<McpConfig>, pool: PgPool) -> Self {
        Self { config, pool }
    }

    /// Start the MCP server (HTTP transport only)
    pub async fn start(self) -> Result<()> {
        tracing::info!("Starting MCP HTTP server...");
        self.start_http().await
    }

    /// Start MCP server with HTTP transport
    async fn start_http(self) -> Result<()> {
        let port = self.config.http_port.expect("HTTP transport requires port");
        let bind = self
            .config
            .http_bind
            .clone()
            .expect("HTTP transport requires bind address");

        let server_info = InitializeResult {
            server_info: Implementation {
                name: "kruxiaflow-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("Kruxia Flow MCP Server".into()),
                description: Some("MCP server for Kruxia Flow workflow orchestration".into()),
                icons: vec![],
                website_url: None,
            },
            capabilities: ServerCapabilities {
                tools: Some(ServerCapabilitiesTools { list_changed: None }),
                ..Default::default()
            },
            protocol_version: ProtocolVersion::V2025_11_25.into(),
            instructions: Some(
                "Use the available tools to discover, submit, monitor, \
                 and control Kruxia Flow workflows."
                    .into(),
            ),
            meta: None,
        };

        let handler =
            super::handler::KruxiaFlowMcpHandler::new(self.config.clone(), self.pool.clone())
                .to_mcp_server_handler();

        let options = HyperServerOptions {
            host: bind,
            port,
            auth: build_auth_provider(&self.config),
            ..Default::default()
        };

        tracing::info!("MCP server listening on {}:{}", options.host, options.port);

        let server = hyper_server::create_server(server_info, handler, options);
        server
            .start()
            .await
            .map_err(|e| anyhow::anyhow!("MCP server: {e}"))?;

        Ok(())
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

// ---------------------------------------------------------------------------
// Auth provider
// ---------------------------------------------------------------------------

/// Build an auth provider if auth is required and a JWT secret is configured.
fn build_auth_provider(config: &McpConfig) -> Option<Arc<dyn AuthProvider>> {
    if !config.auth_required {
        return None;
    }
    let secret = config.jwt_secret.as_ref()?;
    Some(Arc::new(McpJwtAuthProvider {
        secret: secret.clone(),
    }))
}

/// Bearer-token auth provider using HS256 JWT validation.
///
/// The SDK's AuthMiddleware handles Bearer-token extraction and expiry checking.
/// We only need to decode the token and map claims to AuthInfo.
struct McpJwtAuthProvider {
    secret: String,
}

#[async_trait]
impl AuthProvider for McpJwtAuthProvider {
    /// Decode the JWT and return extracted claims as AuthInfo.
    async fn verify_token(
        &self,
        access_token: String,
    ) -> std::result::Result<AuthInfo, AuthenticationError> {
        let token_data = jsonwebtoken::decode::<serde_json::Value>(
            &access_token,
            &jsonwebtoken::DecodingKey::from_secret(self.secret.as_bytes()),
            &jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256),
        )
        .map_err(|e| AuthenticationError::TokenVerificationFailed {
            description: format!("JWT validation failed: {e}"),
            status_code: None,
        })?;

        let claims = &token_data.claims;

        let expires_at = claims.get("exp").and_then(|v| v.as_i64()).map(|exp| {
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(exp as u64)
        });

        let token_unique_id = claims
            .get("jti")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or(access_token);

        Ok(AuthInfo {
            token_unique_id,
            client_id: None,
            user_id: claims
                .get("sub")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            scopes: None,
            expires_at,
            audience: None,
            extra: None,
        })
    }

    /// No OAuth endpoints — this server only validates Bearer tokens.
    fn auth_endpoints(&self) -> Option<&HashMap<String, OauthEndpoint>> {
        None
    }

    /// Not called when auth_endpoints() returns None.
    async fn handle_request(
        &self,
        _request: http::Request<&str>,
        _state: Arc<McpAppState>,
    ) -> std::result::Result<http::Response<GenericBody>, TransportServerError> {
        Ok(http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .body(GenericBody::from_string(
                "No auth endpoints configured".to_string(),
            ))
            .unwrap())
    }

    fn protected_resource_metadata_url(&self) -> Option<&str> {
        None
    }
}
