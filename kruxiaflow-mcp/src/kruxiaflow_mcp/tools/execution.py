"""Execution tools for validating, submitting, and canceling workflows."""

from typing import Any

import yaml
from fastmcp import Context

from ..client import KruxiaFlowClient


def register_execution_tools(mcp: Any, client: KruxiaFlowClient) -> None:
    """Register execution tools with the MCP server.

    Args:
        mcp: FastMCP server instance
        client: Kruxia Flow API client
    """

    @mcp.tool()
    async def validate_workflow(
        workflow_yaml: str,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Validate a workflow definition without submitting it for execution.

        Checks the workflow YAML for syntax errors, invalid activity types,
        circular dependencies, and other structural issues. Use this before
        submitting to catch errors early.

        Args:
            workflow_yaml: Complete workflow definition in YAML format

        Returns:
            Dictionary containing validation results:
            - valid: Boolean indicating if workflow is valid
            - errors: List of validation errors (if any)
            - warnings: List of warnings (if any)
            - activities: Count of activities in the workflow
            - dependencies: Information about the dependency graph

        Example Input:
            ```yaml
            name: my_workflow
            activities:
              - key: fetch_data
                activity_name: http_request
                parameters:
                  url: "https://api.example.com/data"
            ```

        Example Output:
            {
                "valid": true,
                "errors": [],
                "warnings": [],
                "activities": 1,
                "dependencies": {"fetch_data": []}
            }
        """
        # Parse YAML to validate syntax
        try:
            workflow_def = yaml.safe_load(workflow_yaml)
        except yaml.YAMLError as e:
            return {
                "valid": False,
                "errors": [f"Invalid YAML syntax: {e!s}"],
                "warnings": [],
                "activities": 0,
                "dependencies": {},
            }

        # Submit to Kruxia Flow API for validation (using dry_run or validate endpoint)
        # Note: The API may not have a dedicated validate endpoint yet,
        # so we'll do basic client-side validation for now
        try:
            # Basic structure validation
            if not isinstance(workflow_def, dict):
                return {
                    "valid": False,
                    "errors": ["Workflow must be a dictionary"],
                    "warnings": [],
                    "activities": 0,
                    "dependencies": {},
                }

            if "name" not in workflow_def:
                return {
                    "valid": False,
                    "errors": ["Workflow must have a 'name' field"],
                    "warnings": [],
                    "activities": 0,
                    "dependencies": {},
                }

            if "activities" not in workflow_def:
                return {
                    "valid": False,
                    "errors": ["Workflow must have an 'activities' field"],
                    "warnings": [],
                    "activities": 0,
                    "dependencies": {},
                }

            activities = workflow_def.get("activities", [])
            if not isinstance(activities, list):
                return {
                    "valid": False,
                    "errors": ["'activities' must be a list"],
                    "warnings": [],
                    "activities": 0,
                    "dependencies": {},
                }

            # Build dependency map
            dependencies: dict[str, list[str]] = {}
            activity_keys = set()
            errors = []
            warnings = []

            for activity in activities:
                if not isinstance(activity, dict):
                    errors.append("Each activity must be a dictionary")
                    continue

                key = activity.get("key")
                if not key:
                    errors.append("Each activity must have a 'key' field")
                    continue

                if key in activity_keys:
                    errors.append(f"Duplicate activity key: {key}")
                    continue

                activity_keys.add(key)

                if "activity_name" not in activity:
                    errors.append(f"Activity '{key}' missing 'activity_name' field")

                # Check for depends_on
                depends_on = activity.get("depends_on", [])
                if depends_on and not isinstance(depends_on, list):
                    errors.append(f"Activity '{key}': 'depends_on' must be a list")
                    depends_on = []

                dependencies[key] = depends_on if isinstance(depends_on, list) else []

            # Check for undefined dependencies
            for key, deps in dependencies.items():
                for dep in deps:
                    if dep not in activity_keys:
                        errors.append(f"Activity '{key}' depends on undefined activity '{dep}'")

            # Check for circular dependencies (simple cycle detection)
            def has_cycle(node: str, visited: set[str], rec_stack: set[str]) -> bool:
                visited.add(node)
                rec_stack.add(node)

                for neighbor in dependencies.get(node, []):
                    if neighbor not in visited:
                        if has_cycle(neighbor, visited, rec_stack):
                            return True
                    elif neighbor in rec_stack:
                        return True

                rec_stack.remove(node)
                return False

            visited: set[str] = set()
            for activity_key in activity_keys:
                if activity_key not in visited and has_cycle(activity_key, visited, set()):
                    errors.append("Workflow contains circular dependencies")
                    break

            return {
                "valid": len(errors) == 0,
                "errors": errors,
                "warnings": warnings,
                "activities": len(activities),
                "dependencies": dependencies,
            }

        except Exception as e:
            return {
                "valid": False,
                "errors": [f"Validation error: {e!s}"],
                "warnings": [],
                "activities": 0,
                "dependencies": {},
            }

    @mcp.tool()
    async def submit_workflow(
        definition_name: str,
        input: dict[str, Any],
        budget_limit_usd: float | None = None,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Submit a workflow for execution.

        Submits a workflow definition for immediate execution. The workflow
        will be validated first, then queued for execution by the orchestrator.
        Activities will be scheduled based on their dependencies.

        Args:
            definition_name: Name of the workflow definition to execute
            input: Input parameters for the workflow (must match workflow's expected inputs)
            budget_limit_usd: Optional budget limit in USD (workflow will abort if exceeded)

        Returns:
            Dictionary containing:
            - workflow_id: Unique identifier for this workflow execution
            - status: Initial status (usually "pending" or "running")
            - definition_name: Name of the workflow definition
            - submitted_at: Timestamp when workflow was submitted

        Example:
            result = await submit_workflow(
                definition_name="weather_report",
                input={"city": "San Francisco", "state": "CA"},
                budget_limit_usd=0.10
            )
            # Returns: {"workflow_id": "019353a1-b0c1-7000-8000-000000000001", ...}
        """
        # Prepare submission payload
        payload: dict[str, Any] = {
            "workflow_definition": definition_name,
            "input": input,
        }

        # Add budget limit if specified
        if budget_limit_usd is not None:
            payload["budget_limit_usd"] = budget_limit_usd

        # Submit to Kruxia Flow API
        result = await client.post("/api/v1/workflows", json=payload)

        return result

    @mcp.tool()
    async def cancel_workflow(
        workflow_id: str,
        reason: str | None = None,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Cancel a running workflow.

        Stops a workflow that is currently executing. All running activities
        will be allowed to complete, but no new activities will be started.
        The workflow status will be set to "canceled".

        Args:
            workflow_id: Unique identifier of the workflow to cancel
            reason: Optional reason for cancellation (for audit logging)

        Returns:
            Dictionary containing:
            - workflow_id: ID of the canceled workflow
            - status: New status ("canceled" or "canceling")
            - message: Confirmation message

        Example:
            result = await cancel_workflow(
                workflow_id="019353a1-b0c1-7000-8000-000000000001",
                reason="User requested cancellation"
            )
        """
        # Prepare cancellation payload
        payload: dict[str, Any] = {}
        if reason:
            payload["reason"] = reason

        # Send cancellation request to API
        result = await client.post(f"/api/v1/workflows/{workflow_id}/cancel", json=payload)

        return result
