/// MCP (Model Context Protocol) server module
///
/// This module provides an integrated MCP server that enables AI agents
/// to interact with Kruxia Flow through a standardized protocol.
///
/// The MCP server is opt-in at compile time (requires `mcp-server` feature)
/// to keep the default binary lean for edge deployments.
pub mod config;
pub mod error;
pub mod handler;
pub mod server;
pub mod tools;

pub use config::McpConfig;
pub use server::spawn_mcp_server;
