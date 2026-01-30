"""Mermaid diagram generator for workflow visualization."""

from typing import Any


def generate_workflow_diagram(
    workflow_def: dict[str, Any],
    execution_status: dict[str, Any] | None = None,
) -> str:
    """Generate a Mermaid flowchart from a workflow definition.

    Creates a visual representation of the workflow showing activities
    and their dependencies. Optionally colors activities based on their
    execution status.

    Args:
        workflow_def: Workflow definition dictionary with activities
        execution_status: Optional execution status with activity states

    Returns:
        Mermaid flowchart syntax as a string

    Example Output:
        ```mermaid
        flowchart TB
            start([Start])
            fetch_data[fetch_data<br/>http_request]
            process[process<br/>llm_prompt]
            complete([Complete])

            start --> fetch_data
            fetch_data --> process
            process --> complete

            style fetch_data fill:#90EE90
            style process fill:#FFD700
        ```
    """
    activities = workflow_def.get("activities", [])
    workflow_name = workflow_def.get("name", "workflow")

    # Build activity map
    activity_map: dict[str, dict[str, Any]] = {}
    for activity in activities:
        key = activity.get("key")
        if key:
            activity_map[key] = activity

    # Start building Mermaid diagram
    lines = ["flowchart TB"]

    # Add workflow name as comment
    lines.append(f"    %% Workflow: {workflow_name}")
    lines.append("")

    # Add start node
    lines.append("    start([Start])")
    lines.append("")

    # Add activity nodes
    for key, activity in activity_map.items():
        activity_name = activity.get("activity_name", "unknown")

        # Format activity label (use 1: 2: instead of 1. 2. to avoid mermaid issues)
        # Use <br/> for line breaks
        label = f"{key}<br/>{activity_name}"

        # Use rectangles for regular activities
        lines.append(f"    {key}[{label}]")

    lines.append("")

    # Add end node
    lines.append("    complete([Complete])")
    lines.append("")

    # Add edges based on dependencies
    # First, find activities with no dependencies (connect from start)
    activities_with_deps = set()
    for key, activity in activity_map.items():
        depends_on = activity.get("depends_on", [])
        if depends_on:
            activities_with_deps.add(key)
            # Add edges from dependencies
            for dep in depends_on:
                if dep in activity_map:
                    lines.append(f"    {dep} --> {key}")
        else:
            # No dependencies, connect from start
            lines.append(f"    start --> {key}")

    lines.append("")

    # Find leaf activities (activities that nothing depends on)
    activities_depended_on = set()
    for activity in activities:
        depends_on = activity.get("depends_on", [])
        for dep in depends_on:
            activities_depended_on.add(dep)

    leaf_activities = [key for key in activity_map if key not in activities_depended_on]

    # Connect leaf activities to end
    for key in leaf_activities:
        lines.append(f"    {key} --> complete")

    # Add styling based on execution status
    if execution_status:
        lines.append("")
        lines.append("    %% Activity status styling")

        activity_statuses = {}
        if "activities" in execution_status:
            for activity_status in execution_status["activities"]:
                key = activity_status.get("key")
                status = activity_status.get("status")
                if key:
                    activity_statuses[key] = status

        # Color nodes based on status
        for key, status in activity_statuses.items():
            if status == "completed":
                lines.append(f"    style {key} fill:#90EE90")  # Light green
            elif status == "running":
                lines.append(f"    style {key} fill:#FFD700")  # Gold
            elif status == "failed":
                lines.append(f"    style {key} fill:#FF6B6B")  # Light red
            elif status == "skipped":
                lines.append(f"    style {key} fill:#D3D3D3")  # Light gray
            elif status == "pending":
                lines.append(f"    style {key} fill:#87CEEB")  # Sky blue

        # Color start/complete nodes
        workflow_status = execution_status.get("status")
        if workflow_status == "completed":
            lines.append("    style complete fill:#90EE90")
        elif workflow_status in ["running", "pending"]:
            lines.append("    style start fill:#90EE90")

    return "\n".join(lines)


def generate_cost_breakdown_diagram(cost_data: dict[str, Any]) -> str:
    """Generate a Mermaid diagram showing cost breakdown by activity.

    Creates a simple bar chart or breakdown showing which activities
    contributed most to the total cost.

    Args:
        cost_data: Cost breakdown data from get_workflow_cost

    Returns:
        Mermaid diagram showing cost distribution

    Example Output:
        ```mermaid
        graph LR
            Total[Total: $0.045]
            Total --> A1[ask_question: $0.042]
            Total --> A2[store_response: $0.003]
        ```
    """
    total_cost = cost_data.get("total_cost_usd", 0.0)
    activities = cost_data.get("activities", [])

    lines = ["graph LR"]
    lines.append(f'    Total["Total: ${total_cost:.4f}"]')

    for i, activity in enumerate(activities):
        key = activity.get("activity_key", f"activity_{i}")
        cost = activity.get("cost_usd", 0.0)
        provider = activity.get("provider", "")

        if cost > 0:
            label = f"{key}: ${cost:.4f}"
            if provider:
                label += f"<br/>{provider}"

            # Sanitize node ID (replace hyphens with underscores)
            node_id = key.replace("-", "_").replace(".", "_")

            lines.append(f'    Total --> {node_id}["{label}"]')

    return "\n".join(lines)
