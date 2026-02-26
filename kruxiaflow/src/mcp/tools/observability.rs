/// MCP Observability Tools
///
/// Five read-only tools for monitoring workflow executions and analysing costs:
/// - get_workflow_status: current status + optional activity details
/// - list_workflows: paginated list with status filter
/// - get_activity_output: output + cost for a specific activity
/// - get_workflow_cost: cost breakdown with per-activity and per-provider aggregation
/// - estimate_workflow_cost: pre-execution cost estimate for a definition
use rust_decimal::prelude::*;
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use rust_mcp_sdk::tool_box;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

use super::{error_response, parse_uuid, text_response};

// ============================================================================
// Tool: get_workflow_status
// ============================================================================

#[mcp_tool(
    name = "get_workflow_status",
    description = "Get the current status of a workflow execution.\n\
        \n\
        Retrieves status information for a specific workflow including overall \
        status and timestamps. Optionally include full activity-level details: \
        each activity's status, start and completion times, and retry count.\n\
        \n\
        When to use: After submitting a workflow (via submit_workflow) to monitor \
        its progress. Poll periodically or check after expected completion time.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetWorkflowStatus {
    /// UUID of the workflow execution to check
    pub workflow_id: String,

    /// If true, include status details for every activity in the workflow
    #[serde(default)]
    pub include_activities: bool,
}

// ============================================================================
// Tool: list_workflows
// ============================================================================

#[mcp_tool(
    name = "list_workflows",
    description = "List workflow executions with optional filtering.\n\
        \n\
        Retrieves a paginated list of workflow executions. Filter by status to \
        find workflows in specific states — for example all running workflows, \
        or all that have failed.\n\
        \n\
        When to use: To get an overview of workflow activity, find specific \
        workflows, or monitor the health of a deployment.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListWorkflows {
    /// Filter by status: created, running, completed, failed, paused
    pub status: Option<String>,

    /// Maximum number of workflows to return (default: 20)
    #[serde(default = "default_limit")]
    pub limit: Option<i64>,

    /// Number of workflows to skip for pagination (default: 0)
    #[serde(default)]
    pub offset: Option<i64>,
}

fn default_limit() -> Option<i64> {
    Some(20)
}

// ============================================================================
// Tool: get_activity_output
// ============================================================================

#[mcp_tool(
    name = "get_activity_output",
    description = "Get the output of a specific activity in a workflow.\n\
        \n\
        Retrieves the results produced by a completed activity, including its \
        output payload, cost, and any files it produced. Output format varies \
        by activity type: http_request returns response + status_code; \
        llm_prompt returns result + cost + token usage; postgres_query returns \
        rows + row_count.\n\
        \n\
        When to use: After a workflow completes or an activity finishes, to \
        retrieve intermediate or final results. The activity must be in \
        'completed' status.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetActivityOutput {
    /// UUID of the workflow execution
    pub workflow_id: String,

    /// Key of the activity (as defined in the workflow YAML)
    pub activity_key: String,
}

// ============================================================================
// Tool: get_workflow_cost
// ============================================================================

#[mcp_tool(
    name = "get_workflow_cost",
    description = "Get the cost breakdown for a workflow execution.\n\
        \n\
        Retrieves detailed cost information: total cost, per-activity breakdown \
        with provider and model details, token usage where available, and budget \
        utilisation if a budget limit was set. Costs are tracked for LLM calls, \
        embeddings, and other metered services.\n\
        \n\
        When to use: After or during workflow execution to understand costs. \
        Useful for budget monitoring and post-execution cost analysis.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetWorkflowCost {
    /// UUID of the workflow execution
    pub workflow_id: String,
}

// ============================================================================
// Tool: estimate_workflow_cost
// ============================================================================

