/// MCP Visualization Tools
///
/// Two read-only tools that generate Mermaid diagrams:
/// - render_workflow_diagram: dependency graph with optional execution-status colours
/// - render_cost_diagram: cost tree rooted at total cost

use rust_decimal::prelude::*;
use rust_mcp_sdk::macros::{mcp_tool, JsonSchema};
use rust_mcp_sdk::schema::{schema_utils::CallToolError, CallToolResult, TextContent};
use rust_mcp_sdk::tool_box;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

// ============================================================================
// Tool: render_workflow_diagram
// ============================================================================

#[mcp_tool(
    name = "render_workflow_diagram",
    description = "Generate a Mermaid flowchart of a workflow's activity dependency graph.\n\
        \n\
        When a workflow_id is provided, each activity node is colour-coded by its \
        current execution status (green = completed, amber = running, red = failed, \
        orange = waiting, blue = pending, grey = skipped). When only a definition_name \
        is given, the diagram shows the static dependency structure without status colours.\n\
        \n\
        At least one of definition_name or workflow_id must be provided.\n\
        \n\
        When to use: To visualise workflow structure before or during execution. \
        Paste the returned 'diagram' string into any Mermaid renderer (GitHub, \
        mdBook, online playground) to see the graph.",
    read_only_hint = true,
    idempotent_hint = true,
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RenderWorkflowDiagram {
    /// Name of a deployed workflow definition (required when workflow_id is omitted)
    pub definition_name: Option<String>,

    /// UUID of a workflow execution — when provided, activities are colour-coded by status
    pub workflow_id: Option<String>,
}

// ============================================================================
// Tool: render_cost_diagram
// ============================================================================

#[mcp_tool(
    name = "render_cost_diagram",
    description = "Generate a Mermaid flowchart showing cost breakdown for a workflow.\n\
        \n\
        Produces a tree diagram with total cost at the root and one node per \
        activity that has recorded costs. Each node displays the activity name, \
        key, and cost in USD. Activities are ordered highest-cost first.\n\
        \n\
        When to use: After a workflow has started executing and costs have been \
        recorded. Useful for understanding which activities contribute most to \
        the total cost.",
    read_only_hint = true,
    idempotent_hint = true,
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RenderCostDiagram {
    /// UUID of the workflow execution whose costs to visualise
    pub workflow_id: String,
}

// ============================================================================
// Enum + routing glue
// ============================================================================

tool_box!(
    VisualizationTools,
    [RenderWorkflowDiagram, RenderCostDiagram]
);

// ============================================================================
// Async runners
// ============================================================================

/// Generate a Mermaid flowchart for a workflow definition or execution.
pub async fn run_render_workflow_diagram(
    pool: &PgPool,
    params: &RenderWorkflowDiagram,
) -> Result<CallToolResult, CallToolError> {
    if params.definition_name.is_none() && params.workflow_id.is_none() {
        return text_response(&serde_json::json!({
            "error": "At least one of 'definition_name' or 'workflow_id' must be provided",
        }));
    }

    // If workflow_id given, fetch the workflow record for status map + definition_name
    let (definition_name, status_map) = if let Some(ref wf_id_str) = params.workflow_id {
        let workflow_id = parse_uuid(wf_id_str)?;
        let svc = kruxiaflow_core::workflow::WorkflowQueryService::new(pool.clone());
        match svc.get_workflow(workflow_id).await {
            Ok(record) => {
                let statuses = extract_status_map(&record.activities);
                (record.definition_name.clone(), Some(statuses))
            }
            Err(kruxiaflow_core::workflow::WorkflowQueryError::WorkflowNotFound(_)) => {
                return text_response(&serde_json::json!({
                    "error": format!("Workflow '{}' not found", wf_id_str),
                    "workflow_id": wf_id_str,
                }));
            }
            Err(e) => {
                tracing::error!("render_workflow_diagram error: {e:?}");
                return Err(CallToolError::from_message(format!(
                    "Database error looking up workflow '{}': {e}",
                    wf_id_str
                )));
            }
        }
    } else {
        (params.definition_name.clone().unwrap(), None)
    };

    // Fetch the deployed definition for dependency structure
    let repo = kruxiaflow_core::WorkflowDefinitionRepository::new(pool.clone());
    let stored = repo
        .get_latest(&definition_name)
        .await
        .map_err(|e| {
            CallToolError::from_message(format!(
                "Error looking up definition '{}': {e}",
                definition_name
            ))
        })?;

    let stored = match stored {
        Some(s) => s,
        None => {
            return text_response(&serde_json::json!({
                "error": format!(
                    "Workflow definition '{}' not found. Deploy it first.",
                    definition_name
                ),
                "definition_name": definition_name,
            }));
        }
    };

    let activity_count = stored.activities.len();
    let has_status = status_map.is_some();
    let diagram = build_workflow_mermaid(&stored, &status_map);

    text_response(&serde_json::json!({
        "diagram": diagram,
        "definition_name": definition_name,
        "workflow_id": params.workflow_id,
        "activity_count": activity_count,
        "has_status_colours": has_status,
    }))
}

