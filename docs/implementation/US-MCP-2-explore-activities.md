# US-MCP-2: Explore Available Activity Types

**Epic:** MCP Server for AI Agent Integration
**Category:** Discovery
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** discover what activity types are available for building workflows
**So that** I can create new workflows using the correct activity types and parameters

---

## Acceptance Criteria

### AC1: List All Activity Types
- ✅ Agent can call `list_activities()` to get all built-in activity types
- ✅ Returns 7 built-in activities: http_request, llm_prompt, postgres_query, postgres_transaction, embedding, email_send, script
- ✅ Each activity includes description and parameter schema

### AC2: Understand Activity Parameters
- ✅ For each activity, returns required and optional parameters
- ✅ Includes parameter descriptions and types
- ✅ Shows what outputs each activity produces

### AC3: Worker Information
- ✅ Shows which worker executes each activity (builtin, py-std, py-data, py-ml, py-nlp)
- ✅ For script activities, lists available worker variants

---

## Implementation Details

### MCP Tool: `list_activities`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/discovery.py:111-141`

**Signature:**
```python
async def list_activities(
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns:**
```json
{
  "activities": [
    {
      "name": "http_request",
      "description": "Make HTTP/REST API requests with configurable retries",
      "worker": "builtin",
      "parameters": {
        "method": "HTTP method (GET, POST, PUT, DELETE, etc.)",
        "url": "Target URL (supports template expressions)",
        "headers": "Optional request headers",
        "body": "Optional request body",
        "query": "Optional query parameters"
      },
      "outputs": ["response", "status_code", "headers"],
      "settings": {
        "retry": "Configurable retry policy",
        "timeout": "Activity-level timeout"
      }
    },
    {
      "name": "llm_prompt",
      "description": "Call LLM APIs with multi-model fallback and budget controls",
      "worker": "builtin",
      "parameters": {
        "model": "Model name or array for fallback",
        "prompt": "User prompt text",
        "system": "Optional system prompt",
        "max_tokens": "Maximum tokens to generate",
        "temperature": "Sampling temperature 0-1"
      },
      "outputs": ["result", "cost_usd", "provider", "model", "usage"]
    }
  ],
  "total": 7,
  "note": "All activities support template expressions like {{INPUT.field}}"
}
```

---

## Activity Types Reference

### 1. `http_request`
- **Purpose:** Make HTTP/REST API calls
- **Worker:** builtin
- **Common Use Cases:** Fetch data, webhook notifications, API integrations
- **Key Parameters:** method, url, headers, body, query
- **Outputs:** response, status_code, headers

### 2. `llm_prompt`
- **Purpose:** Call LLM APIs (Claude, OpenAI, Google)
- **Worker:** builtin
- **Common Use Cases:** Text generation, analysis, summarization
- **Key Parameters:** model, prompt, system, max_tokens, temperature
- **Outputs:** result, cost_usd, provider, model, usage
- **Special Features:** Multi-model fallback, budget controls

### 3. `postgres_query`
- **Purpose:** Execute PostgreSQL queries
- **Worker:** builtin
- **Common Use Cases:** Data storage, retrieval, updates
- **Key Parameters:** query, params, database_url
- **Outputs:** rows, row_count

### 4. `postgres_transaction`
- **Purpose:** Execute multiple queries atomically
- **Worker:** builtin
- **Common Use Cases:** Complex database operations requiring ACID guarantees
- **Key Parameters:** queries, database_url
- **Outputs:** results, row_counts

### 5. `embedding`
- **Purpose:** Generate text embeddings
- **Worker:** builtin
- **Common Use Cases:** RAG indexing, semantic search, similarity
- **Key Parameters:** model, input, dimensions
- **Outputs:** embeddings, dimensions, cost_usd

### 6. `email_send`
- **Purpose:** Send emails via SMTP
- **Worker:** builtin
- **Common Use Cases:** Notifications, reports, alerts
- **Key Parameters:** to, from, subject, body, html
- **Outputs:** message_id, status

### 7. `script`
- **Purpose:** Execute Python scripts
- **Worker:** py-std, py-data, py-ml, py-nlp
- **Common Use Cases:** Custom logic, data transformations, ML inference
- **Key Parameters:** code, globals, timeout
- **Outputs:** result, stdout, stderr
- **Worker Variants:**
  - **py-std:** Universal utilities (httpx, orjson, pydantic)
  - **py-data:** ETL/transformation (pandas, polars, duckdb)
  - **py-ml:** Training/inference (sklearn, torch, numpy)
  - **py-nlp:** Text processing (transformers, spacy)

---

## Usage Examples

### Example 1: Show Available Activities to User
```python
# User asks: "What can workflows do?"

activities = await list_activities()

print("Kruxia Flow workflows can use these activities:")
for activity in activities["activities"]:
    print(f"\n{activity['name']}:")
    print(f"  {activity['description']}")
    print(f"  Worker: {activity['worker']}")
```

### Example 2: Agent Creates New Workflow
```python
# User: "Create a workflow that fetches weather and posts to webhook"

# Agent first checks available activities
activities = await list_activities()
activity_names = {a["name"] for a in activities["activities"]}

# Confirms http_request is available
if "http_request" in activity_names:
    # Create workflow using http_request activities
    workflow_yaml = """
    name: weather_webhook
    activities:
      - key: fetch_weather
        activity_name: http_request
        parameters:
          method: GET
          url: "https://api.weather.gov/..."

      - key: post_webhook
        activity_name: http_request
        parameters:
          method: POST
          url: "{{INPUT.webhook_url}}"
          body: "{{fetch_weather.response}}"
        depends_on: [fetch_weather]
    """
```

### Example 3: Choose Right Worker for Script Activity
```python
# User: "Run this pandas data transformation script"

# Agent checks which worker to use
activities = await list_activities()
script_activity = next(a for a in activities["activities"] if a["name"] == "script")

# Sees py-data worker is for pandas
print("Workers available for script activity:")
for worker, desc in script_activity.get("workers", {}).items():
    print(f"  {worker}: {desc}")

# Uses py-data worker
workflow_yaml = """
activities:
  - key: transform_data
    activity_name: script
    worker: py-data  # Has pandas installed
    parameters:
      code: |
        import pandas as pd
        df = pd.DataFrame(...)
        result = df.groupby(...).sum()
"""
```

---

## Testing

**Test File:** `tests/tools/test_discovery.py`

**Coverage:**
- ✅ All 7 activity types returned
- ✅ Each activity has proper structure (name, description, parameters, outputs)
- ✅ http_request activity has correct parameters

**Test Results:** All tests passing

---

## Related User Stories

- **US-MCP-1:** Discover Available Workflows
- **US-MCP-4:** Validate Workflows Before Execution (uses activity types)
- **US-MCP-5:** Submit Workflows (uses activity types)

---

## Documentation

- **User Guide:** `docs/implementation/mcp-userguide.md` - Activity types reference in appendix
- **Development Plan:** `docs/implementation/mcp-server-development-plan.md` - Task MCP-3
- **PRD:** `docs/implementation/mcp-server-prd.md` - Discovery Tools section

---

## Notes

- Activity list is static (built into the MCP server)
- This is intentional - activities are core to Kruxia Flow, not dynamically loaded
- Future: Could add endpoint to Kruxia Flow API to return available activities dynamically
- Agent should cache this list - it never changes during a session
