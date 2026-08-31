"""Visualization tools for rendering workflow diagrams."""

from typing import Any

from fastmcp import Context

from ..client import KruxiaFlowClient
from ..utils.mermaid import generate_cost_breakdown_diagram, generate_workflow_diagram


def register_visualization_tools(mcp: Any, client: KruxiaFlowClient) -> None:
    """Register visualization tools with the MCP server.

    Args:
        mcp: FastMCP server instance
        client: Kruxia Flow API client
    """

    @mcp.tool()
    async def render_workflow_diagram(
        definition_name: str | None = None,
        workflow_id: str | None = None,
        include_status: bool = True,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Generate a Mermaid flowchart diagram for a workflow.

        Creates a visual representation of a workflow showing activities
        and their dependencies. Can generate diagrams from either a workflow
        definition (structure only) or an execution (with status colors).

        Args:
            definition_name: Name of workflow definition to visualize
            workflow_id: ID of workflow execution to visualize (with status)
            include_status: If True and workflow_id provided, color nodes by status

        Returns:
            Dictionary containing:
            - diagram: Mermaid flowchart syntax
            - format: "mermaid"
            - type: "flowchart"

        Node Colors (when include_status=True):
            - Green: Completed activities
            - Gold: Running activities
            - Red: Failed activities
            - Gray: Skipped activities
            - Sky Blue: Pending activities

        Example Usage:
            # Visualize workflow structure
            diagram = await render_workflow_diagram(
                definition_name="research_assistant"
            )

            # Visualize execution with status
            diagram = await render_workflow_diagram(
                workflow_id="019353a1-b0c1-7000-8000-000000000001",
                include_status=True
            )

        Rendering in Claude Code:
            Claude Code and Claude Desktop will automatically render the
            Mermaid diagram inline when you display the diagram text.
        """
        workflow_def = None
        execution_status = None

        # Get workflow definition
        if definition_name:
            workflow_def = await client.get_workflow_definition(definition_name)
        elif workflow_id:
            # Get execution details which include the definition
            execution_status = await client.get_workflow(
                workflow_id=workflow_id,
                include_activities=include_status,
            )
            # Extract definition from execution
            # (The API may include definition in execution response)
            # For now, we'll fetch it separately if definition_name is available
            if "definition_name" in execution_status:
                workflow_def = await client.get_workflow_definition(
                    execution_status["definition_name"]
                )
        else:
            return {
                "error": "Must provide either definition_name or workflow_id",
                "diagram": "",
                "format": "mermaid",
                "type": "flowchart",
            }

        if not workflow_def:
            return {
                "error": "Could not retrieve workflow definition",
                "diagram": "",
                "format": "mermaid",
                "type": "flowchart",
            }

        # Generate Mermaid diagram
        diagram = generate_workflow_diagram(
            workflow_def=workflow_def,
            execution_status=execution_status if include_status else None,
        )

        return {
            "diagram": diagram,
            "format": "mermaid",
            "type": "flowchart",
            "workflow_name": workflow_def.get("name"),
            "activity_count": len(workflow_def.get("activities", [])),
        }

    @mcp.tool()
    async def render_cost_diagram(
        workflow_id: str,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Generate a Mermaid diagram showing cost breakdown by activity.

        Creates a visual representation of how costs are distributed across
        activities in a workflow execution. Useful for identifying expensive
        operations and optimizing costs.

        Args:
            workflow_id: Unique identifier of the workflow execution

        Returns:
            Dictionary containing:
            - diagram: Mermaid graph syntax
            - format: "mermaid"
            - type: "graph"
            - total_cost_usd: Total workflow cost

        Example Usage:
            cost_diagram = await render_cost_diagram(
                workflow_id="019353a1-b0c1-7000-8000-000000000001"
            )
            print(cost_diagram["diagram"])

        Cost Insights:
            - Quickly identify which activities are most expensive
            - See provider breakdown (Anthropic, OpenAI, etc.)
            - Compare costs across workflow executions
            - Make informed decisions about model selection
        """
        # Get cost data
        cost_data = await client.get_workflow_cost(workflow_id=workflow_id)

        # Generate diagram
        diagram = generate_cost_breakdown_diagram(cost_data)

        return {
            "diagram": diagram,
            "format": "mermaid",
            "type": "graph",
            "total_cost_usd": cost_data.get("total_cost_usd", 0.0),
            "activity_count": len(cost_data.get("activities", [])),
        }
