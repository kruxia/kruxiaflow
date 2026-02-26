/// MCP Discovery Tools
///
/// Four read-only tools that let AI agents discover what's available in Kruxia Flow:
/// - list_workflow_definitions: what workflows are deployed
/// - get_workflow_definition: full structure of a specific workflow
/// - list_activities: what activity types exist
/// - get_workflow_authoring_guide: how to write workflows
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{CallToolResult, TextContent, schema_utils::CallToolError};

use super::error_response;
use rust_mcp_sdk::tool_box;
use sqlx::PgPool;

// ============================================================================
// Tool: list_workflow_definitions
// ============================================================================

#[mcp_tool(
    name = "list_workflow_definitions",
    description = "List all deployed workflow definitions in Kruxia Flow.\n\
        \n\
        Returns the latest version of each workflow with its name, version, activity count, \
        and deployment time. Use get_workflow_definition to retrieve the full structure of \
        a specific workflow.\n\
        \n\
        When to use: Start here to discover what workflows are available before submitting \
        or monitoring them.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListWorkflowDefinitions {
    /// Filter workflows whose name starts with this prefix (optional)
    pub name: Option<String>,
}

// ============================================================================
// Tool: get_workflow_definition
// ============================================================================

#[mcp_tool(
    name = "get_workflow_definition",
    description = "Get the full structure of a deployed workflow definition.\n\
        \n\
        Returns all activities, their dependencies (depends_on), settings (retry, timeout, \
        budget), and other configuration. If no version is specified, returns the latest \
        deployed version.\n\
        \n\
        When to use: After list_workflow_definitions to inspect a specific workflow before \
        submitting it, or to understand its structure for monitoring.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetWorkflowDefinition {
    /// Workflow name (required)
    pub name: String,

    /// Specific version in YYYYmmdd.HHMMSS.uuuuuu format (optional — returns latest if omitted)
    pub version: Option<String>,
}

// ============================================================================
// Tool: list_activities
// ============================================================================

#[mcp_tool(
    name = "list_activities",
    description = "List all available built-in activity types in Kruxia Flow.\n\
        \n\
        Returns a catalog of every activity type with its parameters, outputs, and settings. \
        Use this to understand what building blocks are available when authoring workflows. \
        All activities support template expressions like {{INPUT.field}} for dynamic values.\n\
        \n\
        When to use: When authoring a workflow and need to know what activities exist and \
        what parameters they accept.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListActivities {}

/// Number of activities in the hardcoded catalog. Must match ACTIVITY_CATALOG length.
/// Update this when adding or removing activities.
const ACTIVITY_CATALOG_COUNT: usize = 8;

