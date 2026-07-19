use thiserror::Error;

pub type Result<T> = std::result::Result<T, McpError>;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("MCP stdio transport is not supported in the integrated server: {0}")]
    UnsupportedTransport(String),

    #[error("Server error: {0}")]
    ServerError(String),
}