#[mcp_tool(
    name = "estimate_workflow_cost",
    description = "Estimate the cost of running a workflow before execution.\n\
        \n\
        Provides a cost estimate based on a deployed workflow definition and a \
        sample input. Estimates use provider-specific token heuristics and current \
        model pricing from the database. Returns a min/max range per activity and \
        an overall range, plus the assumptions made.\n\
        \n\
        When to use: Before submitting a workflow — especially one with llm_prompt \
        activities — to understand expected costs and plan budgets. Only \
        llm_prompt activities contribute to the estimate; all others are zero-cost.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct EstimateWorkflowCost {
    /// Name of the deployed workflow definition
    pub definition_name: String,

    /// Sample input payload — keys should match what the workflow's
    /// {{INPUT.key}} expressions reference. Used to approximate prompt lengths.
    pub input_sample: serde_json::Value,
}

// ============================================================================
// Enum + routing glue
// ============================================================================

tool_box!(
    ObservabilityTools,
    [
        GetWorkflowStatus,
        ListWorkflows,
        GetActivityOutput,
        GetWorkflowCost,
        EstimateWorkflowCost
    ]
);

// ============================================================================
// Async runners
// ============================================================================

/// Get workflow status, optionally with full activity details.
pub async fn run_get_workflow_status(
    pool: &PgPool,
    params: &GetWorkflowStatus,
) -> Result<CallToolResult, CallToolError> {
    let workflow_id = parse_uuid(&params.workflow_id)?;
    let svc = kruxiaflow_core::workflow::WorkflowQueryService::new(pool.clone());

    match svc.get_workflow(workflow_id).await {
        Ok(record) => {
            let is_terminal = record.status == "completed" || record.status == "failed";
            let completed_at: serde_json::Value = if is_terminal {
                serde_json::Value::String(record.updated_at.to_rfc3339())
            } else {
                serde_json::Value::Null
            };

            let mut response = serde_json::json!({
                "workflow_id": record.id.to_string(),
                "definition_name": record.definition_name,
                "status": record.status,
                "started_at": record.created_at.to_rfc3339(),
                "completed_at": completed_at,
            });

            if params.include_activities {
                response["activities"] = extract_activities_array(&record.activities);
            }

            text_response(&response)
        }
        Err(kruxiaflow_core::workflow::WorkflowQueryError::WorkflowNotFound(_)) => {
            error_response(&serde_json::json!({
                "error": format!("Workflow '{}' not found", params.workflow_id),
                "workflow_id": params.workflow_id,
            }))
        }
        Err(e) => {
            tracing::error!("get_workflow_status error: {e:?}");
            Err(CallToolError::from_message(format!(
                "Database error looking up workflow '{}': {e}",
                params.workflow_id
            )))
        }
    }
}

/// List workflows with optional status filter and pagination.
pub async fn run_list_workflows(
    pool: &PgPool,
    params: &ListWorkflows,
) -> Result<CallToolResult, CallToolError> {
    let limit = params.limit.unwrap_or(20);
    let offset = params.offset.unwrap_or(0);

    let filters = kruxiaflow_core::workflow::WorkflowFilters {
        status: params.status.clone(),
        ..Default::default()
    };

    let svc = kruxiaflow_core::workflow::WorkflowQueryService::new(pool.clone());

    match svc.list_workflows(filters, limit, offset).await {
        Ok((records, total)) => {
            let workflows: Vec<serde_json::Value> = records
                .iter()
                .map(|r| {
                    let status_str = r.status.to_string();
                    let is_terminal = status_str == "completed" || status_str == "failed";
                    serde_json::json!({
                        "workflow_id": r.id.to_string(),
                        "definition_name": r.definition_name,
                        "status": status_str,
                        "started_at": r.created_at.to_rfc3339(),
                        "completed_at": if is_terminal {
                            serde_json::Value::String(r.updated_at.to_rfc3339())
                        } else {
                            serde_json::Value::Null
                        },
                    })
                })
                .collect();

            text_response(&serde_json::json!({
                "workflows": workflows,
                "total": total,
                "limit": limit,
                "offset": offset,
            }))
        }
        Err(e) => {
            tracing::error!("list_workflows error: {e:?}");
            Err(CallToolError::from_message(format!(
                "Database error listing workflows: {e}"
            )))
        }
    }
}