impl ListActivities {
    /// NOTE: This catalog is manually maintained. When activities are added to
    /// or removed from the worker, this list and its "total" count must be
    /// updated to match. See worker/src/activities/ for the source of truth.
    pub fn call_tool(&self) -> Result<CallToolResult, CallToolError> {
        let activities = serde_json::json!([
            {
                "name": "echo",
                "worker": "std",
                "description": "Returns its input unchanged. Useful for testing and debugging workflows.",
                "parameters": {
                    "message": "Any value to echo back (string, object, or array)"
                },
                "outputs": ["result"],
                "settings": {
                    "timeout": "Activity-level timeout in seconds"
                }
            },
            {
                "name": "http_request",
                "worker": "std",
                "description": "Make HTTP/REST API requests with configurable method, headers, body, and retries.",
                "parameters": {
                    "method": "HTTP method: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS",
                    "url": "Target URL (supports template expressions)",
                    "headers": "Optional HTTP headers as key-value pairs",
                    "body": "Optional request body (JSON, text, or form data)",
                    "query": "Optional query parameters as key-value pairs",
                    "timeout": "Request timeout in seconds (default: 30)"
                },
                "outputs": ["response", "status_code", "headers"],
                "settings": {
                    "retry": "Configurable retry policy (max_attempts, strategy, backoff)",
                    "timeout": "Activity-level timeout in seconds"
                }
            },
            {
                "name": "llm_prompt",
                "worker": "std",
                "description": "Call LLM APIs with multi-provider support, fallback chains, and budget controls. Supports Anthropic, OpenAI, Google, and Ollama.",
                "parameters": {
                    "model": "Model identifier or array for fallback (e.g. \"anthropic/claude-sonnet-4-5-20250929\")",
                    "prompt": "User prompt text (supports template expressions)",
                    "system": "Optional system prompt",
                    "max_tokens": "Maximum tokens to generate (default: 1024)",
                    "temperature": "Sampling temperature 0.0–1.0 (default: 1.0)",
                    "tools": "Optional tool definitions for function calling",
                    "stream": "Enable streaming responses (default: false)"
                },
                "outputs": ["result", "cost_usd", "provider", "model", "usage"],
                "settings": {
                    "retry": "Retry with exponential backoff",
                    "budget": "Budget limit in USD — action: abort or skip",
                    "streaming": "WebSocket streaming for incremental token delivery"
                }
            },
            {
                "name": "embedding",
                "worker": "std",
                "description": "Generate text embeddings with provider fallback chains.",
                "parameters": {
                    "model": "Embedding model (e.g. \"openai/text-embedding-3-small\")",
                    "input": "Text or array of texts to embed",
                    "dimensions": "Optional output dimensions (model-dependent)"
                },
                "outputs": ["embeddings", "dimensions", "cost_usd"],
                "settings": {
                    "retry": "Retry with exponential backoff",
                    "budget": "Budget limit for embedding costs"
                }
            },
            {
                "name": "postgres_query",
                "worker": "std",
                "description": "Execute a PostgreSQL query (SELECT, INSERT, UPDATE, DELETE) with parameterized placeholders.",
                "parameters": {
                    "query": "SQL with $1, $2, … placeholders",
                    "params": "Array of values bound to placeholders",
                    "database_url": "Optional connection string (defaults to KRUXIAFLOW_DATABASE_URL)"
                },
                "outputs": ["rows", "row_count"],
                "settings": {
                    "retry": "Retry for transient errors",
                    "timeout": "Query timeout in seconds"
                }
            },
            {
                "name": "postgres_transaction",
                "worker": "std",
                "description": "Execute multiple SQL statements inside a single ACID transaction.",
                "parameters": {
                    "queries": "Array of SQL statements to run atomically",
                    "database_url": "Optional connection string (defaults to KRUXIAFLOW_DATABASE_URL)"
                },
                "outputs": ["results", "row_counts"],
                "settings": {
                    "retry": "Retry on serialization failures",
                    "isolation": "Transaction isolation level (default: READ COMMITTED)"
                }
            },
            {
                "name": "email_send",
                "worker": "std",
                "description": "Send an email via SMTP with plain-text or HTML body.",
                "parameters": {
                    "to": "Recipient address or array of addresses",
                    "from": "Sender address",
                    "subject": "Email subject",
                    "body": "Email body (plain text or HTML)",
                    "html": "true if body is HTML (default: false)",
                    "cc": "Optional CC addresses",
                    "bcc": "Optional BCC addresses"
                },
                "outputs": ["message_id", "status"],
                "settings": {
                    "retry": "Retry on SMTP failures",
                    "smtp": "SMTP config: host, port, auth credentials"
                }
            },
            {
                "name": "script",
                "worker": "python",
                "description": "Execute a Python script. Multiple worker pools are available with different pre-installed packages.",
                "parameters": {
                    "code": "Python source code to execute",
                    "globals": "Optional dict of variables injected into global scope",
                    "timeout": "Execution timeout in seconds (default: 300)"
                },
                "outputs": ["result", "stdout", "stderr"],
                "settings": {
                    "retry": "Retry for transient failures",
                    "timeout": "Script execution timeout"
                },
                "worker_pools": {
                    "py-std": "General utilities (httpx, orjson, pydantic, dateutil)",
                    "py-data": "ETL (pandas, polars, duckdb, sqlalchemy)",
                    "py-ml": "ML (sklearn, torch, numpy, scipy)",
                    "py-nlp": "NLP (transformers, spacy, tiktoken)"
                }
            }
        ]);

        let activity_list = activities.as_array().expect("activities must be an array");
        debug_assert_eq!(
            activity_list.len(),
            ACTIVITY_CATALOG_COUNT,
            "ACTIVITY_CATALOG_COUNT ({}) does not match actual catalog length ({}). \
             Update the constant when adding or removing activities.",
            ACTIVITY_CATALOG_COUNT,
            activity_list.len(),
        );

        let response = serde_json::json!({
            "activities": activities,
            "total": activity_list.len(),
            "note": "All activities support template expressions like {{INPUT.field}} and {{activity_key.output_name}} for dynamic parameter values"
        });

        Ok(CallToolResult::text_content(vec![TextContent::from(
            serde_json::to_string_pretty(&response)
                .map_err(|e| CallToolError::from_message(e.to_string()))?,
        )]))
    }
}

