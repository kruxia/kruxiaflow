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
/// **For stdio-only MCP clients:**
/// Use a thin stdio→HTTP proxy (e.g. `mcp-remote`) pointed at the HTTP endpoint.
/// Modern clients (Claude Code, Cursor) speak HTTP MCP natively.
///
/// **This HTTP-only MCP server is designed for:**
/// - Production deployments with full observability
/// - Multi-client AI agent access over network
/// - Coexistence with API server and other services
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::task::JoinHandle;

use super::error::{McpError, Result};
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

use super::config::McpConfig;

/// MCP server instance
pub struct McpServer {
    config: Arc<McpConfig>,
    pool: PgPool,
    cache_service: Arc<dyn kruxiaflow_core::CacheService>,
    auth_service: Option<Arc<dyn kruxiaflow_oauth::AuthenticationService>>,
}

impl McpServer {
    /// Create a new MCP server instance
    pub fn new(
        config: Arc<McpConfig>,
        pool: PgPool,
        cache_service: Arc<dyn kruxiaflow_core::CacheService>,
        auth_service: Option<Arc<dyn kruxiaflow_oauth::AuthenticationService>>,
    ) -> Self {
        Self {
            config,
            pool,
            cache_service,
            auth_service,
        }
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

        let handler = super::handler::KruxiaFlowMcpHandler::new(
            self.config.clone(),
            self.pool.clone(),
            self.cache_service.clone(),
        )
        .to_mcp_server_handler();

        let auth: Option<Arc<dyn AuthProvider>> = self
            .auth_service
            .map(|svc| -> Arc<dyn AuthProvider> { Arc::new(McpAuthAdapter { auth_service: svc }) });

        // TODO: Add rate limiting and request timeouts when rust-mcp-sdk
        // supports middleware configuration in HyperServerOptions.
        let options = HyperServerOptions {
            host: bind,
            port,
            auth,
            ..Default::default()
        };

        tracing::info!("MCP server listening on {}:{}", options.host, options.port);

        let server = hyper_server::create_server(server_info, handler, options);
        server
            .start()
            .await
            .map_err(|e| McpError::ServerError(format!("MCP server: {e}")))?;

        Ok(())
    }
}

/// Create and configure MCP server
///
/// This function creates an MCP server instance but does not start it.
/// Call `start()` on the returned server to begin serving MCP requests.
pub fn create_mcp_server(
    config: Arc<McpConfig>,
    pool: PgPool,
    cache_service: Arc<dyn kruxiaflow_core::CacheService>,
    auth_service: Option<Arc<dyn kruxiaflow_oauth::AuthenticationService>>,
) -> McpServer {
    McpServer::new(config, pool, cache_service, auth_service)
}

/// Spawn MCP server in a background task
///
/// This is the recommended way to run the MCP server alongside other services.
/// Pass the same `AuthenticationService` used by the REST API so that a single
/// token (RS256, issued via `/oauth/token`) works across both protocols, and the
/// same `CacheService` so cache management tools act on the live cache.
pub fn spawn_mcp_server(
    config: Arc<McpConfig>,
    pool: PgPool,
    cache_service: Arc<dyn kruxiaflow_core::CacheService>,
    auth_service: Option<Arc<dyn kruxiaflow_oauth::AuthenticationService>>,
) -> JoinHandle<Result<()>> {
    let server = create_mcp_server(config, pool, cache_service, auth_service);
    tokio::spawn(async move { server.start().await })
}

// ---------------------------------------------------------------------------
// Auth adapter — delegates to the project's AuthenticationService
// ---------------------------------------------------------------------------

/// Auth provider that delegates token validation to the project's
/// `AuthenticationService` (RS256 via `kruxiaflow_oauth`).
///
/// This ensures a single token works across both the REST API and MCP server.
/// Tokens are issued via the existing OAuth2 endpoints (`POST /oauth/token`).
struct McpAuthAdapter {
    auth_service: Arc<dyn kruxiaflow_oauth::AuthenticationService>,
}

#[async_trait]
impl AuthProvider for McpAuthAdapter {
    /// Validate the token via `AuthenticationService::validate_token` and map
    /// the returned `Claims` to the SDK's `AuthInfo`.
    async fn verify_token(
        &self,
        access_token: String,
    ) -> std::result::Result<AuthInfo, AuthenticationError> {
        let claims = self
            .auth_service
            .validate_token(&access_token)
            .await
            .map_err(|e| AuthenticationError::TokenVerificationFailed {
                description: format!("Token validation failed: {e}"),
                status_code: None,
            })?;

        let expires_at = Some(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(claims.exp as u64),
        );

        Ok(AuthInfo {
            token_unique_id: claims.jti,
            client_id: None,
            user_id: Some(claims.sub),
            scopes: None,
            expires_at,
            audience: Some(rust_mcp_sdk::auth::Audience::Single(claims.aud)),
            extra: None,
        })
    }

    /// No OAuth endpoints — tokens are issued by the REST API's `/oauth/token`.
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
                "No auth endpoints configured — use the REST API's /oauth/token to obtain tokens"
                    .to_string(),
            ))
            .unwrap())
    }

    fn protected_resource_metadata_url(&self) -> Option<&str> {
        None
    }
}