/// Get the output of a specific activity.
pub async fn run_get_activity_output(
    pool: &PgPool,
    params: &GetActivityOutput,
) -> Result<CallToolResult, CallToolError> {
    let workflow_id = parse_uuid(&params.workflow_id)?;

    // PostgresStorage only holds a pool — construct locally, same as the API server does.
    let storage = kruxiaflow_core::PostgresStorage::new(pool.clone());
    let svc = kruxiaflow_core::workflow::OutputQueryService::new(pool.clone());

    match svc
        .get_activity_output(workflow_id, &params.activity_key, &storage)
        .await
    {
        Ok(result) => {
            let files: Vec<serde_json::Value> = result
                .files
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "filename": f.filename,
                        "size": f.size,
                        "content_type": f.content_type,
                        "download_url": f.download_url,
                    })
                })
                .collect();

            text_response(&serde_json::json!({
                "workflow_id": result.workflow_id.to_string(),
                "activity_key": result.activity_key,
                "status": result.status,
                "output": result.output,
                "cost_usd": result.cost_usd.to_f64().unwrap_or(0.0),
                "completed_at": result.completed_at.map(|t| t.to_rfc3339()),
                "files": files,
            }))
        }
        Err(kruxiaflow_core::workflow::OutputQueryError::WorkflowNotFound(_)) => {
            error_response(&serde_json::json!({
                "error": format!("Workflow '{}' not found", params.workflow_id),
                "workflow_id": params.workflow_id,
            }))
        }
        Err(kruxiaflow_core::workflow::OutputQueryError::ActivityNotFound(key)) => {
            error_response(&serde_json::json!({
                "error": format!(
                    "Activity '{}' not found in workflow '{}'",
                    key, params.workflow_id
                ),
                "workflow_id": params.workflow_id,
                "activity_key": params.activity_key,
            }))
        }
        Err(kruxiaflow_core::workflow::OutputQueryError::ActivityNotCompleted(key)) => {
            error_response(&serde_json::json!({
                "error": format!(
                    "Activity '{}' has not completed yet — output is not available",
                    key
                ),
                "workflow_id": params.workflow_id,
                "activity_key": params.activity_key,
            }))
        }
        Err(e) => {
            tracing::error!("get_activity_output error: {e:?}");
            Err(CallToolError::from_message(format!(
                "Error retrieving output for activity '{}' in workflow '{}': {e}",
                params.activity_key, params.workflow_id
            )))
        }
    }
}