// ============================================================================
// Tool: get_workflow_authoring_guide
// ============================================================================

#[mcp_tool(
    name = "get_workflow_authoring_guide",
    description = "Get a comprehensive guide for authoring Kruxia Flow workflows.\n\
        \n\
        Covers YAML structure, template expressions, dependency patterns (sequential, \
        parallel, fan-in/fan-out, conditional), activity settings, and complete worked \
        examples. Start here when writing a new workflow from scratch.\n\
        \n\
        When to use: Before authoring a workflow. Pair with list_activities to see the \
        full set of available activities.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetWorkflowAuthoringGuide {}

impl GetWorkflowAuthoringGuide {
    /// NOTE: This guide is manually maintained. Model names in examples
    /// (e.g. anthropic/claude-sonnet-4-5-20250929) will become stale as models change.
    /// Update examples when new models are released.
    pub fn call_tool(&self) -> Result<CallToolResult, CallToolError> {
        let guide = serde_json::json!({
            "yaml_structure": {
                "description": "Top-level structure of a workflow definition file",
                "required_fields": {
                    "name": "Unique workflow identifier (lowercase letters, digits, hyphens, underscores)",
                    "activities": "Map of activity key → activity definition (at least one required)"
                },
                "activity_fields": {
                    "worker": "(required) Worker type that executes this activity — std, python, py-std, py-data, py-ml, py-nlp",
                    "activity_name": "(required) Name of the activity within the worker (e.g. http_request, llm_prompt)",
                    "parameters": "Key-value map passed to the activity at runtime. Supports template expressions.",
                    "depends_on": "List of activity keys that must complete before this one starts. Each entry can be a plain key or {activity_key, conditions}.",
                    "dependency_of": "Inverse of depends_on — declares that other activities depend on this one. Normalised to depends_on internally.",
                    "settings": "Per-activity settings: retry, timeout, budget, scheduling, signals (see settings_configuration)",
                    "output_definitions": "Optional: declare expected outputs for validation and documentation",
                    "iteration_scoped": "If true, outputs are stored per-loop iteration (default: false)",
                    "iteration_limit": "Max loop iterations before failing (prevents infinite loops)",
                    "streaming": "Enable token-level streaming for LLM activities"
                },
                "example": "name: fetch-and-summarise\nactivities:\n  fetch:\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: GET\n      url: \"{{INPUT.url}}\"\n  summarise:\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-4-5-20250929\n      prompt: \"Summarise this text: {{fetch.response}}\"\n    depends_on: [fetch]"
            },
            "template_expressions": {
                "description": "Dynamic values resolved at runtime using {{source.path}} syntax",
                "syntax": "{{SOURCE.field.nested.path}} — dot-separated access into JSON. Array indexing with [N] is supported.",
                "sources": {
                    "INPUT": "The input payload provided when the workflow is submitted (e.g. {{INPUT.user_id}})",
                    "<activity_key>": "Output of a completed activity referenced by its key (e.g. {{fetch.response}}). The activity must be in depends_on (directly or transitively).",
                    "SECRET": "Secrets injected at runtime (e.g. {{SECRET.api_key}}). Not stored in the definition.",
                    "WORKFLOW": "Workflow-level metadata: {{WORKFLOW.id}}, {{WORKFLOW.name}}, {{WORKFLOW.submitted_at}}"
                },
                "examples": [
                    "{{INPUT.query}} — value of 'query' key from workflow input",
                    "{{fetch_data.rows[0].name}} — first row's name field from fetch_data activity output",
                    "{{SECRET.openai_key}} — secret injected at runtime",
                    "{{WORKFLOW.id}} — UUID of the running workflow instance"
                ]
            },
            "dependency_patterns": {
                "description": "How to control execution order and data flow between activities",
                "sequential": {
                    "description": "A → B → C. Each step waits for the previous to complete.",
                    "example": "activities:\n  a:\n    worker: std\n    activity_name: echo\n    parameters: {message: step-a}\n  b:\n    worker: std\n    activity_name: echo\n    parameters: {message: step-b}\n    depends_on: [a]\n  c:\n    worker: std\n    activity_name: echo\n    parameters: {message: step-c}\n    depends_on: [b]"
                },
                "parallel": {
                    "description": "Activities with no shared dependency run concurrently.",
                    "example": "activities:\n  fetch_weather:\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.weather_url}}\"}\n  fetch_stocks:\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.stocks_url}}\"}\n  # Both run in parallel — neither depends on the other"
                },
                "fan_in": {
                    "description": "Multiple activities feed into one (fan-in). The downstream activity waits for ALL upstream activities.",
                    "example": "activities:\n  fetch_a: { worker: std, activity_name: http_request, parameters: {method: GET, url: \"{{INPUT.url_a}}\"} }\n  fetch_b: { worker: std, activity_name: http_request, parameters: {method: GET, url: \"{{INPUT.url_b}}\"} }\n  combine:\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-4-5-20250929\n      prompt: \"Combine: {{fetch_a.response}} and {{fetch_b.response}}\"\n    depends_on: [fetch_a, fetch_b]"
                },
                "conditional": {
                    "description": "An activity only runs if a condition on an upstream output is met.",
                    "example": "activities:\n  check:\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.status_url}}\"}\n  on_success:\n    worker: std\n    activity_name: llm_prompt\n    parameters: {model: anthropic/claude-sonnet-4-5-20250929, prompt: \"Process success\"}\n    depends_on:\n      - activity_key: check\n        conditions: [\"check.status_code == 200\"]"
                }
            },
            "settings_configuration": {
                "retry_policy": {
                    "description": "How to retry a failed activity",
                    "fields": {
                        "max_attempts": "Total attempts including the first try (default: 1 = no retry)",
                        "strategy": "exponential (default) or fixed",
                        "backoff_base_secs": "Base delay in seconds for exponential backoff (default: 1)",
                        "backoff_max_secs": "Maximum delay cap (default: 300)"
                    },
                    "example": "settings:\n  retry:\n    max_attempts: 3\n    strategy: exponential\n    backoff_base_secs: 2\n    backoff_max_secs: 60"
                },
                "timeout": {
                    "description": "Maximum wall-clock time for an activity",
                    "example": "settings:\n  timeout: 300  # 5 minutes"
                },
                "budget": {
                    "description": "Per-activity cost cap (useful for LLM activities)",
                    "fields": {
                        "limit_usd": "Maximum spend in USD",
                        "action": "abort (fail the activity) or skip (return empty result)"
                    },
                    "example": "settings:\n  budget:\n    limit_usd: 0.50\n    action: abort"
                },
                "scheduling": {
                    "description": "Delay activity start by a fixed amount after its dependencies complete",
                    "example": "settings:\n  delay_secs: 60  # Wait 60 s after dependencies finish"
                },
                "signals": {
                    "description": "Pause the activity until an external signal is received",
                    "example": "settings:\n  wait_for_signal:\n    signal_name: approval\n    timeout: 3600  # Fail after 1 hour if no signal"
                }
            },
            "complete_examples": {
                "simple_sequential": {
                    "description": "Fetch a URL, then summarise the response with an LLM",
                    "yaml": "name: fetch-and-summarise\nactivities:\n  fetch:\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: GET\n      url: \"{{INPUT.url}}\"\n    settings:\n      timeout: 30\n      retry:\n        max_attempts: 3\n  summarise:\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-4-5-20250929\n      system: You are a concise summariser.\n      prompt: \"Summarise: {{fetch.response}}\"\n      max_tokens: 256\n    depends_on: [fetch]\n    settings:\n      budget:\n        limit_usd: 0.10\n        action: abort"
                },
                "parallel_fan_in": {
                    "description": "Query two databases in parallel, then merge results",
                    "yaml": "name: parallel-merge\nactivities:\n  query_users:\n    worker: std\n    activity_name: postgres_query\n    parameters:\n      query: \"SELECT * FROM users WHERE active = $1\"\n      params: [true]\n  query_orders:\n    worker: std\n    activity_name: postgres_query\n    parameters:\n      query: \"SELECT * FROM orders WHERE status = $1\"\n      params: [pending]\n  merge:\n    worker: python\n    activity_name: script\n    parameters:\n      code: |\n        users = INPUT[\"query_users_rows\"]\n        orders = INPUT[\"query_orders_rows\"]\n        result = {\"users\": users, \"orders\": orders, \"total\": len(users) + len(orders)}\n    depends_on: [query_users, query_orders]"
                },
                "conditional_with_signal": {
                    "description": "Classify input, route to approval if high-value, then process",
                    "yaml": "name: approval-flow\nactivities:\n  classify:\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-4-5-20250929\n      prompt: \"Classify this request as high or low value: {{INPUT.request}}\"\n      max_tokens: 16\n  await_approval:\n    worker: std\n    activity_name: echo\n    parameters:\n      message: Waiting for approval\n    depends_on:\n      - activity_key: classify\n        conditions: [\"classify.result == high\"]\n    settings:\n      wait_for_signal:\n        signal_name: approval\n        timeout: 86400\n  process:\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: POST\n      url: \"{{INPUT.callback_url}}\"\n      body: \"{\\\"status\\\": \\\"approved\\\"}\"\n    depends_on: [await_approval]"
                }
            },
            "best_practices": {
                "workflow_design": [
                    "Keep workflows focused — one workflow per business process",
                    "Use descriptive activity keys that read like prose (e.g. fetch_user_data, not step1)",
                    "Set explicit timeouts on every activity that touches external services",
                    "Use retry policies on network-facing activities (http_request, llm_prompt)",
                    "Prefer parallel execution where possible — fan-out early, fan-in late"
                ],
                "error_handling": [
                    "Set iteration_limit on any loop to prevent runaway workflows",
                    "Use budget limits on LLM activities to cap unexpected costs",
                    "Use conditional dependencies to handle expected failure paths",
                    "Set wait_for_signal timeouts to avoid workflows that hang forever"
                ],
                "security": [
                    "Pass secrets via SECRET references, never hardcode API keys in parameters",
                    "Scope postgres_query permissions — use read-only connections where possible",
                    "Validate external API responses before passing them to LLM prompts (injection risk)"
                ],
                "template_expressions": [
                    "Only reference activities listed in depends_on (direct or transitive)",
                    "Use array indexing sparingly — prefer activities that return structured objects",
                    "Test template paths with a simple echo activity first when debugging"
                ]
            }
        });

        Ok(CallToolResult::text_content(vec![TextContent::from(
            serde_json::to_string_pretty(&guide)
                .map_err(|e| CallToolError::from_message(e.to_string()))?,
        )]))
    }
}

