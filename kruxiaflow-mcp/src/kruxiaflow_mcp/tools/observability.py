"""Observability tools for monitoring workflow status, outputs, and costs."""

from typing import Any

from fastmcp import Context

from ..client import KruxiaFlowClient


def register_observability_tools(mcp: Any, client: KruxiaFlowClient) -> None:
    """Register observability tools with the MCP server.

    Args:
        mcp: FastMCP server instance
        client: Kruxia Flow API client
    """

    @mcp.tool()
    async def get_workflow_status(
        workflow_id: str,
        include_activities: bool = False,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Get the current status of a workflow execution.

        Retrieves detailed status information for a specific workflow execution,
        including overall status, timestamps, and optionally all activity details.

        Args:
            workflow_id: Unique identifier of the workflow execution
            include_activities: If True, include detailed status for all activities

        Returns:
            Dictionary containing:
            - workflow_id: Workflow identifier
            - status: Current status (pending, running, completed, failed, canceled)
            - definition_name: Name of the workflow definition
            - started_at: When workflow execution began
            - completed_at: When workflow finished (if completed)
            - activities: List of activity details (if include_activities=True)

        Activity Status:
            - pending: Not yet started
            - running: Currently executing
            - completed: Finished successfully
            - failed: Encountered an error
            - skipped: Skipped due to conditional logic

        Example:
            status = await get_workflow_status(
                workflow_id="019353a1-b0c1-7000-8000-000000000001",
                include_activities=True
            )
        """
        return await client.get_workflow(
            workflow_id=workflow_id,
            include_activities=include_activities,
        )

    @mcp.tool()
    async def list_workflows(
        status: str | None = None,
        limit: int = 20,
        offset: int = 0,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """List workflow executions with optional status filtering.

        Retrieves a paginated list of workflow executions. Use status filter
        to find workflows in specific states (e.g., all running workflows).

        Args:
            status: Optional status filter (pending, running, completed, failed, canceled)
            limit: Maximum number of workflows to return (default 20)
            offset: Number of workflows to skip for pagination (default 0)

        Returns:
            Dictionary containing:
            - workflows: List of workflow summaries
            - total: Total count of workflows matching filter
            - limit: Requested limit
            - offset: Requested offset

        Example:
            # List all running workflows
            running = await list_workflows(status="running")

            # List recent completions
            recent = await list_workflows(status="completed", limit=10)
        """
        return await client.list_workflows(
            status=status,
            limit=limit,
            offset=offset,
        )

    @mcp.tool()
    async def get_activity_output(
        workflow_id: str,
        activity_key: str,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Get the output of a specific activity in a workflow.

        Retrieves the results produced by an activity after it completes.
        Each activity can produce multiple named outputs that can be referenced
        by downstream activities.

        Args:
            workflow_id: Unique identifier of the workflow execution
            activity_key: Key of the activity (as defined in workflow)

        Returns:
            Dictionary containing the activity's output:
            - For http_request: {response: {...}, status_code: 200, headers: {...}}
            - For llm_prompt: {result: {...}, cost_usd: 0.015, provider: "anthropic", ...}
            - For postgres_query: {rows: [...], row_count: 10}
            - For embedding: {embeddings: [...], dimensions: 1536, cost_usd: 0.00002}

        Example:
            output = await get_activity_output(
                workflow_id="019353a1-b0c1-7000-8000-000000000001",
                activity_key="fetch_weather"
            )
            # Access specific fields: output["response"]["json"]["temperature"]
        """
        return await client.get_activity_output(
            workflow_id=workflow_id,
            activity_key=activity_key,
        )

    @mcp.tool()
    async def get_workflow_cost(
        workflow_id: str,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Get the cost breakdown for a workflow execution.

        Retrieves detailed cost information including per-activity costs,
        total cost, and budget utilization. Costs are tracked for LLM API calls,
        embedding generation, and other metered services.

        Args:
            workflow_id: Unique identifier of the workflow execution

        Returns:
            Dictionary containing:
            - total_cost_usd: Total cost across all activities
            - budget_limit_usd: Budget limit (if set)
            - budget_used_percent: Percentage of budget consumed
            - activities: Per-activity cost breakdown
            - providers: Cost breakdown by provider (anthropic, openai, etc.)

        Activity Cost Details:
            - activity_key: Activity identifier
            - cost_usd: Cost for this activity
            - provider: Service provider (e.g., "anthropic", "openai")
            - model: Specific model used (e.g., "claude-sonnet-4-5-20250929")
            - tokens: Token usage (prompt_tokens, output_tokens, total_tokens)

        Example:
            cost = await get_workflow_cost("019353a1-b0c1-7000-8000-000000000001")
            print(f"Total: ${cost['total_cost_usd']:.4f}")
            print(f"Budget: {cost['budget_used_percent']:.1f}%")
        """
        return await client.get_workflow_cost(workflow_id=workflow_id)

    @mcp.tool()
    async def estimate_workflow_cost(
        definition_name: str,
        input_sample: dict[str, Any],
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Estimate the cost of running a workflow before execution.

        Provides a cost estimate based on the workflow definition and sample input.
        Useful for budget planning and avoiding unexpected costs. Estimates are based
        on average token counts and activity complexity.

        Args:
            definition_name: Name of the workflow definition
            input_sample: Sample input data to base estimate on

        Returns:
            Dictionary containing:
            - estimated_cost_usd: Estimated total cost
            - cost_range_usd: Min/max cost range
            - activities: Per-activity cost estimates
            - assumptions: List of assumptions made in estimation

        Cost Factors:
            - LLM prompts: Based on model pricing and estimated token counts
            - Embeddings: Based on text length and model pricing
            - HTTP requests: Usually free (unless using metered proxies)
            - Database queries: Usually free (unless using serverless DBs)

        Limitations:
            - Estimates assume typical token counts (may vary with actual data)
            - Does not account for retries or fallback models
            - External API costs (if any) are not included

        Example:
            estimate = await estimate_workflow_cost(
                definition_name="research_assistant",
                input_sample={"question": "What is Rust?"}
            )
            print(f"Estimated cost: ${estimate['estimated_cost_usd']:.4f}")
            print(f"Range: ${estimate['cost_range_usd']['min']:.4f} - ${estimate['cost_range_usd']['max']:.4f}")
        """
        # Get the workflow definition
        definition = await client.get_workflow_definition(definition_name)

        # Parse activities and estimate costs
        activities = definition.get("activities", [])
        activity_estimates = []
        total_estimate = 0.0
        min_estimate = 0.0
        max_estimate = 0.0

        for activity in activities:
            activity_name = activity.get("activity_name", "")
            activity_key = activity.get("key", "")

            # Estimate based on activity type
            estimate = 0.0
            min_cost = 0.0
            max_cost = 0.0

            if activity_name == "llm_prompt":
                # Estimate LLM cost based on model
                model = activity.get("parameters", {}).get("model", "")
                max_tokens = activity.get("parameters", {}).get("max_tokens", 1024)

                # Assume ~100 input tokens (conservative)
                input_tokens = 100
                output_tokens = max_tokens

                # Model pricing (per million tokens)
                # These are rough estimates and should be updated with actual pricing
                pricing = {
                    "anthropic/claude-opus-4": {"input": 15.0, "output": 75.0},
                    "anthropic/claude-sonnet-4": {"input": 3.0, "output": 15.0},
                    "anthropic/claude-3-5-haiku": {"input": 0.8, "output": 4.0},
                    "openai/gpt-4": {"input": 10.0, "output": 30.0},
                    "openai/gpt-4-turbo": {"input": 5.0, "output": 15.0},
                    "openai/gpt-3.5-turbo": {"input": 0.5, "output": 1.5},
                }

                # Find matching pricing
                model_pricing = None
                for model_pattern, price in pricing.items():
                    if model_pattern in model:
                        model_pricing = price
                        break

                if model_pricing:
                    estimate = (
                        input_tokens / 1_000_000 * model_pricing["input"]
                        + output_tokens / 1_000_000 * model_pricing["output"]
                    )
                    min_cost = estimate * 0.5  # Lower bound (shorter response)
                    max_cost = estimate * 2.0  # Upper bound (longer prompt/response)
                else:
                    # Default estimate for unknown models
                    estimate = 0.01
                    min_cost = 0.001
                    max_cost = 0.05

            elif activity_name == "embedding":
                # Estimate embedding cost
                # OpenAI text-embedding-3-small: $0.02 per 1M tokens
                # Assume ~500 tokens per text
                estimate = 0.00001
                min_cost = 0.000005
                max_cost = 0.00005

            elif activity_name in ["http_request", "postgres_query", "postgres_transaction"]:
                # These are typically free
                estimate = 0.0
                min_cost = 0.0
                max_cost = 0.0

            elif activity_name == "email_send":
                # Email costs vary by provider, but usually minimal
                estimate = 0.0001
                min_cost = 0.0
                max_cost = 0.001

            activity_estimates.append(
                {
                    "activity_key": activity_key,
                    "activity_name": activity_name,
                    "estimated_cost_usd": estimate,
                    "cost_range_usd": {"min": min_cost, "max": max_cost},
                }
            )

            total_estimate += estimate
            min_estimate += min_cost
            max_estimate += max_cost

        return {
            "definition_name": definition_name,
            "estimated_cost_usd": total_estimate,
            "cost_range_usd": {"min": min_estimate, "max": max_estimate},
            "activities": activity_estimates,
            "assumptions": [
                "Estimates based on typical token counts",
                "Does not account for retries or fallback models",
                "External API costs not included",
                "Assumes average-length responses",
            ],
            "note": "Actual costs may vary based on input data, response length, and execution path",
        }
