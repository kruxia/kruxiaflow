"""Discovery tools for listing and exploring workflow definitions and activity types."""

from typing import Any

from fastmcp import Context

from ..client import KruxiaFlowClient


def register_discovery_tools(mcp: Any, client: KruxiaFlowClient) -> None:
    """Register discovery tools with the MCP server.

    Args:
        mcp: FastMCP server instance
        client: Kruxia Flow API client
    """

    @mcp.tool()
    async def list_workflow_definitions(
        namespace: str | None = None,
        limit: int = 20,
        offset: int = 0,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """List available workflow definitions.

        Retrieves all workflow definitions that can be submitted for execution.
        Use this to discover what workflows are available before submitting them.

        Args:
            namespace: Optional namespace filter (e.g., "production", "staging")
            limit: Maximum number of definitions to return (default 20)
            offset: Number of definitions to skip for pagination (default 0)

        Returns:
            Dictionary containing:
            - definitions: List of workflow definitions with names and descriptions
            - total: Total count of definitions
            - limit: Requested limit
            - offset: Requested offset

        Example:
            {
                "definitions": [
                    {
                        "name": "weather_report",
                        "description": "Fetch weather forecast for a city",
                        "namespace": "examples"
                    }
                ],
                "total": 1,
                "limit": 20,
                "offset": 0
            }
        """
        return await client.get_workflow_definitions(
            namespace=namespace,
            limit=limit,
            offset=offset,
        )

    @mcp.tool()
    async def get_workflow_definition(
        name: str,
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Get detailed information about a specific workflow definition.

        Retrieves the complete definition including all activities, dependencies,
        parameters, and settings. Use this to understand how a workflow is
        structured before submitting it.

        Args:
            name: Workflow definition name (e.g., "weather_report")

        Returns:
            Dictionary containing the complete workflow definition:
            - name: Workflow name
            - description: Human-readable description
            - activities: List of activities with their configurations
            - parameters: Input parameters the workflow accepts
            - settings: Retry policies, timeouts, budget limits

        Example:
            {
                "name": "weather_report",
                "description": "Fetch weather forecast",
                "activities": [
                    {
                        "key": "fetch_weather",
                        "activity_name": "http_request",
                        "parameters": {"url": "https://api.weather.gov/..."}
                    }
                ]
            }
        """
        return await client.get_workflow_definition(name)

    @mcp.tool()
    async def get_workflow_authoring_guide(
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """Get comprehensive guide for creating workflow definitions.

        Returns detailed documentation on workflow YAML structure, template expressions,
        dependency patterns, and settings configuration. Use this when you need to CREATE
        a new workflow definition from scratch.

        This guide teaches:
        - Complete YAML structure with all fields
        - Template expression syntax ({{INPUT}}, {{activity.output}}, {{SECRET}}, {{WORKFLOW}})
        - Dependency patterns (sequential, parallel, conditional)
        - Settings configuration (retries, budgets, timeouts)
        - Complete working examples

        Returns:
            Dictionary containing comprehensive workflow authoring documentation

        When to Use This:
            Call this tool when you need to create a workflow definition. It provides
            the complete reference needed to author valid workflow YAML with all features.
        """
        return {
            "yaml_structure": {
                "description": "Complete workflow definition structure",
                "required_fields": {
                    "name": "Unique workflow identifier (lowercase, hyphens, underscores)",
                    "activities": "List of activities to execute",
                },
                "optional_fields": {
                    "description": "Human-readable workflow description",
                    "namespace": "Logical grouping (e.g., 'production', 'staging')",
                    "settings": "Workflow-level settings (budget, timeout, retry)",
                },
                "activity_structure": {
                    "required": {
                        "key": "Unique activity identifier within workflow",
                        "activity_name": "Activity type (http_request, llm_prompt, etc.)",
                        "parameters": "Activity-specific parameters (see list_activities)",
                    },
                    "optional": {
                        "worker": "Worker pool name (default: builtin)",
                        "outputs": "List of output fields to capture",
                        "depends_on": "List of dependencies (activity keys or objects with conditions)",
                        "settings": "Activity-level settings (retry, budget, timeout)",
                    },
                },
                "example": """name: my_workflow
description: Example workflow showing all features
activities:
  - key: activity1
    activity_name: http_request
    parameters:
      method: GET
      url: "{{INPUT.api_url}}"
    outputs:
      - response
    settings:
      retry:
        max_attempts: 3
        strategy: exponential

  - key: activity2
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4
      prompt: "Summarize: {{activity1.response}}"
    depends_on:
      - activity1
    settings:
      budget:
        limit_usd: 0.10
        action: abort
""",
            },
            "template_expressions": {
                "description": "Dynamic parameter values using template syntax",
                "syntax": "{{source.field}} or {{source.nested.path[0]}}",
                "sources": {
                    "INPUT": {
                        "description": "Workflow input parameters provided at submission",
                        "examples": [
                            "{{INPUT.user_id}}",
                            "{{INPUT.config.api_key}}",
                            "{{INPUT.items[0].name}}",
                        ],
                    },
                    "activity_key": {
                        "description": "Output from a previous activity (must be in depends_on)",
                        "examples": [
                            "{{fetch_data.response.json.results[0]}}",
                            "{{ask_llm.result.content}}",
                            "{{query_db.rows[0].id}}",
                        ],
                    },
                    "SECRET": {
                        "description": "Secret values from environment (never logged)",
                        "examples": [
                            "{{SECRET.api_key}}",
                            "{{SECRET.database_url}}",
                            "{{SECRET.smtp_password}}",
                        ],
                        "security_note": "Secrets are injected at runtime and never stored in workflow definitions",
                    },
                    "WORKFLOW": {
                        "description": "Workflow metadata",
                        "examples": [
                            "{{WORKFLOW.id}}",
                            "{{WORKFLOW.started_at}}",
                            "{{WORKFLOW.name}}",
                        ],
                    },
                },
                "example": """# Template expression usage example
- key: send_notification
  activity_name: http_request
  parameters:
    method: POST
    url: "{{INPUT.webhook_url}}"
    headers:
      Authorization: "Bearer {{SECRET.api_token}}"
    body:
      workflow_id: "{{WORKFLOW.id}}"
      user: "{{INPUT.user_name}}"
      result: "{{previous_activity.response.data}}"
      timestamp: "{{WORKFLOW.started_at}}"
""",
            },
            "dependency_patterns": {
                "description": "Control activity execution order and conditions",
                "simple_dependency": {
                    "description": "Activity runs after dependencies complete successfully",
                    "syntax": "depends_on: [activity_key1, activity_key2]",
                    "example": """- key: step2
  activity_name: http_request
  parameters: {...}
  depends_on:
    - step1  # Runs after step1 completes
""",
                },
                "conditional_dependency": {
                    "description": "Activity runs only if condition evaluates to true",
                    "syntax": "depends_on: [{activity_key: key, condition: '{{expression}}'}]",
                    "example": """- key: send_alert
  activity_name: email_send
  parameters: {...}
  depends_on:
    - activity_key: check_status
      condition: "{{check_status.response.error == true}}"  # Only runs if error detected
""",
                },
                "parallel_execution": {
                    "description": "Activities without dependencies run concurrently",
                    "example": """# These three activities run in parallel
- key: fetch_weather
  activity_name: http_request
  parameters: {...}

- key: fetch_news
  activity_name: http_request
  parameters: {...}

- key: fetch_stocks
  activity_name: http_request
  parameters: {...}

# This runs after all three complete
- key: generate_report
  activity_name: llm_prompt
  parameters: {...}
  depends_on:
    - fetch_weather
    - fetch_news
    - fetch_stocks
""",
                },
                "fan_out_fan_in": {
                    "description": "One activity feeds multiple, or multiple feed one",
                    "example": """# Fan-out: one activity feeds multiple
- key: fetch_users
  activity_name: postgres_query
  parameters: {...}

- key: notify_user1
  activity_name: email_send
  depends_on: [fetch_users]

- key: notify_user2
  activity_name: email_send
  depends_on: [fetch_users]

# Fan-in: multiple activities feed one
- key: final_report
  activity_name: llm_prompt
  depends_on:
    - notify_user1
    - notify_user2
""",
                },
            },
            "settings_configuration": {
                "description": "Configure retry policies, budgets, and timeouts",
                "retry_policy": {
                    "description": "Automatic retry on transient failures",
                    "fields": {
                        "max_attempts": "Total attempts including initial (default: 1)",
                        "strategy": "exponential or linear (default: exponential)",
                        "base_seconds": "Initial wait time (default: 1)",
                        "factor": "Multiplier for exponential backoff (default: 2)",
                        "max_seconds": "Maximum wait between retries (default: 60)",
                    },
                    "example": """settings:
  retry:
    max_attempts: 3
    strategy: exponential
    base_seconds: 2
    factor: 2
    max_seconds: 60
""",
                },
                "budget_limit": {
                    "description": "Hard USD limit for LLM and embedding costs",
                    "fields": {
                        "limit_usd": "Maximum cost in USD",
                        "action": "abort (fail workflow) or skip (skip activity)",
                    },
                    "example": """settings:
  budget:
    limit_usd: 0.50
    action: abort  # Fail workflow if cost exceeds $0.50
""",
                    "note": "Budget is estimated before execution. Activities that exceed budget are not executed.",
                },
                "timeout": {
                    "description": "Maximum execution time for activity",
                    "example": """settings:
  timeout: 300  # 5 minutes
""",
                },
            },
            "complete_examples": {
                "simple_sequential": {
                    "description": "Basic sequential workflow: A → B → C",
                    "yaml": """name: weather_report
activities:
  - key: fetch_weather
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.weather.gov/gridpoints/LOT/76,73/forecast"
    outputs:
      - response

  - key: send_notification
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.webhook_url}}"
      body:
        temperature: "{{fetch_weather.response.json.properties.periods[0].temperature}}"
        conditions: "{{fetch_weather.response.json.properties.periods[0].shortForecast}}"
    depends_on:
      - fetch_weather
