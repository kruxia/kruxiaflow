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
pub mod control;

pub use discovery::DiscoveryTools;
pub use execution::ExecutionTools;
pub use observability::ObservabilityTools;
pub use visualization::VisualizationTools;
pub use control::ControlTools;
