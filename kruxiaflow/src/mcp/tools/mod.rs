pub mod control;
/// MCP tool registry
///
/// Tools are organised into categories, each in its own module.
/// Each module uses `tool_box!` to generate an enum that provides:
///   - `::tools()` → Vec<Tool> for the list_tools MCP response
///   - `TryFrom<CallToolRequestParams>` → parse an incoming call (succeeds only if the name matches)
pub mod discovery;
pub mod execution;
pub mod observability;
pub mod visualization;

pub use control::ControlTools;
pub use discovery::DiscoveryTools;
pub use execution::ExecutionTools;
pub use observability::ObservabilityTools;
pub use visualization::VisualizationTools;

use rust_mcp_sdk::schema::{CallToolResult, TextContent, schema_utils::CallToolError};

/// Wrap a JSON value as a pretty-printed text response.
pub(crate) fn text_response(value: &serde_json::Value) -> Result<CallToolResult, CallToolError> {
    Ok(CallToolResult::text_content(vec![TextContent::from(
        serde_json::to_string_pretty(value)
            .map_err(|e| CallToolError::from_message(e.to_string()))?,
    )]))
}

/// Parse a string as UUID, returning a tool error if invalid.
pub(crate) fn parse_uuid(s: &str) -> Result<uuid::Uuid, CallToolError> {
    uuid::Uuid::parse_str(s).map_err(|_| {
        CallToolError::from_message(format!("Invalid workflow_id '{}': must be a valid UUID", s))
    })
}