/// Generate a Mermaid cost-tree diagram for a workflow execution.
pub async fn run_render_cost_diagram(
    pool: &PgPool,
    params: &RenderCostDiagram,
) -> Result<CallToolResult, CallToolError> {
    let workflow_id = parse_uuid(&params.workflow_id)?;

    // Check workflow exists and grab activity name map
    let svc = kruxiaflow_core::workflow::WorkflowQueryService::new(pool.clone());
    let record = match svc.get_workflow(workflow_id).await {
        Ok(r) => r,
        Err(kruxiaflow_core::workflow::WorkflowQueryError::WorkflowNotFound(_)) => {
            return text_response(&serde_json::json!({
                "error": format!("Workflow '{}' not found", params.workflow_id),
                "workflow_id": params.workflow_id,
            }));
        }
        Err(e) => {
            tracing::error!("render_cost_diagram error: {e:?}");
            return Err(CallToolError::from_message(format!(
                "Database error looking up workflow '{}': {e}",
                params.workflow_id
            )));
        }
    };
    let name_map = extract_activity_name_map(&record.activities);

    // Total cost via stored proc (same one CostTracker uses)
    let total_cost: Decimal = sqlx::query("SELECT get_workflow_cost($1)")
        .bind(workflow_id)
        .fetch_one(pool)
        .await
        .map(|row| row.get::<Option<Decimal>, _>(0).unwrap_or(Decimal::ZERO))
        .unwrap_or(Decimal::ZERO);

    // Per-activity costs — aggregate across provider/model for the diagram
    let rows = sqlx::query(
        "SELECT activity_key, SUM(cost_usd) as cost_usd \
         FROM activity_costs \
         WHERE workflow_id = $1 \
         GROUP BY activity_key",
    )
    .bind(workflow_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        CallToolError::from_message(format!("Database error querying activity costs: {e}"))
    })?;

    let mut activities: Vec<(String, f64)> = Vec::new();
    for row in &rows {
        let key: String = row.get(0);
        let cost: Decimal = row.get::<Option<Decimal>, _>(1).unwrap_or(Decimal::ZERO);
        activities.push((key, cost.to_f64().unwrap_or(0.0)));
    }

    // Sort highest-cost first for visual clarity
    activities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let activity_count = activities.len();
    let diagram = build_cost_mermaid(total_cost.to_f64().unwrap_or(0.0), &activities, &name_map);

    text_response(&serde_json::json!({
        "diagram": diagram,
        "workflow_id": params.workflow_id,
        "total_cost_usd": total_cost.to_f64().unwrap_or(0.0),
        "activity_count": activity_count,
    }))
}

// ============================================================================
// Mermaid builders
// ============================================================================

/// Build a Mermaid `flowchart TD` for the workflow dependency graph.
///
/// Node IDs are sanitised (non-alphanumeric chars → underscore) so that keys
/// containing dots, hyphens etc. don't break the Mermaid parser.
fn build_workflow_mermaid(
    stored: &kruxiaflow_core::StoredWorkflowDefinition,
    status_map: &Option<HashMap<String, String>>,
) -> String {
    let mut lines = vec!["flowchart TD".to_string()];

    // --- Nodes ---
    for activity in &stored.activities {
        let id = node_id(&activity.key);
        let name = activity.activity_name.as_deref().unwrap_or(&activity.key);

        let label = if let Some(statuses) = status_map {
            let status = statuses
                .get(&activity.key)
                .map(|s| s.as_str())
                .unwrap_or("pending");
            format!(
                "    {}[\"{}<br/>{}<br/>Status: {}\"]",
                id, name, activity.key, status
            )
        } else {
            format!("    {}[\"{}<br/>{}\"]", id, name, activity.key)
        };
        lines.push(label);
    }

    // --- Edges (A depends_on B  →  B --> A) ---
    for activity in &stored.activities {
        if let Some(deps) = &activity.depends_on {
            for dep in deps {
                lines.push(format!(
                    "    {} --> {}",
                    node_id(&dep.activity_key),
                    node_id(&activity.key)
                ));
            }
        }
    }

    // --- Style directives (only when status data is available) ---
    if let Some(statuses) = status_map {
        for activity in &stored.activities {
            let status = statuses
                .get(&activity.key)
                .map(|s| s.as_str())
                .unwrap_or("pending");
            let (fill, color) = status_style(status);
            lines.push(format!(
                "    style {} fill:{},color:{}",
                node_id(&activity.key),
                fill,
                color
            ));
        }
    }

    lines.join("\n")
}

