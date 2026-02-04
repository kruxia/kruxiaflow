/// MCP ServerHandler implementation
///
/// Routes incoming MCP requests to the appropriate tool functions.
/// Tool modules are added incrementally as they are implemented:
///   - Discovery tools: list/get workflow definitions, activity catalog, authoring guide
///   - Execution tools: validate, submit, cancel
///   - Observability tools: status, list, outputs, cost, estimate
///   - Visualization & Control tools: diagrams, signals (future)
use std::sync::Arc;

use async_trait::async_trait;
use rust_mcp_sdk::{
    McpServer,
    mcp_server::ServerHandler,
    schema::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
        schema_utils::CallToolError,
    },
};
use sqlx::PgPool;

use super::tools::{
    ControlTools, DiscoveryTools, ExecutionTools, ObservabilityTools, VisualizationTools, control,
    discovery, execution, observability, visualization,
};
use crate::mcp::config::McpConfig;

/// Handler that dispatches MCP tool calls to Kruxia Flow services.
pub struct KruxiaFlowMcpHandler {
    pub config: Arc<McpConfig>,
    pub pool: PgPool,
}

impl KruxiaFlowMcpHandler {
    pub fn new(config: Arc<McpConfig>, pool: PgPool) -> Self {
        Self { config, pool }
    }
}

#[async_trait]
impl ServerHandler for KruxiaFlowMcpHandler {
    /// Return the list of all available MCP tools.
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: [
                DiscoveryTools::tools(),
                ExecutionTools::tools(),
                ObservabilityTools::tools(),
                VisualizationTools::tools(),
                ControlTools::tools(),
            ]
            .concat(),
        })
    }

    /// Route an incoming tool call to the correct implementation.
    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        tracing::debug!(tool = %params.name, "MCP tool call received");

        // Route by name first so we consume `params` in exactly one try_from call
        // (CallToolRequestParams does not implement Clone).
        match params.name.as_str() {
            // --- Discovery tools ---
            "list_workflow_definitions"
            | "get_workflow_definition"
            | "list_activities"
            | "get_workflow_authoring_guide" => {
                let tool = DiscoveryTools::try_from(params).map_err(CallToolError::new)?;
                match tool {
                    DiscoveryTools::ListWorkflowDefinitions(ref p) => {
                        discovery::run_list_workflow_definitions(&self.pool, p).await
                    }
                    DiscoveryTools::GetWorkflowDefinition(ref p) => {
                        discovery::run_get_workflow_definition(&self.pool, p).await
                    }
                    DiscoveryTools::ListActivities(ref p) => p.call_tool(),
                    DiscoveryTools::GetWorkflowAuthoringGuide(ref p) => p.call_tool(),
                }
            }

            // --- Execution tools ---
            "validate_workflow" | "submit_workflow" | "cancel_workflow" => {
                let tool = ExecutionTools::try_from(params).map_err(CallToolError::new)?;
                match tool {
                    ExecutionTools::ValidateWorkflow(ref p) => p.call_tool(),
                    ExecutionTools::SubmitWorkflow(ref p) => {
                        execution::run_submit_workflow(&self.pool, p).await
                    }
                    ExecutionTools::CancelWorkflow(ref p) => {
                        execution::run_cancel_workflow(&self.pool, p).await
                    }
                }
            }

            // --- Observability tools ---
            "get_workflow_status"
            | "list_workflows"
            | "get_activity_output"
            | "get_workflow_cost"
            | "estimate_workflow_cost" => {
                let tool = ObservabilityTools::try_from(params).map_err(CallToolError::new)?;
                match tool {
                    ObservabilityTools::GetWorkflowStatus(ref p) => {
                        observability::run_get_workflow_status(&self.pool, p).await
                    }
                    ObservabilityTools::ListWorkflows(ref p) => {
                        observability::run_list_workflows(&self.pool, p).await
                    }
                    ObservabilityTools::GetActivityOutput(ref p) => {
                        observability::run_get_activity_output(&self.pool, p).await
                    }
                    ObservabilityTools::GetWorkflowCost(ref p) => {
                        observability::run_get_workflow_cost(&self.pool, p).await
                    }
                    ObservabilityTools::EstimateWorkflowCost(ref p) => {
                        observability::run_estimate_workflow_cost(&self.pool, p).await
                    }
                }
            }

            // --- Visualization tools ---
            "render_workflow_diagram" | "render_cost_diagram" => {
                let tool = VisualizationTools::try_from(params).map_err(CallToolError::new)?;
                match tool {
                    VisualizationTools::RenderWorkflowDiagram(ref p) => {
                        visualization::run_render_workflow_diagram(&self.pool, p).await
                    }
                    VisualizationTools::RenderCostDiagram(ref p) => {
                        visualization::run_render_cost_diagram(&self.pool, p).await
                    }
                }
            }

            // --- Control tools ---
            "send_workflow_signal" | "list_waiting_workflows" => {
                let tool = ControlTools::try_from(params).map_err(CallToolError::new)?;
                match tool {
                    ControlTools::SendWorkflowSignal(ref p) => {
                        control::run_send_workflow_signal(&self.pool, p).await
                    }
                    ControlTools::ListWaitingWorkflows(ref p) => {
                        control::run_list_waiting_workflows(&self.pool, p).await
                    }
                }
            }

            _ => Err(CallToolError::from_message(format!(
                "Unknown tool: '{}'",
                params.name
            ))),
        }
    }
}