/// Get cost breakdown for a workflow.
pub async fn run_get_workflow_cost(
    pool: &PgPool,
    params: &GetWorkflowCost,
) -> Result<CallToolResult, CallToolError> {
    let workflow_id = parse_uuid(&params.workflow_id)?;

    // 1. Check workflow exists and grab activity name map from the JSONB
    let svc = kruxiaflow_core::workflow::WorkflowQueryService::new(pool.clone());
    let record = match svc.get_workflow(workflow_id).await {
        Ok(r) => r,
        Err(kruxiaflow_core::workflow::WorkflowQueryError::WorkflowNotFound(_)) => {
            return error_response(&serde_json::json!({
                "error": format!("Workflow '{}' not found", params.workflow_id),
                "workflow_id": params.workflow_id,
            }));
        }
        Err(e) => {
            tracing::error!("get_workflow_cost query error: {e:?}");
            return Err(CallToolError::from_message(format!(
                "Database error looking up workflow '{}': {e}",
                params.workflow_id
            )));
        }
    };
    let name_map = extract_activity_name_map(&record.activities);

    // 2. Total cost via stored proc (same one CostTracker uses internally)
    // TODO(#9): The per-activity query below uses raw SQL. Migrate to a stored proc
    // with compile-time validation (sqlx::query!) per project conventions.
    let total_cost: Decimal = sqlx::query("SELECT get_workflow_cost($1)")
        .bind(workflow_id)
        .fetch_one(pool)
        .await
        .map(|row| row.get::<Option<Decimal>, _>(0).unwrap_or(Decimal::ZERO))
        .unwrap_or(Decimal::ZERO);

    // 3. Per-activity cost breakdown from activity_costs
    let rows = sqlx::query(
        "SELECT activity_key, provider, model, \
         SUM(cost_usd) as cost_usd, \
         SUM(prompt_tokens) as prompt_tokens, \
         SUM(output_tokens) as output_tokens, \
         SUM(total_tokens) as total_tokens, \
         MAX(workflow_budget_limit_usd) as budget_limit \
         FROM activity_costs \
         WHERE workflow_id = $1 \
         GROUP BY activity_key, provider, model",
    )
    .bind(workflow_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        CallToolError::from_message(format!("Database error querying activity costs: {e}"))
    })?;

    let mut activities: Vec<serde_json::Value> = Vec::new();
    let mut providers: HashMap<String, f64> = HashMap::new();
    let mut budget_limit: Option<Decimal> = None;

    for row in &rows {
        let key: String = row.get(0);
        let provider: String = row.get(1);
        let model: String = row.get(2);
        let cost: Decimal = row.get::<Option<Decimal>, _>(3).unwrap_or(Decimal::ZERO);
        let prompt_tokens: Option<i64> = row.get(4);
        let output_tokens: Option<i64> = row.get(5);
        let total_tokens: Option<i64> = row.get(6);
        let row_budget: Option<Decimal> = row.get(7);

        if budget_limit.is_none() && let Some(b) = row_budget {
            budget_limit = Some(b);
        }

        let cost_f64 = cost.to_f64().unwrap_or(0.0);
        *providers.entry(provider.clone()).or_default() += cost_f64;

        let tokens = if prompt_tokens.is_some() || output_tokens.is_some() {
            Some(serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "output_tokens": output_tokens,
                "total_tokens": total_tokens,
            }))
        } else {
            None
        };

        activities.push(serde_json::json!({
            "activity_key": key,
            "activity_name": name_map.get(&key),
            "cost_usd": cost_f64,
            "provider": provider,
            "model": model,
            "tokens": tokens,
        }));
    }

    let budget_f64 = budget_limit.map(|b| b.to_f64().unwrap_or(0.0));
    let budget_used_percent = budget_f64.and_then(|limit| {
        if limit > 0.0 {
            Some((total_cost.to_f64().unwrap_or(0.0) / limit) * 100.0)
        } else {
            None
        }
    });

    let providers_json: serde_json::Value = providers
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();

    text_response(&serde_json::json!({
        "workflow_id": params.workflow_id,
        "total_cost_usd": total_cost.to_f64().unwrap_or(0.0),
        "budget_limit_usd": budget_f64,
        "budget_used_percent": budget_used_percent,
        "activities": activities,
        "providers": providers_json,
    }))
}