// ============================================================================
// Enum + routing glue (generated by tool_box! macro)
// ============================================================================

tool_box!(
    DiscoveryTools,
    [
        ListWorkflowDefinitions,
        GetWorkflowDefinition,
        ListActivities,
        GetWorkflowAuthoringGuide
    ]
);

// ============================================================================
// Async implementations for DB-backed tools
// ============================================================================

/// Execute list_workflow_definitions against the database.
///
/// Returns the latest version of each workflow, optionally filtered by name prefix.
pub async fn run_list_workflow_definitions(
    pool: &PgPool,
    params: &ListWorkflowDefinitions,
) -> Result<CallToolResult, CallToolError> {
    let repo = kruxiaflow_core::WorkflowDefinitionRepository::new(pool.clone());

    let definitions = repo.list().await.map_err(|e| {
        tracing::error!("list_workflow_definitions: {e:?}");
        CallToolError::from_message(format!("Database error listing workflow definitions: {e}"))
    })?;

    // repo.list() returns all versions sorted by (name ASC, created_at DESC).
    // Deduplicate to latest version per name by keeping the first occurrence of each name.
    let mut seen = std::collections::HashSet::new();
    let mut latest: Vec<_> = Vec::new();
    for d in definitions {
        if seen.insert(d.name.clone()) {
            latest.push(d);
        }
    }

    // Optional name-prefix filter
    if let Some(ref prefix) = params.name {
        latest.retain(|d| d.name.starts_with(prefix.as_str()));
    }

    let summaries: Vec<serde_json::Value> = latest
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name,
                "version": d.version,
                "activity_count": d.activities.len(),
                "created_at": d.created_at.to_rfc3339(),
            })
        })
        .collect();

    let response = serde_json::json!({
        "definitions": summaries,
        "total": summaries.len(),
    });

    Ok(CallToolResult::text_content(vec![TextContent::from(
        serde_json::to_string_pretty(&response)
            .map_err(|e| CallToolError::from_message(e.to_string()))?,
    )]))
}