""",
                },
                "parallel_with_fanin": {
                    "description": "Parallel activities feeding into final step",
                    "yaml": """name: research_assistant
activities:
  # These run in parallel
  - key: search_web
    activity_name: http_request
    parameters:
      method: GET
      url: "{{INPUT.search_api}}"
      query:
        q: "{{INPUT.question}}"

  - key: search_docs
    activity_name: postgres_query
    parameters:
      query: "SELECT * FROM docs WHERE content ILIKE $1"
      params:
        - "%{{INPUT.question}}%"

  # This waits for both to complete
  - key: synthesize_answer
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4
      prompt: |
        Based on web results: {{search_web.response}}
        And doc results: {{search_docs.rows}}
        Answer: {{INPUT.question}}
    depends_on:
      - search_web
      - search_docs
""",
                },
                "conditional_execution": {
                    "description": "Activities with conditional dependencies",
                    "yaml": """name: order_processing
activities:
  - key: check_inventory
    activity_name: http_request
    parameters:
      method: GET
      url: "{{INPUT.inventory_api}}/check"
      query:
        product_id: "{{INPUT.product_id}}"
    outputs:
      - response

  - key: process_order
    activity_name: postgres_transaction
    parameters:
      statements:
        - query: "INSERT INTO orders (product_id, quantity) VALUES ($1, $2)"
          params:
            - "{{INPUT.product_id}}"
            - "{{INPUT.quantity}}"
    depends_on:
      - activity_key: check_inventory
        condition: "{{check_inventory.response.available == true}}"

  - key: send_confirmation
    activity_name: email_send
    parameters:
      to: "{{INPUT.customer_email}}"
      subject: "Order Confirmed"
      body: "Your order has been processed"
    depends_on:
      - process_order