/// Estimate cost for a workflow definition before execution.
pub async fn run_estimate_workflow_cost(
    pool: &PgPool,
    params: &EstimateWorkflowCost,
) -> Result<CallToolResult, CallToolError> {
    // 1. Look up the deployed definition
    let repo = kruxiaflow_core::WorkflowDefinitionRepository::new(pool.clone());
    let stored = repo
        .get_latest(&params.definition_name)
        .await
        .map_err(|e| {
            CallToolError::from_message(format!(
                "Error looking up definition '{}': {e}",
                params.definition_name
            ))
        })?;

    let stored = match stored {
        Some(s) => s,
        None => {
            return error_response(&serde_json::json!({
                "error": format!(
                    "Workflow definition '{}' not found. Deploy it first.",
                    params.definition_name
                ),
                "definition_name": params.definition_name,
            }));
        }
    };

    // 2. Walk activities; estimate cost only for llm_prompt
    let calculator = kruxiaflow_core::CostCalculator::new(pool.clone());
    let mut activity_estimates: Vec<serde_json::Value> = Vec::new();
    let mut total_min = Decimal::ZERO;
    let mut total_max = Decimal::ZERO;
    let mut warnings: Vec<String> = Vec::new();

    for activity in &stored.activities {
        let activity_name = activity.activity_name.as_deref().unwrap_or("unknown");

        if activity_name == "llm_prompt" {
            let p = activity.parameters.as_ref();

            let explicit_provider = p.and_then(|m| m.get("provider").and_then(|v| v.as_str()));
            let explicit_model = p.and_then(|m| m.get("model").and_then(|v| v.as_str()));

            let provider = explicit_provider.unwrap_or("anthropic");
            let model = explicit_model.unwrap_or("claude-sonnet-4-5-20250929");

            if explicit_provider.is_none() || explicit_model.is_none() {
                warnings.push(format!(
                    "Activity '{}' missing explicit {}: defaulting to {}/{}. \
                     Cost estimate may be inaccurate if a different model is used at runtime.",
                    activity.key,
                    if explicit_provider.is_none() && explicit_model.is_none() {
                        "provider and model"
                    } else if explicit_provider.is_none() {
                        "provider"
                    } else {
                        "model"
                    },
                    provider,
                    model,
                ));
            }
            let max_tokens = p
                .and_then(|m| m.get("max_tokens").and_then(|v| v.as_u64()))
                .unwrap_or(1024) as u32;
            let prompt_template = p
                .and_then(|m| m.get("prompt").and_then(|v| v.as_str()))
                .unwrap_or("");

            let rendered = substitute_input_template(prompt_template, &params.input_sample);

            // max cost: full max_tokens output
            let max_cost = calculator
                .estimate_llm_cost(provider, model, &rendered, max_tokens)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("Could not estimate cost for {}/{}: {e}", provider, model);
                    Decimal::ZERO
                });

            // min cost: 25% of max_tokens output
            let min_tokens = std::cmp::max(max_tokens / 4, 1);
            let min_cost = calculator
                .estimate_llm_cost(provider, model, &rendered, min_tokens)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        "Could not estimate min cost for {}/{}: {e}",
                        provider,
                        model
                    );
                    Decimal::ZERO
                });

            let estimated = (min_cost + max_cost) / Decimal::from(2);
            total_min += min_cost;
            total_max += max_cost;

            activity_estimates.push(serde_json::json!({
                "activity_key": activity.key,
                "activity_name": activity_name,
                "estimated_cost_usd": estimated.to_f64().unwrap_or(0.0),
                "cost_range_usd": {
                    "min": min_cost.to_f64().unwrap_or(0.0),
                    "max": max_cost.to_f64().unwrap_or(0.0),
                },
            }));
        } else {
            activity_estimates.push(serde_json::json!({
                "activity_key": activity.key,
                "activity_name": activity_name,
                "estimated_cost_usd": 0.0,
                "cost_range_usd": null,
            }));
        }
    }

    let total_estimated = (total_min + total_max) / Decimal::from(2);

    text_response(&serde_json::json!({
        "definition_name": params.definition_name,
        "estimated_cost_usd": total_estimated.to_f64().unwrap_or(0.0),
        "cost_range_usd": {
            "min": total_min.to_f64().unwrap_or(0.0),
            "max": total_max.to_f64().unwrap_or(0.0),
        },
        "activities": activity_estimates,
        "warnings": warnings,
        "assumptions": [
            "Token estimates use provider-specific heuristics (Anthropic: 3.5 chars/token, OpenAI: 4.0 chars/token)",
            "Prompt length based on template text with {{INPUT.key}} values substituted from input_sample",
            "Min cost assumes 25% of max_tokens used; max cost assumes 100% of max_tokens",
            "Non-LLM activities have zero estimated cost",
            "Does not account for retries or fallback models",
        ],
    }))
}

// ============================================================================
// Helpers
// ============================================================================

/// Extract activities from the workflow's activities JSONB as a flat JSON array.
///
/// The JSONB may be an object keyed by activity_key (orchestrator format) or
/// already an array. When it's an object, "key" is injected into each entry.
fn extract_activities_array(activities_json: &serde_json::Value) -> serde_json::Value {
    if let Some(arr) = activities_json.as_array() {
        serde_json::Value::Array(arr.clone())
    } else if let Some(obj) = activities_json.as_object() {
        let arr: Vec<serde_json::Value> = obj
            .iter()
            .map(|(key, val)| {
                let mut entry = val.clone();
                if let Some(map) = entry.as_object_mut() {
                    map.insert("key".to_string(), serde_json::Value::String(key.clone()));
                }
                entry
            })
            .collect();
        serde_json::Value::Array(arr)
    } else {
        serde_json::Value::Array(vec![])
    }
}

