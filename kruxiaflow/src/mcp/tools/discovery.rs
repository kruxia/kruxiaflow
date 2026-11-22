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
                "description": "Returns its parameters unchanged, as the output named echo. Useful for testing, debugging, and passing values along.",
                "parameters": {
                    "<any>": "Any parameters; all are returned as the echo output (e.g. {{step.echo.message}})"
                },
                "outputs": ["echo"],
                "settings": {
                    "timeout_seconds": "Activity-level timeout in seconds"
                }
            },
            {
                "name": "http_request",
                "worker": "std",
                "description": "Make HTTP/REST API requests with configurable method, headers, query, body, and timeout.",
                "parameters": {
                    "method": "HTTP method: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS",
                    "url": "Target URL (supports template expressions)",
                    "headers": "Optional HTTP headers: a map of string values",
                    "query": "Optional query parameters: a map of string values (quote numbers, e.g. page: \"2\")",
                    "body": "Optional request body, always sent as JSON (a string is sent as a JSON string)",
                    "timeout_seconds": "Request timeout in seconds (default: 30)",
                    "include_body": "Include the response body in the output (default: true)",
                    "download_to_file": "Write the response body to a file with this name instead of returning it. It is kept as a workflow file only if the step declares it in outputs (e.g. outputs: [{name: report, type: file}]); otherwise it is written in the worker's working directory"
                },
                "outputs": ["response"],
                "output_shape": "response: {status (HTTP status code), success (true for 2xx), body (parsed JSON, or text)}. Reference as {{step.response.body}} or {{step.response.status}}",
                "settings": {
                    "retry": "Retry policy (max_attempts, strategy, base_seconds, factor, max_seconds)",
                    "timeout_seconds": "Activity-level timeout in seconds"
                }
            },
            {
                "name": "llm_prompt",
                "worker": "std",
                "description": "Call LLM APIs with multi-provider support, fallback chains, and budget controls. Supports Anthropic, OpenAI, Google, and Ollama.",
                "parameters": {
                    "model": "Model identifier, or an array for budget-aware fallback (e.g. \"anthropic/claude-sonnet-5\"). Call search_llm_models for current identifiers",
                    "prompt": "User prompt text (supports template expressions)",
                    "system_prompt": "Optional system prompt",
                    "max_tokens": "Maximum tokens to generate",
                    "temperature": "Sampling temperature",
                    "top_p": "Nucleus sampling",
                    "stop_sequences": "Optional array of stop sequences"
                },
                "outputs": ["result"],
                "output_shape": "result: {content (the text), model, provider, finish_reason, cost_usd, usage {prompt_tokens, output_tokens, total_tokens, cached_tokens}, budget_skipped_models}. Reference the text as {{step.result.content}}",
                "settings": {
                    "retry": "Retry with backoff",
                    "budget": "Budget limit in USD: {limit, action: abort or continue}"
                },
                "streaming": "Token streaming over WebSocket: set streaming: true on the activity itself, not under settings (a settings value is ignored)"
            },
            {
                "name": "embedding",
                "worker": "std",
                "description": "Generate text embeddings with provider fallback chains.",
                "parameters": {
                    "model": "Embedding model (e.g. \"openai/text-embedding-3-small\"), or an array for fallback",
                    "input": "Array of texts to embed",
                    "batch_size": "Texts per provider request (default: 500)"
                },
                "outputs": ["result"],
                "output_shape": "result: {embeddings_file (a workflow file of the vectors, JSON Lines), embedding_count, model, provider, usage}. On a server the vectors are always written to that file and embeddings is null; embeddings holds the vectors inline only when no workflow storage is available",
                "settings": {
                    "retry": "Retry with backoff",
                    "budget": "Budget limit for embedding costs"
                }
            },
            {
                "name": "postgres_query",
                "worker": "std",
                "description": "Execute a PostgreSQL query (SELECT, INSERT, UPDATE, DELETE) with parameterized placeholders.",
                "parameters": {
                    "db_url": "(required) Connection string; use a secret, e.g. \"{{SECRET.db_url}}\"",
                    "query": "SQL with $1, $2, … placeholders",
                    "params": "Array of values bound to the placeholders"
                },
                "outputs": ["result"],
                "output_shape": "result: {rows} (an array of row objects) for SELECT, WITH, and INSERT/UPDATE/DELETE … RETURNING; otherwise {rows_affected}; never both. Reference as {{step.result.rows[0].column}}",
                "settings": {
                    "retry": "Retry for transient errors",
                    "timeout_seconds": "Query timeout in seconds"
                }
            },
            {
                "name": "postgres_transaction",
                "worker": "std",
                "description": "Execute multiple SQL statements inside a single ACID transaction.",
                "parameters": {
                    "db_url": "(required) Connection string; use a secret",
                    "statements": "Array of {query, params} run in order, atomically",
                    "isolation_level": "read_committed (default), repeatable_read, or serializable"
                },
                "outputs": ["result"],
                "output_shape": "result: {success, statements_executed, results (one per statement: {rows} for SELECT, WITH and … RETURNING, {rows_affected} otherwise)}",
                "settings": {
                    "retry": "Retry on serialization failures"
                }
            },
            {
                "name": "email_send",
                "worker": "std",
                "description": "Send an email via SMTP with a plain-text and/or HTML body.",
                "parameters": {
                    "smtp_url": "(required) SMTP URL, e.g. \"{{SECRET.smtp_url}}\" (smtp:// or smtps://, with credentials)",
                    "from": "Sender address",
                    "to": "Array of recipient addresses",
                    "subject": "Email subject",
                    "text_body": "Plain-text body",
                    "html_body": "HTML body",
                    "cc": "Optional array of CC addresses",
                    "bcc": "Optional array of BCC addresses",
                    "reply_to": "Optional reply-to address"
                },
                "outputs": ["result"],
                "output_shape": "result: {success, message_id, recipients_accepted, recipients_rejected}",
                "settings": {
                    "retry": "Retry on SMTP failures"
                }
            },
            {
                "name": "script",
                "worker": "py-std",
                "description": "Execute a Python script on a Python worker. The worker type decides which packages are installed.",
                "parameters": {
                    "script": "Python source code to execute",
                    "inputs": "Optional object, available to the script as INPUT"
                },
                "outputs": ["result"],
                "output_shape": "result: the OUTPUT dict the script assigns (e.g. OUTPUT = {\"rows\": 3}). Reference as {{step.result.rows}}",
                "settings": {
                    "retry": "Retry for transient failures",
                    "timeout_seconds": "Activity-level timeout in seconds"
                },
                "worker_pools": {
                    "py-std": "pandas, polars, pyarrow, duckdb, numpy, scikit-learn, torch (CPU), sentence-transformers, transformers, tiktoken, orjson, python-dateutil",
                    "py-sys": "The SDK only, with curl and jq"
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
                    "worker": "(optional, default: std) Worker type that executes this activity — std, py-std, py-sys (or a custom worker type)",
                    "activity_name": "(required) Name of the activity within the worker (e.g. http_request, llm_prompt)",
                    "parameters": "Key-value map passed to the activity at runtime. Supports template expressions.",
                    "depends_on": "List of activity keys that must complete before this one starts. Each entry can be a plain key or {activity_key, conditions}.",
                    "dependency_of": "Inverse of depends_on — declares that other activities depend on this one. Normalised to depends_on internally.",
                    "settings": "Per-activity settings: retry, timeout_seconds, budget, cache, delay / scheduled_for, wait_for_signal, iteration_limit (see settings_configuration). Unknown keys are silently ignored, so spell them exactly",
                    "outputs": "Optional: declare the activity's outputs, e.g. [{name: report, type: file}] to keep a file it writes as a workflow file",
                    "iteration_scoped": "If true, outputs are stored per-loop iteration (default: false)",
                    "iteration_limit": "Max loop iterations (also accepted under settings). At the limit the loop stops: the activity is not run again, and nothing fails",
                    "streaming": "Enable token-level streaming for LLM activities (here, on the activity, not under settings)"
                },
                "example": "name: fetch-and-summarise\nactivities:\n  - key: fetch\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: GET\n      url: \"{{INPUT.url}}\"\n  - key: summarise\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: \"Summarise this text: {{fetch.response.body}}\"\n    depends_on: [fetch]"
            },
            "template_expressions": {
                "description": "Dynamic values resolved at runtime using {{source.path}} syntax",
                "syntax": "{{SOURCE.field.nested.path}} — dot-separated access into JSON. Array indexing with [N] is supported.",
                "sources": {
                    "INPUT": "The input payload provided when the workflow is submitted (e.g. {{INPUT.user_id}})",
                    "<activity_key>": "Output of a completed activity: {{<key>.<output_name>.<path>}}, e.g. {{fetch.response.body}} (http_request), {{summarise.result.content}} (llm_prompt), {{query.result.rows}} (postgres_query). See each activity's outputs and output_shape in list_activities. The activity must be in depends_on (directly or transitively), and {{<key>.status}} is always available",
                    "SECRET": "Secrets injected at runtime (e.g. {{SECRET.api_key}}). Not stored in the definition.",
                    "WORKFLOW": "Workflow-level metadata: {{WORKFLOW.id}} (the workflow's UUID) and {{WORKFLOW.status}}"
                },
                "examples": [
                    "{{INPUT.query}} — value of 'query' key from workflow input",
                    "{{fetch_data.result.rows[0].name}} — first row's name field from a postgres_query step named fetch_data",
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
                    "example": "activities:\n  - { key: fetch_a, worker: std, activity_name: http_request, parameters: {method: GET, url: \"{{INPUT.url_a}}\"} }\n  - { key: fetch_b, worker: std, activity_name: http_request, parameters: {method: GET, url: \"{{INPUT.url_b}}\"} }\n  - key: combine\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: \"Combine: {{fetch_a.response.body}} and {{fetch_b.response.body}}\"\n    depends_on: [fetch_a, fetch_b]"
                },
                "conditional": {
                    "description": "An activity only runs if a condition on an upstream output is met. Conditions must be {{ }} templates: a bare expression is a non-empty string, which is always true.",
                    "example": "activities:\n  - key: check\n    worker: std\n    activity_name: http_request\n    parameters: {method: GET, url: \"{{INPUT.status_url}}\"}\n  - key: on_success\n    worker: std\n    activity_name: llm_prompt\n    parameters: {model: anthropic/claude-sonnet-5, prompt: \"Process success\"}\n    depends_on:\n      - activity_key: check\n        conditions: [\"{{ check.response.status == 200 }}\"]"
                }
            },
            "settings_configuration": {
                "retry_policy": {
                    "description": "How to retry a failed activity",
                    "fields": {
                        "max_attempts": "Total attempts including the first try (default: 1 = no retry)",
                        "strategy": "exponential (default) or fixed",
                        "base_seconds": "Base delay in seconds (default: 2)",
                        "factor": "Multiplier per attempt for exponential backoff (default: 2.0)",
                        "max_seconds": "Maximum delay cap in seconds (default: 300)"
                    },
                    "example": "settings:\n  retry:\n    max_attempts: 3\n    strategy: exponential\n    base_seconds: 2\n    max_seconds: 60"
                },
                "timeout": {
                    "description": "Maximum wall-clock time for an activity (the key is timeout_seconds)",
                    "example": "settings:\n  timeout_seconds: 300  # 5 minutes"
                },
                "budget": {
                    "description": "Per-activity cost cap (useful for LLM activities)",
                    "fields": {
                        "limit": "Maximum spend in USD (field name is 'limit', not 'limit_usd')",
                        "action": "abort (default: stop when the limit is reached) or continue (log a warning and go on)"
                    },
                    "example": "settings:\n  budget:\n    limit: 0.50\n    action: abort"
                },
                "scheduling": {
                    "description": "Delay activity start after its dependencies complete (delay: a duration such as 60s, 5m, 2h), or start at a time (scheduled_for: an ISO 8601 timestamp). Both accept templates",
                    "example": "settings:\n  delay: 60s  # Wait 60 s after dependencies finish"
                },
                "signals": {
                    "description": "Pause the activity until an external signal is received (send it with send_workflow_signal). event_name and timeout_seconds are required; on_timeout is fail (default), continue or skip. Each wait takes one signal. In a loop the activity waits again in every round, for that round's signal",
                    "example": "settings:\n  wait_for_signal:\n    event_name: approval\n    timeout_seconds: 3600\n    on_timeout: fail  # Fail after 1 hour if no signal",
                    "hold_early": {
                        "description": "A signal sent while the activity is not waiting (too early, or while a loop is busy with the previous round) is refused, with reason not_waiting. With hold_early: true it is held instead (up to 100 per event name) and delivered, oldest first, when the activity next waits: the activity then runs at once with that signal's data. Signals still held when the workflow ends are discarded",
                        "example": "settings:\n  wait_for_signal:\n    event_name: message\n    timeout_seconds: 3600\n    hold_early: true  # Keep signals that arrive between waits"
                    }
                }
            },
            "complete_examples": {
                "simple_sequential": {
                    "description": "Fetch a URL, then summarise the response with an LLM",
                    "yaml": "name: fetch-and-summarise\nactivities:\n  - key: fetch\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: GET\n      url: \"{{INPUT.url}}\"\n    settings:\n      timeout_seconds: 30\n      retry:\n        max_attempts: 3\n  - key: summarise\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      system_prompt: You are a concise summariser.\n      prompt: \"Summarise: {{fetch.response.body}}\"\n      max_tokens: 256\n    depends_on: [fetch]\n    settings:\n      budget:\n        limit: 0.10\n        action: abort"
                },
                "parallel_fan_in": {
                    "description": "Query two databases in parallel, then merge results",
                    "yaml": "name: parallel-merge\nactivities:\n  - key: query_users\n    worker: std\n    activity_name: postgres_query\n    parameters:\n      db_url: \"{{SECRET.db_url}}\"\n      query: \"SELECT * FROM users WHERE active = $1\"\n      params: [true]\n  - key: query_orders\n    worker: std\n    activity_name: postgres_query\n    parameters:\n      db_url: \"{{SECRET.db_url}}\"\n      query: \"SELECT * FROM orders WHERE status = $1\"\n      params: [pending]\n  - key: merge\n    worker: std\n    activity_name: echo\n    parameters:\n      users: \"{{query_users.result.rows}}\"\n      orders: \"{{query_orders.result.rows}}\"\n    depends_on: [query_users, query_orders]"
                },
                "conditional_with_signal": {
                    "description": "Classify input: low-value requests are processed directly; high-value requests first wait for an approval signal (and fail after 24 hours without one)",
                    "yaml": "name: approval-flow\nactivities:\n  - key: classify\n    worker: std\n    activity_name: llm_prompt\n    parameters:\n      model: anthropic/claude-sonnet-5\n      prompt: \"Classify this request as high or low value. Answer with exactly one word, high or low: {{INPUT.request}}\"\n      max_tokens: 16\n  - key: await_approval\n    worker: std\n    activity_name: echo\n    parameters:\n      message: Waiting for approval\n    depends_on:\n      - activity_key: classify\n        conditions: [\"{{ classify.result.content | trim | lower == 'high' }}\"]\n    settings:\n      wait_for_signal:\n        event_name: approval\n        timeout_seconds: 86400\n        on_timeout: fail\n  - key: process_low\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: POST\n      url: \"{{INPUT.callback_url}}\"\n      body: {value: low}\n    depends_on:\n      - activity_key: classify\n        conditions: [\"{{ classify.result.content | trim | lower != 'high' }}\"]\n  - key: process_high\n    worker: std\n    activity_name: http_request\n    parameters:\n      method: POST\n      url: \"{{INPUT.callback_url}}\"\n      body: {value: high, approved: true}\n    depends_on: [await_approval]"
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
        CheckSystemHealth
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

    // =========================================================================
    // Catalog and guide accuracy: agents copy these examples verbatim, and
    // settings silently ignore unknown keys, so a wrong key is a setting that
    // quietly does nothing
    // =========================================================================

    fn guide_json() -> serde_json::Value {
        let result = GetWorkflowAuthoringGuide {}.call_tool().unwrap();
        serde_json::from_str(&result.content[0].as_text_content().unwrap().text).unwrap()
    }

    /// Every "example" / "yaml" string in the guide, with where it is
    fn guide_examples() -> Vec<(String, String)> {
        fn walk(v: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
            match v {
                serde_json::Value::Object(map) => {
                    for (k, child) in map {
                        let here = format!("{path}.{k}");
                        if (k == "example" || k == "yaml") && child.is_string() {
                            out.push((here, child.as_str().unwrap().to_string()));
                        } else {
                            walk(child, &here, out);
                        }
                    }
                }
                serde_json::Value::Array(items) => {
                    for (i, child) in items.iter().enumerate() {
                        walk(child, &format!("{path}[{i}]"), out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&guide_json(), "guide", &mut out);
        out
    }

    /// Turn a fragment into a whole workflow definition
    fn as_workflow(example: &str) -> String {
        if example.starts_with("name:") {
            example.to_string()
        } else if example.starts_with("activities:") {
            format!("name: example\n{example}")
        } else if example.starts_with("settings:") {
            let indented: String = example.lines().map(|l| format!("    {l}\n")).collect();
            format!("name: example\nactivities:\n  - key: a\n    activity_name: echo\n{indented}")
        } else {
            panic!("unrecognised example shape: {example}")
        }
    }

    /// Keys present in `input` but missing after a parse/serialize round trip
    fn dropped_keys(
        input: &serde_json::Value,
        parsed: &serde_json::Value,
        path: &str,
        out: &mut Vec<String>,
    ) {
        if let (Some(i), Some(p)) = (input.as_object(), parsed.as_object()) {
            for (k, v) in i {
                match p.get(k) {
                    None => out.push(format!("{path}.{k}")),
                    Some(pv) => dropped_keys(v, pv, &format!("{path}.{k}"), out),
                }
            }
        }
    }

    #[test]
    fn test_every_guide_example_is_a_valid_workflow() {
        let mut problems = Vec::new();
        for (where_, example) in guide_examples() {
            let yaml = as_workflow(&example);
            let definition = match kruxiaflow_core::workflow::WorkflowDefinition::from_yaml(&yaml) {
                Ok(d) => d,
                Err(e) => {
                    problems.push(format!("{where_}: does not parse: {e:?}"));
                    continue;
                }
            };
            // settings keys the parser ignored
            let raw: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
            for (activity, parsed) in raw["activities"]
                .as_array()
                .unwrap()
                .iter()
                .zip(&definition.activities)
            {
                if let Some(settings) = activity.get("settings") {
                    let round_trip = serde_json::to_value(&parsed.settings).unwrap();
                    let mut dropped = Vec::new();
                    dropped_keys(settings, &round_trip, "settings", &mut dropped);
                    if !dropped.is_empty() {
                        problems.push(format!("{where_}: ignored settings keys {dropped:?}"));
                    }
                }
            }
            // conditions must be templates, or they are always true
            for activity in &definition.activities {
                for dep in activity.depends_on.iter().flatten() {
                    for condition in dep.conditions.iter().flatten() {
                        if !condition.contains("{{") {
                            problems.push(format!(
                                "{where_}: condition {condition:?} has no {{{{ }}}}, so it is always true"
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            problems.is_empty(),
            "guide examples:\n{}",
            problems.join("\n")
        );
    }

    #[test]
    fn test_guide_workflow_variables_exist() {
        // The orchestrator sets WORKFLOW.id and WORKFLOW.status only
        // (build_template_context); anything else is an undefined-value error
        let text = serde_json::to_string(&guide_json()).unwrap();
        for (i, _) in text.match_indices("WORKFLOW.") {
            let field: String = text[i + 9..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            assert!(
                ["id", "status"].contains(&field.as_str()),
                "guide mentions WORKFLOW.{field}, which does not exist"
            );
        }
    }

    /// Parameter and output names each activity actually accepts and returns:
    /// the parameter structs and ActivityResult names in worker/src/activities/
    /// (http.rs HttpRequestParams, llm.rs LLMPromptParams / EmbeddingParams,
    /// postgres.rs QueryParams / TransactionParams, email.rs EmailParams,
    /// echo.rs), and the Python worker's script activity. Update this when an
    /// activity changes.
    const ACTIVITY_TRUTH: &[(&str, &[&str], &[&str])] = &[
        (
            "http_request",
            &[
                "method",
                "url",
                "headers",
                "query",
                "body",
                "timeout_seconds",
                "include_body",
                "download_to_file",
            ],
            &["response"],
        ),
        (
            "llm_prompt",
            &[
                "model",
                "prompt",
                "system_prompt",
                "max_tokens",
                "temperature",
                "top_p",
                "stop_sequences",
            ],
            &["result"],
        ),
        ("embedding", &["model", "input", "batch_size"], &["result"]),
        (
            "postgres_query",
            &["db_url", "query", "params"],
            &["result"],
        ),
        (
            "postgres_transaction",
            &["db_url", "statements", "isolation_level"],
            &["result"],
        ),
        (
            "email_send",
            &[
                "smtp_url",
                "from",
                "to",
                "subject",
                "text_body",
                "html_body",
                "cc",
                "bcc",
                "reply_to",
            ],
            &["result"],
        ),
        ("echo", &[], &["echo"]),
        // py-std worker, kruxiaflow-python kruxiaflow/workers/script_activity.py
        ("script", &["script", "inputs"], &["result"]),
    ];

    /// The parameters each activity can't run without: the fields of its
    /// parameter struct that have no default (the script activity runs an
    /// empty script without `script`)
    const ACTIVITY_REQUIRED: &[(&str, &[&str])] = &[
        ("http_request", &["method", "url"]),
        ("llm_prompt", &["model", "prompt"]),
        ("embedding", &["model", "input"]),
        ("postgres_query", &["db_url", "query"]),
        ("postgres_transaction", &["db_url", "statements"]),
        ("email_send", &["smtp_url", "from", "to", "subject"]),
        ("script", &["script"]),
    ];

    /// The catalog must use the names in [`ACTIVITY_TRUTH`]
    #[test]
    fn test_catalog_matches_activity_parameters_and_outputs() {
        let truth = ACTIVITY_TRUTH;
        let result = ListActivities {}.call_tool().unwrap();
        let catalog: serde_json::Value =
            serde_json::from_str(&result.content[0].as_text_content().unwrap().text).unwrap();
        let mut problems = Vec::new();
        for (name, params, outputs) in truth {
            let entry = catalog["activities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["name"] == *name)
                .unwrap_or_else(|| panic!("{name} missing from catalog"));
            if *name != "echo" {
                let documented = entry["parameters"].as_object().unwrap();
                for key in documented.keys() {
                    if !params.contains(&key.as_str()) {
                        problems.push(format!("{name}: catalog parameter {key:?} is not accepted (accepted: {params:?})"));
                    }
                }
                for param in params.iter() {
                    if !documented.contains_key(*param) {
                        problems.push(format!(
                            "{name}: accepted parameter {param:?} is missing from the catalog"
                        ));
                    }
                }
            }
            let listed: Vec<&str> = entry["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o.as_str().unwrap())
                .collect();
            if listed != *outputs {
                problems.push(format!(
                    "{name}: catalog outputs {listed:?}, activity returns {outputs:?}"
                ));
            }
        }
        for entry in catalog["activities"].as_array().unwrap() {
            let name = entry["name"].as_str().unwrap();
            if !truth.iter().any(|(t, _, _)| *t == name) {
                problems.push(format!("{name}: in the catalog but not in ACTIVITY_TRUTH"));
            }
        }
        assert!(problems.is_empty(), "catalog:\n{}", problems.join("\n"));
    }

    /// Activity-level keys and parameters in the guide's examples must be
    /// real, and each example must give its activity's required parameters:
    /// unknown keys are silently ignored, and `streaming` under settings is
    /// overwritten by the activity-level field
    #[test]
    fn test_guide_examples_use_real_keys_and_parameters() {
        const ACTIVITY_KEYS: &[&str] = &[
            "key",
            "worker",
            "activity_name",
            "parameters",
            "outputs",
            "depends_on",
            "dependency_of",
            "settings",
            "iteration_scoped",
            "iteration_limit",
            "streaming",
        ];
        let mut problems = Vec::new();
        for (where_, example) in guide_examples() {
            let yaml = as_workflow(&example);
            let raw: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
            for activity in raw["activities"].as_array().unwrap() {
                let key = activity["key"].as_str().unwrap_or("?");
                for field in activity.as_object().unwrap().keys() {
                    if !ACTIVITY_KEYS.contains(&field.as_str()) {
                        problems.push(format!("{where_}: {key}: unknown activity field {field:?}"));
                    }
                }
                if activity["settings"].get("streaming").is_some() {
                    problems.push(format!(
                        "{where_}: {key}: streaming under settings is ignored; set it on the activity"
                    ));
                }
                let name = activity["activity_name"].as_str().unwrap_or_default();
                if let (Some((_, params, _)), Some(given)) = (
                    ACTIVITY_TRUTH.iter().find(|(t, _, _)| *t == name),
                    activity["parameters"].as_object(),
                ) && name != "echo"
                {
                    for param in given.keys() {
                        if !params.contains(&param.as_str()) {
                            problems.push(format!(
                                "{where_}: {key}: {name} does not accept parameter {param:?}"
                            ));
                        }
                    }
                }
                if let Some((_, required)) = ACTIVITY_REQUIRED.iter().find(|(t, _)| *t == name) {
                    for param in required.iter() {
                        if activity["parameters"].get(*param).is_none() {
                            problems
                                .push(format!("{where_}: {key}: {name} needs parameter {param:?}"));
                        }
                    }
                }
            }
        }
        // the activity_fields list names the YAML keys
        let fields = guide_json()["yaml_structure"]["activity_fields"].clone();
        assert!(fields.is_object(), "the guide lists activity_fields");
        for field in fields.as_object().into_iter().flat_map(|f| f.keys()) {
            if !ACTIVITY_KEYS.contains(&field.as_str()) {
                problems.push(format!(
                    "activity_fields: {field:?} is not an activity field"
                ));
            }
        }
        assert!(problems.is_empty(), "guide:\n{}", problems.join("\n"));
    }

    /// The approval example routes both kinds of request: low-value ones are
    /// processed directly, high-value ones wait for the approval step. Driven
    /// through the orchestrator's own readiness and skip evaluation.
    #[test]
    fn test_approval_example_routes_both_values() {
        use WorkflowActivityStatus::{Completed, NotScheduled, Skipped};
        use kruxiaflow_core::orchestrator::{
            ActivityState, WorkflowActivityStatus, WorkflowState, find_ready_activities,
            find_skipped_activities,
        };
        use kruxiaflow_core::workflow::{ActivityOutput, WorkflowDefinition};

        fn activity(
            key: &str,
            status: WorkflowActivityStatus,
            content: Option<&str>,
        ) -> (String, ActivityState) {
            (
                key.to_string(),
                ActivityState {
                    key: key.to_string(),
                    status,
                    outputs: content.map(|c| {
                        vec![ActivityOutput::value(
                            "result",
                            serde_json::json!({"content": c}),
                        )]
                    }),
                    error: None,
                    started_at: None,
                    completed_at: None,
                    attempt: 1,
                    last_error: None,
                    accumulated_cost_usd: rust_decimal::Decimal::ZERO,
                    iteration: 0,
                    iteration_outputs: None,
                    signal_data: None,
                    // only the step with output has completed
                    completion_seq: u64::from(content.is_some()),
                },
            )
        }
        fn state(activities: Vec<(String, ActivityState)>) -> WorkflowState {
            WorkflowState {
                workflow_id: uuid::Uuid::now_v7(),
                definition_name: "approval-flow".to_string(),
                status: kruxiaflow_core::events::WorkflowStatus::Running,
                activities: activities.into_iter().collect(),
                state_data: serde_json::json!({}),
                input: serde_json::json!({}),
                error_message: None,
            }
        }
        fn keys(v: &[&kruxiaflow_core::workflow::ActivityDefinition]) -> Vec<String> {
            let mut k: Vec<String> = v.iter().map(|a| a.key.clone()).collect();
            k.sort();
            k
        }

        let yaml = guide_json()["complete_examples"]["conditional_with_signal"]["yaml"]
            .as_str()
            .unwrap()
            .to_string();
        let definition = WorkflowDefinition::from_yaml(&yaml).unwrap();

        // low: processed directly; the approval branch is skipped
        let low = state(vec![
            activity("classify", Completed, Some("low")),
            activity("await_approval", NotScheduled, None),
            activity("process_low", NotScheduled, None),
            activity("process_high", NotScheduled, None),
        ]);
        assert_eq!(
            keys(&find_ready_activities(&definition, &low).unwrap()),
            ["process_low"]
        );
        assert_eq!(
            keys(&find_skipped_activities(&definition, &low).unwrap()),
            ["await_approval"]
        );
        let low_after_skip = state(vec![
            activity("classify", Completed, Some("low")),
            activity("await_approval", Skipped, None),
            activity("process_low", NotScheduled, None),
            activity("process_high", NotScheduled, None),
        ]);
        assert_eq!(
            keys(&find_skipped_activities(&definition, &low_after_skip).unwrap()),
            ["process_high"]
        );

        // high (as a model might answer): waits for approval first
        let high = state(vec![
            activity("classify", Completed, Some(" High\n")),
            activity("await_approval", NotScheduled, None),
            activity("process_low", NotScheduled, None),
            activity("process_high", NotScheduled, None),
        ]);
        assert_eq!(
            keys(&find_ready_activities(&definition, &high).unwrap()),
            ["await_approval"]
        );
        assert_eq!(
            keys(&find_skipped_activities(&definition, &high).unwrap()),
            ["process_low"]
        );
    }
}