""",
                },
                "budget_controlled_llm": {
                    "description": "LLM workflow with budget limits and model fallback",
                    "yaml": """name: research_with_budget
activities:
  - key: ask_question
    activity_name: llm_prompt
    parameters:
      # Model fallback chain - tries each until success or budget skip
      model:
        - openai/o1-pro           # Expensive, may be skipped
        - anthropic/claude-sonnet-4
        - google/gemini-2.0-flash-lite
      prompt: "{{INPUT.question}}"
      max_tokens: 1000
    settings:
      retry:
        max_attempts: 3
        strategy: exponential
      budget:
        limit_usd: 0.01  # Tight budget
        action: abort    # Fail if exceeds

  - key: store_result
    activity_name: postgres_query
    parameters:
      query: |
        INSERT INTO research_log (question, answer, cost)
        VALUES ($1, $2, $3)
      params:
        - "{{INPUT.question}}"
        - "{{ask_question.result.content}}"
        - "{{ask_question.result.cost_usd}}"
    depends_on:
      - ask_question
""",
                },
            },
            "best_practices": {
                "workflow_design": [
                    "Keep workflows focused - one workflow per logical process",
                    "Use meaningful activity keys (fetch_user not a1, a2, a3)",
                    "Add description to explain workflow purpose",
                    "Use parallel execution where possible for better performance",
                ],
                "error_handling": [
                    "Configure retry policies for transient failures (network, rate limits)",
                    "Use conditional dependencies to handle expected failure cases",
                    "Set appropriate timeouts to prevent hanging workflows",
                    "Use budget limits to prevent runaway LLM costs",
                ],
                "security": [
                    "NEVER hardcode secrets in workflow definitions",
                    "Always use {{SECRET.key}} for sensitive values",
                    "Secrets are injected at runtime from environment variables",
                    "Use namespace to separate production from staging workflows",
                ],
                "template_expressions": [
                    "Test template paths with example data before submitting",
                    "Use array indexing carefully - check bounds in conditions",
                    "Nested JSON access: {{activity.response.json.data[0].field}}",
                    "Reference activity outputs only if activity is in depends_on chain",
                ],
            },
            "next_steps": {
                "description": "How to use this guide effectively",
                "steps": [
                    "1. Review YAML structure to understand required and optional fields",
                    "2. Call list_activities() to see available activity types and parameters",
                    "3. Choose appropriate dependency pattern (sequential, parallel, conditional)",
                    "4. Use template expressions for dynamic values ({{INPUT}}, {{activity.output}})",
                    "5. Configure settings (retry, budget, timeout) based on requirements",
                    "6. Call validate_workflow() to check for errors before submission",
                    "7. Call submit_workflow() to execute the workflow",
                ],
            },
        }

    @mcp.tool()
    async def list_activities(
        ctx: Context | None = None,
    ) -> dict[str, Any]:
        """List available activity types for building workflows.

        Returns all built-in activity types that can be used in workflow definitions.
        Each activity has specific parameters and capabilities. Use this to understand
        what building blocks are available when creating workflows.

        For comprehensive workflow authoring documentation including YAML structure,
        template expressions, and patterns, call get_workflow_authoring_guide().

        Returns:
            Dictionary containing:
            - activities: List of activity types with descriptions and parameters

        Available Activity Types:
            - http_request: Make HTTP/REST API calls (GET, POST, PUT, DELETE, etc.)
            - llm_prompt: Call LLM APIs (Claude, OpenAI, Google, etc.) with budget controls
            - postgres_query: Execute PostgreSQL queries (SELECT, INSERT, UPDATE, DELETE)
            - postgres_transaction: Execute multiple queries in a single transaction
            - embedding: Generate embeddings using OpenAI or other providers
            - email_send: Send emails via SMTP
            - script: Execute Python scripts with pre-installed packages (py-std, py-data, py-ml, py-nlp)

        Example:
            {
                "activities": [
                    {
                        "name": "http_request",
                        "description": "Make HTTP API requests",
                        "parameters": {
                            "method": "HTTP method (GET, POST, PUT, DELETE, etc.)",
                            "url": "Target URL (supports template expressions)",
                            "headers": "Optional request headers",
                            "body": "Optional request body",
                            "query": "Optional query parameters"
                        }
                    }
                ]
            }
        """
        # Built-in activities are well-known and static
        # This list is maintained in sync with the Kruxia Flow core
        return {
            "activities": [
                {
                    "name": "http_request",
                    "description": "Make HTTP/REST API requests with configurable retries",
                    "worker": "builtin",
                    "parameters": {
                        "method": "HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS)",
                        "url": "Target URL (supports {{INPUT.field}} and {{activity.output}} expressions)",
                        "headers": "Optional HTTP headers as key-value pairs",
                        "body": "Optional request body (JSON, text, or form data)",
                        "query": "Optional query parameters as key-value pairs",
                        "timeout": "Request timeout in seconds (default: 30)",
                    },
                    "outputs": ["response", "status_code", "headers"],
                    "settings": {
                        "retry": "Configurable retry policy (max_attempts, strategy, backoff)",
                        "timeout": "Activity-level timeout",
                    },
                },
                {
                    "name": "llm_prompt",
                    "description": "Call LLM APIs with multi-model fallback and budget controls",
                    "worker": "builtin",
                    "parameters": {
                        "model": "Model name or array of models for fallback (e.g., 'anthropic/claude-sonnet-4-5-20250929', ['openai/gpt-4', 'anthropic/claude-3-5-haiku-20241022'])",
                        "prompt": "User prompt text (supports template expressions)",
                        "system": "Optional system prompt",
                        "max_tokens": "Maximum tokens to generate (default: 1024)",
                        "temperature": "Sampling temperature 0-1 (default: 1.0)",
                        "tools": "Optional tool definitions for function calling",
                        "stream": "Enable streaming responses (default: false)",
                    },
                    "outputs": ["result", "cost_usd", "provider", "model", "usage"],
                    "settings": {
                        "retry": "Retry policy with exponential backoff",
                        "budget": "Budget limit in USD (limit_usd, action: abort|skip)",
                        "streaming": "Enable WebSocket streaming for incremental results",
                    },
                },
                {
                    "name": "postgres_query",
                    "description": "Execute PostgreSQL queries (SELECT, INSERT, UPDATE, DELETE)",
                    "worker": "builtin",
                    "parameters": {
                        "query": "SQL query with $1, $2, ... placeholders",
                        "params": "Array of parameter values to bind to placeholders",
                        "database_url": "Optional PostgreSQL connection string (defaults to KRUXIAFLOW_DATABASE_URL)",
                    },
                    "outputs": ["rows", "row_count"],
                    "settings": {
                        "retry": "Retry policy for transient database errors",
                        "timeout": "Query timeout in seconds",
                    },
                },
                {
                    "name": "postgres_transaction",
                    "description": "Execute multiple PostgreSQL queries in a single ACID transaction",
                    "worker": "builtin",
                    "parameters": {
                        "queries": "Array of SQL queries to execute atomically",
                        "database_url": "Optional PostgreSQL connection string",
                    },
                    "outputs": ["results", "row_counts"],
                    "settings": {
                        "retry": "Retry policy for serialization failures",
                        "isolation": "Transaction isolation level (default: READ COMMITTED)",
                    },
                },
                {
                    "name": "embedding",
                    "description": "Generate embeddings using OpenAI or other embedding providers",
                    "worker": "builtin",
                    "parameters": {
                        "model": "Embedding model (e.g., 'openai/text-embedding-3-small')",
                        "input": "Text or array of texts to embed",
                        "dimensions": "Optional output dimensions (for models that support it)",
                    },
                    "outputs": ["embeddings", "dimensions", "cost_usd"],
                    "settings": {
                        "retry": "Retry policy with exponential backoff",
                        "budget": "Budget limit for embedding costs",
                    },
                },
                {
                    "name": "email_send",
                    "description": "Send emails via SMTP",
                    "worker": "builtin",
                    "parameters": {
                        "to": "Recipient email address or array of addresses",
                        "from": "Sender email address",
                        "subject": "Email subject line",
                        "body": "Email body (plain text or HTML)",
                        "html": "Whether body is HTML (default: false)",
                        "cc": "Optional CC addresses",
                        "bcc": "Optional BCC addresses",
                        "attachments": "Optional file attachments",
                    },
                    "outputs": ["message_id", "status"],
                    "settings": {
                        "retry": "Retry policy for SMTP failures",
                        "smtp": "SMTP server configuration (host, port, auth)",
                    },
                },
                {
                    "name": "script",
                    "description": "Execute Python scripts with pre-installed packages",
                    "worker": "py-std",  # or py-data, py-ml, py-nlp
                    "parameters": {
                        "code": "Python code to execute",
                        "globals": "Optional global variables to inject",
                        "timeout": "Execution timeout in seconds (default: 300)",
                    },
                    "outputs": ["result", "stdout", "stderr"],
                    "settings": {
                        "retry": "Retry policy for transient failures",
                        "timeout": "Script execution timeout",
                    },
                    "workers": {
                        "py-std": "Universal utilities (httpx, orjson, pydantic, dateutil)",
                        "py-data": "ETL/transformation (pandas, polars, duckdb, sqlalchemy)",
                        "py-ml": "Training/inference (sklearn, torch, numpy, scipy)",
                        "py-nlp": "Text processing (transformers, spacy, tiktoken)",
                    },
                },
            ],
            "total": 7,
            "note": "All activities support template expressions like {{INPUT.field}} and {{activity.output.field}} for dynamic parameter values",
        }