/// Execute get_workflow_definition against the database.
///
/// Returns the full definition for the given name + version (or latest if version is omitted).
/// Returns an error JSON payload (not a Rust Err) when the workflow is not found, matching
/// the Python MCP server convention.
pub async fn run_get_workflow_definition(
    pool: &PgPool,
    params: &GetWorkflowDefinition,
) -> Result<CallToolResult, CallToolError> {
    let repo = kruxiaflow_core::WorkflowDefinitionRepository::new(pool.clone());

    let stored = if let Some(ref version) = params.version {
        repo.get(&params.name, version).await.map_err(|e| {
            tracing::error!("get_workflow_definition: {e:?}");
            CallToolError::from_message(format!(
                "Error retrieving workflow '{}' version '{}': {e}",
                params.name, version
            ))
        })?
    } else {
        repo.get_latest(&params.name).await.map_err(|e| {
            tracing::error!("get_workflow_definition: {e:?}");
            CallToolError::from_message(format!(
                "Error retrieving latest version of workflow '{}': {e}",
                params.name
            ))
        })?
    };

    let response = match stored {
        Some(def) => {
            // Serialise activities as-is from the stored JSONB representation
            let activities: serde_json::Value = serde_json::to_value(&def.activities)
                .map_err(|e| CallToolError::from_message(format!("Serialization error: {e}")))?;

            serde_json::json!({
                "name": def.name,
                "version": def.version,
                "activities": activities,
                "created_at": def.created_at.to_rfc3339(),
            })
        }
        None => {
            let version_label = params.version.as_deref().unwrap_or("latest");
            return error_response(&serde_json::json!({
                "error": format!(
                    "Workflow '{}' (version: {}) not found",
                    params.name, version_label
                ),
                "name": params.name,
                "version": params.version,
            }));
        }
    };

    Ok(CallToolResult::text_content(vec![TextContent::from(
        serde_json::to_string_pretty(&response)
            .map_err(|e| CallToolError::from_message(e.to_string()))?,
    )]))
}

