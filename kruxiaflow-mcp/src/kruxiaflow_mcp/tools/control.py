"""Control tools for workflow signal handling and human-in-the-loop interaction."""

from typing import Any

from fastmcp import Context

from ..client import KruxiaFlowClient


def register_control_tools(mcp: Any, client: KruxiaFlowClient) -> None:
    """Register control tools with the MCP server.

    Args:
        mcp: FastMCP server instance
        client: Kruxia Flow API client
    """

    @mcp.tool()
    async def send_workflow_signal(
        workflow_id: str,
        signal_name: str,
        signal_data: dict[str, Any] | None = None,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Send a signal to a workflow waiting for human input.

        Workflows can include activities that wait for external signals before
        continuing execution. This tool allows AI agents to send signals to
        resume workflow execution with provided data.

        Args:
            workflow_id: Unique identifier of the workflow execution
            signal_name: Name of the signal the workflow is waiting for
            signal_data: Optional data to send with the signal (available to downstream activities)

        Returns:
            Dictionary containing:
            - workflow_id: Workflow identifier
            - signal_name: Name of the signal that was sent
            - received_at: Timestamp when signal was received
            - status: Updated workflow status

        Human-in-the-Loop Pattern:
            Workflows can pause and wait for signals using special activities:
            ```yaml
            - key: wait_for_approval
              activity_name: wait_for_signal
              parameters:
                signal_name: "user_approval"
                timeout_seconds: 3600
            ```

        Example Usage:
            # Workflow is waiting for approval
            result = await send_workflow_signal(
                workflow_id="019353a1-b0c1-7000-8000-000000000001",
                signal_name="user_approval",
                signal_data={"approved": True, "comments": "Looks good!"}
            )

        Use Cases:
            - Manual approval gates in deployment workflows
            - Quality review checkpoints
            - User input for interactive agents
            - Escalation to human experts
            - A/B testing with manual selection
        """
        payload: dict[str, Any] = {
            "signal_name": signal_name,
        }

        if signal_data:
            payload["signal_data"] = signal_data

        result = await client.post(f"/api/v1/workflows/{workflow_id}/signal", json=payload)

        return result

    @mcp.tool()
    async def list_waiting_workflows(
        signal_name: str | None = None,
        limit: int = 20,
        offset: int = 0,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """List workflows that are waiting for signals.

        Finds workflows that are paused and waiting for external signals
        before they can continue execution. Useful for identifying workflows
        that need human attention or approval.

        Args:
            signal_name: Optional filter by specific signal name
            limit: Maximum number of workflows to return (default 20)
            offset: Number of workflows to skip for pagination (default 0)

        Returns:
            Dictionary containing:
            - workflows: List of waiting workflows
            - total: Total count of waiting workflows
            - limit: Requested limit
            - offset: Requested offset

        Workflow Information:
            Each workflow in the list includes:
            - workflow_id: Unique identifier
            - definition_name: Workflow definition name
            - waiting_since: Timestamp when workflow started waiting
            - signal_name: Name of the signal it's waiting for
            - timeout_at: When the wait will timeout (if configured)
            - context: Additional context about why it's waiting

        Example Usage:
            # Find all workflows waiting for approval
            waiting = await list_waiting_workflows(signal_name="user_approval")

            # Check all waiting workflows
            all_waiting = await list_waiting_workflows()

        Typical Workflow:
            1. Agent discovers waiting workflows
            2. Agent examines workflow context and decides action
            3. Agent sends signal to resume workflow (or escalates to human)
        """
        params: dict[str, Any] = {
            "status": "waiting",  # Filter for waiting workflows
            "limit": limit,
            "offset": offset,
        }

        if signal_name:
            params["signal_name"] = signal_name

        # Use the list_workflows endpoint with waiting status filter
        result = await client.list_workflows(
            status="waiting",
            limit=limit,
            offset=offset,
        )

        # Add signal_name filter if provided (client-side filtering if API doesn't support it)
        if signal_name and "workflows" in result:
            filtered_workflows = [
                wf for wf in result["workflows"] if wf.get("waiting_for_signal") == signal_name
            ]
            result["workflows"] = filtered_workflows
            result["total"] = len(filtered_workflows)

        return result
