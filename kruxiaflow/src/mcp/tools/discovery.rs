/// MCP Discovery Tools
///
/// Seven read-only tools that let AI agents discover what's available in Kruxia Flow:
/// - list_workflow_definitions: what workflows are deployed
/// - get_workflow_definition: full structure of a specific workflow
/// - list_activities: what activity types exist
/// - get_workflow_authoring_guide: how to write workflows
/// - search_llm_models: model catalog with live pricing
/// - list_llm_providers: configured providers and capabilities
/// - check_system_health: readiness of database, event source, and queue
/// - get_run_code_contract: what sandboxed run_code steps may use (when enabled)
use rust_decimal::prelude::*;
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{CallToolResult, TextContent, schema_utils::CallToolError};

use super::{error_response, text_response};
use rust_mcp_sdk::tool_box;
use sqlx::{PgPool, Row};

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
        what parameters they accept. On servers with sandboxed code enabled the catalog \
        includes run_code; call get_run_code_contract for its full contract.",
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
    ///
    /// `run_code` (sandboxed Python) is appended when this server's built-in
    /// worker has it enabled; its entry is built from the worker's own
    /// contract, so it cannot drift.
    pub fn call_tool(
        &self,
        run_code: Option<&serde_json::Value>,
    ) -> Result<CallToolResult, CallToolError> {
        let mut activities = serde_json::json!([
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
                "security": "NOT sandboxed: runs CPython with all builtins inside the worker process, so the code can read the worker's environment and reach the network. Use only for trusted, developer-written code. Never pass model-written or customer-supplied code to script; use run_code where it is available.",
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

        let activity_list = activities
            .as_array_mut()
            .expect("activities must be an array");
        debug_assert_eq!(
            activity_list.len(),
            ACTIVITY_CATALOG_COUNT,
            "ACTIVITY_CATALOG_COUNT ({}) does not match actual catalog length ({}). \
             Update the constant when adding or removing activities.",
            ACTIVITY_CATALOG_COUNT,
            activity_list.len(),
        );
        if let Some(contract) = run_code {
            activity_list.push(run_code_catalog_entry(contract));
        }
        let total = activity_list.len();

        let response = serde_json::json!({
            "activities": activities,
            "total": total,
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
    pub fn call_tool(
        &self,
        run_code: Option<&serde_json::Value>,
    ) -> Result<CallToolResult, CallToolError> {
        let mut guide = serde_json::json!({
            "note": "Model identifiers in the examples below may be outdated. Call \
                     search_llm_models for the current model catalog with live pricing \
                     before choosing a model, and estimate_workflow_cost before \
                     submitting with a budget.",
            "yaml_structure": {
                "description": "Top-level structure of a workflow definition file",
                "required_fields": {
                    "name": "Unique workflow identifier (lowercase letters, digits, hyphens, underscores)",
                    "activities": "LIST of activity definitions (at least one required). Each entry carries its unique identifier in the 'key' field — a map keyed by activity name is NOT accepted."
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
                "example": "name: fetch-and-summarise\nactivities:\n  - key: fetch\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: GET\n      url: \"{{INPUT.url}}\"\n  - key: summarise\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: \"Summarise this text: {{fetch.response}}\"\n    depends_on: [fetch]"
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
                    "example": "activities:\n  - key: a\n    worker: std\n    activity_name: echo\n    parameters: {message: step-a}\n  - key: b\n    worker: std\n    activity_name: echo\n    parameters: {message: step-b}\n    depends_on: [a]\n  - key: c\n    worker: std\n    activity_name: echo\n    parameters: {message: step-c}\n    depends_on: [b]"
                },
                "parallel": {
                    "description": "Activities with no shared dependency run concurrently.",
                    "example": "activities:\n  - key: fetch_weather\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.weather_url}}\"}\n  - key: fetch_stocks\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.stocks_url}}\"}\n  # Both run in parallel — neither depends on the other"
                },
                "fan_in": {
                    "description": "Multiple activities feed into one (fan-in). The downstream activity waits for ALL upstream activities.",
                    "example": "activities:\n  - { key: fetch_a, worker: std, activity_name: http_request, parameters: {method: GET, url: \"{{INPUT.url_a}}\"} }\n  - { key: fetch_b, worker: std, activity_name: http_request, parameters: {method: GET, url: \"{{INPUT.url_b}}\"} }\n  - key: combine\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: \"Combine: {{fetch_a.response}} and {{fetch_b.response}}\"\n    depends_on: [fetch_a, fetch_b]"
                },
                "conditional": {
                    "description": "An activity only runs if a condition on an upstream output is met.",
                    "example": "activities:\n  - key: check\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.status_url}}\"}\n  - key: on_success\n    worker: std\n    activity_name: llm_prompt\n    parameters: {model: anthropic/claude-sonnet-5, prompt: \"Process success\"}\n    depends_on:\n      - activity_key: check\n        conditions: [\"check.status_code == 200\"]"
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
                        "limit": "Maximum spend in USD (field name is 'limit', not 'limit_usd')",
                        "action": "abort (fail the activity) or skip (return empty result)"
                    },
                    "example": "settings:\n  budget:\n    limit: 0.50\n    action: abort"
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
                    "yaml": "name: fetch-and-summarise\nactivities:\n  - key: fetch\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: GET\n      url: \"{{INPUT.url}}\"\n    settings:\n      timeout: 30\n      retry:\n        max_attempts: 3\n  - key: summarise\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      system: You are a concise summariser.\n      prompt: \"Summarise: {{fetch.response}}\"\n      max_tokens: 256\n    depends_on: [fetch]\n    settings:\n      budget:\n        limit: 0.10\n        action: abort"
                },
                "parallel_fan_in": {
                    "description": "Query two databases in parallel, then merge results",
                    "yaml": "name: parallel-merge\nactivities:\n  - key: query_users\n    worker: std\n    activity_name: postgres_query\n    parameters:\n      query: \"SELECT * FROM users WHERE active = $1\"\n      params: [true]\n  - key: query_orders\n    worker: std\n    activity_name: postgres_query\n    parameters:\n      query: \"SELECT * FROM orders WHERE status = $1\"\n      params: [pending]\n  - key: merge\n    worker: python\n    activity_name: script\n    parameters:\n      code: |\n        users = INPUT[\"query_users_rows\"]\n        orders = INPUT[\"query_orders_rows\"]\n        result = {\"users\": users, \"orders\": orders, \"total\": len(users) + len(orders)}\n    depends_on: [query_users, query_orders]"
                },
                "conditional_with_signal": {
                    "description": "Classify input, route to approval if high-value, then process",
                    "yaml": "name: approval-flow\nactivities:\n  - key: classify\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: \"Classify this request as high or low value: {{INPUT.request}}\"\n      max_tokens: 16\n  - key: await_approval\n    worker: std\n    activity_name: echo\n    parameters:\n      message: Waiting for approval\n    depends_on:\n      - activity_key: classify\n        conditions: [\"classify.result == high\"]\n    settings:\n      wait_for_signal:\n        signal_name: approval\n        timeout: 86400\n  - key: process\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: POST\n      url: \"{{INPUT.callback_url}}\"\n      body: \"{\\\"status\\\": \\\"approved\\\"}\"\n    depends_on: [await_approval]"
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

        add_template_guard_practice(&mut guide);
        if let Some(contract) = run_code {
            guide["sandboxed_code"] = run_code_guide_section(contract);
        }

        Ok(CallToolResult::text_content(vec![TextContent::from(
            serde_json::to_string_pretty(&guide)
                .map_err(|e| CallToolError::from_message(e.to_string()))?,
        )]))
    }
}

// ============================================================================
// Tool: search_llm_models
// ============================================================================

#[mcp_tool(
    name = "search_llm_models",
    description = "Search the LLM model catalog with live pricing data.\n\
        \n\
        Returns models with input/output/cached pricing per million tokens, context \
        window sizes, and capabilities — straight from the engine's model catalog, the \
        same pricing the engine uses to meter costs and enforce budgets. Filter by \
        provider, name substring, and/or maximum input price.\n\
        \n\
        When to use: To make budget-aware model selections before authoring or \
        submitting a workflow. Typical flow: search pricing here, pick a model, \
        estimate with estimate_workflow_cost, then submit_workflow with \
        budget_limit_usd.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SearchLlmModels {
    /// Filter by provider (e.g. "anthropic", "openai", "google", "ollama")
    pub provider: Option<String>,

    /// Case-insensitive substring match on the model name (e.g. "claude", "gpt")
    pub name: Option<String>,

    /// Only return models costing at most this much (USD) per million input tokens
    pub max_input_price_per_million: Option<f64>,
}

// ============================================================================
// Tool: list_llm_providers
// ============================================================================

#[mcp_tool(
    name = "list_llm_providers",
    description = "List all configured LLM providers with their capabilities.\n\
        \n\
        Returns each provider's supported features (completion, embeddings, \
        streaming), whether it requires an API key, and how many models it has in \
        the catalog. Includes self-hosted options like Ollama when configured.\n\
        \n\
        When to use: To discover what providers are available before searching for \
        specific models with search_llm_models.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListLlmProviders {}

// ============================================================================
// Tool: check_system_health
// ============================================================================

#[mcp_tool(
    name = "check_system_health",
    description = "Check whether Kruxia Flow is healthy and ready to accept workflows.\n\
        \n\
        Verifies database connectivity, the event source, and the activity queue — \
        the same checks as the REST /health/ready endpoint.\n\
        \n\
        When to use: Before submitting workflows if you want to handle downtime \
        gracefully, or to diagnose why submissions are failing.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CheckSystemHealth {}

// ============================================================================
// Tool: get_run_code_contract
// ============================================================================

#[mcp_tool(
    name = "get_run_code_contract",
    description = "Get the contract for writing sandboxed run_code steps on this server.\n\
        \n\
        run_code runs untrusted Python (typically code an LLM step just wrote) in a \
        Pydantic Monty sandbox: no filesystem, network or environment, only the \
        capabilities a step grants, under hard limits. Returns the capabilities with \
        their Python signatures, the importable standard-library modules, this \
        server's limit ceilings, the result shape, the error kinds and the rules the \
        code must follow (for example: the last line's value is the result).\n\
        \n\
        When to use: Before writing a run_code step or the prompt of an llm_prompt \
        step that writes code for one. Returns an error if run_code is not enabled \
        on this server.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct GetRunCodeContract {}

/// The run_code entry for list_activities, from the worker's contract
fn run_code_catalog_entry(contract: &serde_json::Value) -> serde_json::Value {
    let capability_names: Vec<serde_json::Value> = contract["capabilities"]
        .as_array()
        .map(|caps| caps.iter().map(|c| c["name"].clone()).collect())
        .unwrap_or_default();
    serde_json::json!({
        "name": "run_code",
        "worker": "std",
        "description": "Run untrusted Python (typically code an LLM step just wrote) in a Pydantic Monty sandbox: no filesystem, network or environment, only the capabilities the step grants, under hard limits on memory, time, recursion and host calls. Code errors COMPLETE the step with result.ok == false and an error written for the model, so a loop can let it repair the code. Call get_run_code_contract for the full contract.",
        "parameters": {
            "code": "(required) Python source; a single surrounding Markdown code fence is stripped, so {{writer.result.content}} works. The last line's value is the result",
            "inputs": "Object; each key becomes a global variable (keys must be Python identifiers)",
            "capabilities": "Names of host functions the code may call, from the server's list",
            "type_check": "Type-check against inputs and capability signatures before running (default: true)",
            "limits": "max_memory, max_feed_duration_secs, max_recursion_depth, max_suspensions, max_total_sleep_secs; each at most the server's ceiling"
        },
        "outputs": ["result"],
        "result": contract["result"].clone(),
        "available_capabilities": capability_names,
        "settings": {
            "retry": "Only infrastructure failures fail (and retry) the activity; code errors complete it with result.ok == false"
        }
    })
}

/// The authoring guide's sandboxed-code section, from the worker's contract
fn run_code_guide_section(contract: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "description": "run_code runs Python that an LLM wrote, safely. Use it for model-written business rules, transforms and glue code. Use the python worker's script activity only for trusted, developer-written code: it is not sandboxed.",
        "rules": contract["rules"].clone(),
        "available_modules": contract["available_modules"].clone(),
        "capabilities": contract["capabilities"].clone(),
        "limit_ceilings": contract["limit_ceilings"].clone(),
        "error_kinds": contract["error_kinds"].clone(),
        "patterns": {
            "branch_on_outcome": "depends_on: [{activity_key: run, conditions: [\"{{ run.result.ok }}\"]}]",
            "repair_loop": contract["repair_loop"].clone()
        },
        "example": "name: llm-rule-with-repair\nactivities:\n  - key: write\n    worker: std\n    activity_name: llm_prompt\n    iteration_limit: 3\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: |\n        Write Python that flags orders over $500. A variable `orders` holds a list of dicts with keys id and total.\n        The last line must be an expression: the list of flagged ids. Reply with only the code in one ```python block.\n        {% if run.status == 'completed' %}Your previous code failed: {{ run.result.error.message }}{% endif %}\n    depends_on:\n      - activity_key: run\n        conditions: [\"{{ run.result.ok == false }}\"]\n  - key: run\n    worker: std\n    activity_name: run_code\n    parameters:\n      code: \"{{ write.result.content }}\"\n      inputs: {orders: \"{{ INPUT.orders }}\"}\n      limits: {max_feed_duration_secs: 2}\n    depends_on: [write]\n  - key: use_result\n    worker: std\n    activity_name: echo\n    parameters: {flagged: \"{{ run.result.value }}\"}\n    depends_on:\n      - activity_key: run\n        conditions: [\"{{ run.result.ok }}\"]",
        "safety": "A sandbox makes code safe, not correct: for rules that matter, add a wait_for_signal approval step before the generated code is used in production."
    })
}

/// A template lesson for every workflow: guard references to steps that may
/// not have run by their status. Added in code, not in the guide literal.
fn add_template_guard_practice(guide: &mut serde_json::Value) {
    if let Some(list) = guide["best_practices"]["template_expressions"].as_array_mut() {
        list.push(serde_json::json!(
            "Guard references to a step that may not have run (a loop's first pass, an optional branch) with {% if step.status == 'completed' %}...{% endif %}. Every activity is in the template context with a status, so {% if step is defined %} is always true; an unguarded step.result reference fails the activity with TEMPLATE_ERROR"
        ));
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
        GetWorkflowAuthoringGuide,
        SearchLlmModels,
        ListLlmProviders,
        CheckSystemHealth,
        GetRunCodeContract
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
pub fn run_list_activities(
    params: &ListActivities,
    run_code: Option<&serde_json::Value>,
) -> Result<CallToolResult, CallToolError> {
    params.call_tool(run_code)
}

/// Wrapper for GetWorkflowAuthoringGuide — uniform run_* dispatch pattern.
pub fn run_get_workflow_authoring_guide(
    params: &GetWorkflowAuthoringGuide,
    run_code: Option<&serde_json::Value>,
) -> Result<CallToolResult, CallToolError> {
    params.call_tool(run_code)
}

/// get_run_code_contract: the contract, or an error explaining how to enable it
pub fn run_get_run_code_contract(
    _params: &GetRunCodeContract,
    run_code: Option<&serde_json::Value>,
) -> Result<CallToolResult, CallToolError> {
    match run_code {
        Some(contract) => text_response(contract),
        None => error_response(&serde_json::json!({
            "error": "run_code is not enabled on this server",
            "how_to_enable": "Use the Docker image (it ships the Monty worker and enables run_code), or build with --features monty and set KRUXIAFLOW_MONTY_BINARY",
            "note": "This server reports its own built-in worker. If run_code runs on separate `kruxiaflow worker` processes, their configuration is not visible here.",
        })),
    }
}

/// Search llm_models with live pricing, filtered by provider, name substring,
/// and/or maximum input price.
pub async fn run_search_llm_models(
    pool: &PgPool,
    params: &SearchLlmModels,
) -> Result<CallToolResult, CallToolError> {
    let max_input_price = match params.max_input_price_per_million {
        Some(p) => {
            let decimal = Decimal::from_f64_retain(p).filter(|d| d.is_sign_positive());
            if decimal.is_none() {
                return error_response(&serde_json::json!({
                    "error": "max_input_price_per_million must be a non-negative number",
                    "max_input_price_per_million": p,
                }));
            }
            decimal
        }
        None => None,
    };

    // TODO(#9): Runtime query for now; migrate to a stored proc with
    // compile-time validation per project conventions.
    let rows = sqlx::query(
        "SELECT provider, name, display_name, \
         input_price_per_million, output_price_per_million, \
         cached_input_price_per_million, cache_write_price_per_million, \
         supports_completion, supports_embeddings, \
         context_window, max_output_tokens \
         FROM llm_models \
         WHERE ($1::text IS NULL OR provider = $1) \
           AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%') \
           AND ($3::numeric IS NULL OR input_price_per_million <= $3) \
         ORDER BY provider, name",
    )
    .bind(&params.provider)
    .bind(&params.name)
    .bind(max_input_price)
    .fetch_all(pool)
    .await
    .map_err(|e| CallToolError::from_message(format!("Database error searching models: {e}")))?;

    let models: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mut capabilities: Vec<&str> = Vec::new();
            if row.get::<bool, _>("supports_completion") {
                capabilities.push("completion");
            }
            if row.get::<bool, _>("supports_embeddings") {
                capabilities.push("embeddings");
            }
            serde_json::json!({
                "provider": row.get::<String, _>("provider"),
                "name": row.get::<String, _>("name"),
                "display_name": row.get::<String, _>("display_name"),
                "input_price_per_million": row.get::<Decimal, _>("input_price_per_million").to_f64(),
                "output_price_per_million": row.get::<Decimal, _>("output_price_per_million").to_f64(),
                "cached_input_price_per_million": row
                    .get::<Option<Decimal>, _>("cached_input_price_per_million")
                    .and_then(|d| d.to_f64()),
                "cache_write_price_per_million": row
                    .get::<Option<Decimal>, _>("cache_write_price_per_million")
                    .and_then(|d| d.to_f64()),
                "capabilities": capabilities,
                "context_window": row.get::<Option<i32>, _>("context_window"),
                "max_output_tokens": row.get::<Option<i32>, _>("max_output_tokens"),
            })
        })
        .collect();

    text_response(&serde_json::json!({
        "models": models,
        "total": models.len(),
    }))
}

/// List llm_providers with capabilities and model counts.
pub async fn run_list_llm_providers(
    pool: &PgPool,
    _params: &ListLlmProviders,
) -> Result<CallToolResult, CallToolError> {
    // TODO(#9): Runtime query for now; migrate to a stored proc with
    // compile-time validation per project conventions.
    let rows = sqlx::query(
        "SELECT p.name, p.display_name, \
         p.supports_completion, p.supports_embeddings, p.supports_streaming, \
         p.requires_api_key, \
         (SELECT COUNT(*) FROM llm_models m WHERE m.provider = p.name) AS model_count \
         FROM llm_providers p \
         ORDER BY p.name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| CallToolError::from_message(format!("Database error listing providers: {e}")))?;

    let providers: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mut capabilities: Vec<&str> = Vec::new();
            if row.get::<bool, _>("supports_completion") {
                capabilities.push("completion");
            }
            if row.get::<bool, _>("supports_embeddings") {
                capabilities.push("embeddings");
            }
            if row.get::<bool, _>("supports_streaming") {
                capabilities.push("streaming");
            }
            serde_json::json!({
                "name": row.get::<String, _>("name"),
                "display_name": row.get::<String, _>("display_name"),
                "capabilities": capabilities,
                "requires_api_key": row.get::<bool, _>("requires_api_key"),
                "model_count": row.get::<i64, _>("model_count"),
            })
        })
        .collect();

    text_response(&serde_json::json!({
        "providers": providers,
        "total": providers.len(),
    }))
}

/// Run the same readiness checks as the REST /health/ready endpoint.
pub async fn run_check_system_health(
    pool: &PgPool,
    _params: &CheckSystemHealth,
) -> Result<CallToolResult, CallToolError> {
    use kruxiaflow_api::health::{
        check_activity_queue_health, check_database_health, check_event_source_health,
    };

    let (db_result, event_source_result, queue_result) = tokio::join!(
        check_database_health(pool),
        check_event_source_health(pool),
        check_activity_queue_health(pool)
    );

    let describe = |result: &Result<(), kruxiaflow_api::health::HealthCheckError>| match result {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("error: {e}"),
    };

    let database = describe(&db_result);
    let event_source = describe(&event_source_result);
    let activity_queue = describe(&queue_result);
    let ready = db_result.is_ok() && event_source_result.is_ok() && queue_result.is_ok();

    text_response(&serde_json::json!({
        "status": if ready { "ready" } else { "not_ready" },
        "checks": {
            "database": database,
            "event_source": event_source,
            "activity_queue": activity_queue,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_activities_returns_correct_count() {
        let tool = ListActivities {};
        let result = tool.call_tool(None).unwrap();
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
        let result = tool.call_tool(None).unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            for activity in parsed["activities"].as_array().unwrap() {
                assert!(activity.get("name").is_some(), "Activity missing 'name'");
                assert!(
                    activity.get("worker").is_some(),
                    "Activity missing 'worker'"
                );
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
        let result = tool.call_tool(None).unwrap();
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
        let result = tool.call_tool(None).unwrap();
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

    // =========================================================================
    // run_code (sandboxed Python) discovery
    // =========================================================================

    fn json_of(result: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(&result.content[0].as_text_content().unwrap().text).unwrap()
    }

    /// A contract shaped like kruxiaflow_std_worker::run_code_contract's,
    /// so these tests run without the `monty` feature
    fn sample_contract() -> serde_json::Value {
        serde_json::json!({
            "activity": "std.run_code",
            "monty_version": "1.6.0",
            "capabilities": [
                {"name": "log", "stub": "def log(message: str) -> None: ..."},
                {"name": "now", "stub": "def now() -> str: ..."}
            ],
            "available_modules": ["json", "re"],
            "limit_ceilings": {"max_memory": 1000},
            "rules": ["The last line's value is the result"],
            "result": {"ok": "...", "value": "...", "error": "...", "run": "..."},
            "error_kinds": {"type": "...", "runtime": "..."},
            "repair_loop": "back-edge on ok == false"
        })
    }

    fn activity<'a>(catalog: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
        catalog["activities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
    }

    #[test]
    fn test_run_code_hidden_when_not_enabled() {
        let catalog = json_of(&ListActivities {}.call_tool(None).unwrap());
        assert!(activity(&catalog, "run_code").is_none());
        assert_eq!(catalog["total"], ACTIVITY_CATALOG_COUNT);

        let guide = json_of(&GetWorkflowAuthoringGuide {}.call_tool(None).unwrap());
        assert!(guide.get("sandboxed_code").is_none());

        let result = run_get_run_code_contract(&GetRunCodeContract {}, None).unwrap();
        assert_eq!(result.is_error, Some(true));
        let body = json_of(&result);
        assert!(body["error"].as_str().unwrap().contains("not enabled"));
        assert!(
            body["how_to_enable"]
                .as_str()
                .unwrap()
                .contains("KRUXIAFLOW_MONTY_BINARY")
        );
    }

    #[test]
    fn test_run_code_listed_when_enabled() {
        let contract = sample_contract();
        let catalog = json_of(&ListActivities {}.call_tool(Some(&contract)).unwrap());
        assert_eq!(catalog["total"], ACTIVITY_CATALOG_COUNT + 1);
        let run_code = activity(&catalog, "run_code").expect("run_code listed");
        assert_eq!(run_code["worker"], "std");
        assert_eq!(
            run_code["available_capabilities"],
            serde_json::json!(["log", "now"])
        );
        assert_eq!(run_code["result"], contract["result"]);
        for param in ["code", "inputs", "capabilities", "type_check", "limits"] {
            assert!(
                run_code["parameters"].get(param).is_some(),
                "missing {param}"
            );
        }
    }

    #[test]
    fn test_script_is_marked_unsandboxed() {
        let catalog = json_of(&ListActivities {}.call_tool(None).unwrap());
        let script = activity(&catalog, "script").unwrap();
        assert!(
            script["security"]
                .as_str()
                .unwrap()
                .contains("NOT sandboxed")
        );
    }

    #[test]
    fn test_guide_sandboxed_code_section_when_enabled() {
        let contract = sample_contract();
        let guide = json_of(
            &GetWorkflowAuthoringGuide {}
                .call_tool(Some(&contract))
                .unwrap(),
        );
        let section = &guide["sandboxed_code"];
        assert_eq!(section["available_modules"], contract["available_modules"]);
        assert_eq!(section["capabilities"], contract["capabilities"]);
        assert_eq!(section["rules"], contract["rules"]);
        assert!(
            section["safety"]
                .as_str()
                .unwrap()
                .contains("safe, not correct")
        );
    }

    /// The guide's run_code example must be a valid workflow, or agents copy a
    /// broken one
    #[test]
    fn test_guide_run_code_example_is_a_valid_workflow() {
        let contract = sample_contract();
        let guide = json_of(
            &GetWorkflowAuthoringGuide {}
                .call_tool(Some(&contract))
                .unwrap(),
        );
        let yaml = guide["sandboxed_code"]["example"].as_str().unwrap();
        let definition = kruxiaflow_core::workflow::WorkflowDefinition::from_yaml(yaml)
            .unwrap_or_else(|e| panic!("example does not validate: {e:?}\n{yaml}"));
        let keys: Vec<&str> = definition
            .activities
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(keys, vec!["write", "run", "use_result"]);
        let write = &definition.activities[0];
        assert!(write.is_loop_activity, "write is the loop head");
        assert_eq!(write.iteration_limit, Some(3));
    }

    #[test]
    fn test_guide_teaches_the_status_guard() {
        let guide = json_of(&GetWorkflowAuthoringGuide {}.call_tool(None).unwrap());
        let practices = guide["best_practices"]["template_expressions"]
            .as_array()
            .unwrap();
        assert!(
            practices
                .iter()
                .any(|p| p.as_str().unwrap().contains("step.status == 'completed'"))
        );
    }

    #[test]
    fn test_get_run_code_contract_returns_it_when_enabled() {
        let contract = sample_contract();
        let result = run_get_run_code_contract(&GetRunCodeContract {}, Some(&contract)).unwrap();
        assert_ne!(result.is_error, Some(true));
        assert_eq!(json_of(&result), contract);
    }

    #[test]
    fn test_get_run_code_contract_is_registered() {
        let names: Vec<String> = DiscoveryTools::tools()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(
            names.iter().any(|n| n == "get_run_code_contract"),
            "{names:?}"
        );
    }

    /// With the real worker contract: the catalog reflects the server's
    /// actual capabilities, modules and ceilings
    #[cfg(feature = "monty")]
    #[test]
    fn test_real_contract_flows_into_the_catalog() {
        let config = kruxiaflow_std_worker::MontyConfig {
            binary: std::path::PathBuf::from("/monty"),
            min_processes: 1,
            max_processes: 2,
            request_timeout: std::time::Duration::from_secs(30),
            recycle_after: 1,
            max_memory_bytes: 12_345_678,
            max_duration: std::time::Duration::from_secs(4),
            max_recursion_depth: 500,
        };
        let contract = kruxiaflow_std_worker::run_code_contract(&config);
        assert_eq!(contract["limit_ceilings"]["max_memory"], 12_345_678);
        assert_eq!(contract["limit_ceilings"]["max_feed_duration_secs"], 4.0);
        assert_eq!(contract["limit_ceilings"]["max_recursion_depth"], 500);
        let modules = contract["available_modules"].as_array().unwrap();
        assert!(modules.iter().any(|m| m == "json"));
        assert!(!modules.iter().any(|m| m == "statistics"));

        let catalog = json_of(&ListActivities {}.call_tool(Some(&contract)).unwrap());
        assert_eq!(
            activity(&catalog, "run_code").unwrap()["available_capabilities"],
            serde_json::json!(["fail", "json_path", "log", "now"])
        );
        for cap in contract["capabilities"].as_array().unwrap() {
            let name = cap["name"].as_str().unwrap();
            assert!(
                cap["stub"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("def {name}("))
            );
        }
    }
}