/// Build a HashMap<activity_key, activity_name> from the activities JSONB.
/// Used by get_workflow_cost to annotate cost rows with human-readable names.
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

/// Replace every `{{INPUT.<key>}}` in `template` with the corresponding value
/// from `input_sample`. Non-matching placeholders are left untouched.
fn substitute_input_template(template: &str, input_sample: &serde_json::Value) -> String {
    let mut result = template.to_string();

    if let Some(obj) = input_sample.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{{{INPUT.{}}}}}", key);
            let replacement = match value {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            result = result.replace(&placeholder, &replacement);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // extract_activities_array tests
    // =========================================================================

    #[test]
    fn test_extract_activities_array_from_object() {
        let input = serde_json::json!({
            "fetch": {"status": "completed", "activity_name": "http_request"},
            "process": {"status": "running", "activity_name": "echo"},
        });
        let result = extract_activities_array(&input);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // Object format should inject "key" field
        for item in arr {
            assert!(item.get("key").is_some());
        }
    }

    #[test]
    fn test_extract_activities_array_from_array() {
        let input = serde_json::json!([
            {"key": "fetch", "status": "completed"},
            {"key": "process", "status": "running"},
        ]);
        let result = extract_activities_array(&input);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_extract_activities_array_from_null() {
        let result = extract_activities_array(&serde_json::Value::Null);
        assert_eq!(result, serde_json::json!([]));
    }

    // =========================================================================
    // extract_activity_name_map tests
    // =========================================================================

    #[test]
    fn test_extract_activity_name_map_object_format() {
        let input = serde_json::json!({
            "fetch": {"activity_name": "http_request"},
            "process": {"activity_name": "echo"},
        });
        let map = extract_activity_name_map(&input);
        assert_eq!(map.get("fetch").unwrap(), "http_request");
        assert_eq!(map.get("process").unwrap(), "echo");
    }

    #[test]
    fn test_extract_activity_name_map_array_format() {
        let input = serde_json::json!([
            {"key": "fetch", "activity_name": "http_request"},
            {"key": "process", "activity_name": "echo"},
        ]);
        let map = extract_activity_name_map(&input);
        assert_eq!(map.get("fetch").unwrap(), "http_request");
        assert_eq!(map.get("process").unwrap(), "echo");
    }

    #[test]
    fn test_extract_activity_name_map_empty() {
        let map = extract_activity_name_map(&serde_json::Value::Null);
        assert!(map.is_empty());
    }

    // =========================================================================
    // substitute_input_template tests
    // =========================================================================

    #[test]
    fn test_substitute_input_template_basic() {
        let template = "Hello {{INPUT.name}}, your ID is {{INPUT.id}}";
        let input = serde_json::json!({"name": "Alice", "id": "42"});
        let result = substitute_input_template(template, &input);
        assert_eq!(result, "Hello Alice, your ID is 42");
    }

    #[test]
    fn test_substitute_input_template_non_string_values() {
        let template = "Count: {{INPUT.count}}, Active: {{INPUT.active}}";
        let input = serde_json::json!({"count": 5, "active": true});
        let result = substitute_input_template(template, &input);
        assert_eq!(result, "Count: 5, Active: true");
    }

    #[test]
    fn test_substitute_input_template_unmatched_placeholder() {
        let template = "{{INPUT.exists}} and {{INPUT.missing}}";
        let input = serde_json::json!({"exists": "found"});
        let result = substitute_input_template(template, &input);
        assert_eq!(result, "found and {{INPUT.missing}}");
    }

    #[test]
    fn test_substitute_input_template_no_placeholders() {
        let template = "No placeholders here";
        let input = serde_json::json!({"key": "value"});
        let result = substitute_input_template(template, &input);
        assert_eq!(result, "No placeholders here");
    }

    #[test]
    fn test_substitute_input_template_null_input() {
        let template = "{{INPUT.key}}";
        let result = substitute_input_template(template, &serde_json::Value::Null);
        assert_eq!(result, "{{INPUT.key}}");
    }
}