/// Build a Mermaid `flowchart TD` cost tree: root → per-activity nodes.
fn build_cost_mermaid(
    total_cost: f64,
    activities: &[(String, f64)],
    name_map: &HashMap<String, String>,
) -> String {
    let mut lines = vec!["flowchart TD".to_string()];

    // Root node
    lines.push(format!("    total[\"Total Cost<br/>${:.4}\"]", total_cost));
    lines.push("    style total fill:#6f42c1,color:#fff".to_string());

    // Per-activity nodes, edges, and styles
    for (key, cost) in activities {
        let id = node_id(key);
        let name = name_map.get(key).map(|s| s.as_str()).unwrap_or(key);
        lines.push(format!(
            "    {}[\"{}<br/>{}<br/>${:.4}\"]",
            id, name, key, cost
        ));
        if *cost > 0.0 {
            lines.push(format!("    style {} fill:#17a2b8,color:#fff", id));
        }
        lines.push(format!("    total --> {}", id));
    }

    lines.join("\n")
}

// ============================================================================
// Helpers
// ============================================================================

/// Wrap a JSON value as a pretty-printed text response.
fn text_response(value: &serde_json::Value) -> Result<CallToolResult, CallToolError> {
    Ok(CallToolResult::text_content(vec![TextContent::from(
        serde_json::to_string_pretty(value)
            .map_err(|e| CallToolError::from_message(e.to_string()))?,
    )]))
}

/// Parse a string as UUID, returning a tool error if invalid.
fn parse_uuid(s: &str) -> Result<uuid::Uuid, CallToolError> {
    uuid::Uuid::parse_str(s).map_err(|_| {
        CallToolError::from_message(format!(
            "Invalid workflow_id '{}': must be a valid UUID",
            s
        ))
    })
}

/// Sanitise a key for use as a Mermaid node ID (alphanumeric + underscore only).
fn node_id(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

/// Extract a `HashMap<activity_key, status>` from the activities JSONB.
/// Handles both object-keyed and array formats (same dual-format as observability).
fn extract_status_map(activities_json: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(arr) = activities_json.as_array() {
        for item in arr {
            if let (Some(key), Some(status)) = (
                item.get("key").and_then(|v| v.as_str()),
                item.get("status").and_then(|v| v.as_str()),
            ) {
                map.insert(key.to_string(), status.to_string());
            }
        }
    } else if let Some(obj) = activities_json.as_object() {
        for (key, val) in obj {
            if let Some(status) = val.get("status").and_then(|v| v.as_str()) {
                map.insert(key.clone(), status.to_string());
            }
        }
    }
    map
}

/// Extract a `HashMap<activity_key, activity_name>` from the activities JSONB.
fn extract_activity_name_map(activities_json: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(arr) = activities_json.as_array() {
        for item in arr {
            if let (Some(key), Some(name)) = (
                item.get("key").and_then(|v| v.as_str()),
                item.get("activity_name").and_then(|v| v.as_str()),
            ) {
                map.insert(key.to_string(), name.to_string());
            }
        }
    } else if let Some(obj) = activities_json.as_object() {
        for (key, val) in obj {
            if let Some(name) = val.get("activity_name").and_then(|v| v.as_str()) {
                map.insert(key.clone(), name.to_string());
            }
        }
    }
    map
}

/// Return (fill colour, text colour) for a given activity status string.
fn status_style(status: &str) -> (&'static str, &'static str) {
    match status.to_lowercase().as_str() {
        "completed" => ("#28a745", "#fff"),
        "running" => ("#ffc107", "#000"),
        "failed" => ("#dc3545", "#fff"),
        "waiting" => ("#fd7e14", "#fff"),
        "pending" => ("#17a2b8", "#fff"),
        "skipped" => ("#adb5bd", "#000"),
        "not_scheduled" | "notscheduled" => ("#6c757d", "#fff"),
        _ => ("#dee2e6", "#000"),
    }
}