/// Wrapper for ListActivities — uniform run_* dispatch pattern.
pub fn run_list_activities(params: &ListActivities) -> Result<CallToolResult, CallToolError> {
    params.call_tool()
}

/// Wrapper for GetWorkflowAuthoringGuide — uniform run_* dispatch pattern.
pub fn run_get_workflow_authoring_guide(
    params: &GetWorkflowAuthoringGuide,
) -> Result<CallToolResult, CallToolError> {
    params.call_tool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_activities_returns_correct_count() {
        let tool = ListActivities {};
        let result = tool.call_tool().unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            let total = parsed["total"].as_u64().unwrap() as usize;
            let activities = parsed["activities"].as_array().unwrap();
            assert_eq!(total, activities.len());
            assert_eq!(total, ACTIVITY_CATALOG_COUNT);
        }
    }

    #[test]
    fn test_list_activities_all_have_required_fields() {
        let tool = ListActivities {};
        let result = tool.call_tool().unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            for activity in parsed["activities"].as_array().unwrap() {
                assert!(activity.get("name").is_some(), "Activity missing 'name'");
                assert!(activity.get("worker").is_some(), "Activity missing 'worker'");
                assert!(
                    activity.get("description").is_some(),
                    "Activity missing 'description'"
                );
                assert!(
                    activity.get("parameters").is_some(),
                    "Activity missing 'parameters'"
                );
                assert!(
                    activity.get("outputs").is_some(),
                    "Activity missing 'outputs'"
                );
            }
        }
    }

    #[test]
    fn test_list_activities_unique_names() {
        let tool = ListActivities {};
        let result = tool.call_tool().unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            let names: Vec<&str> = parsed["activities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["name"].as_str().unwrap())
                .collect();
            let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
            assert_eq!(names.len(), unique.len(), "Duplicate activity names found");
        }
    }

    #[test]
    fn test_get_workflow_authoring_guide_returns_content() {
        let tool = GetWorkflowAuthoringGuide {};
        let result = tool.call_tool().unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            assert!(parsed.get("yaml_structure").is_some());
            assert!(parsed.get("template_expressions").is_some());
            assert!(parsed.get("dependency_patterns").is_some());
            assert!(parsed.get("settings_configuration").is_some());
            assert!(parsed.get("complete_examples").is_some());
            assert!(parsed.get("best_practices").is_some());
        }
    }
}
