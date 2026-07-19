# US-MCP-1: Discover Available Workflows

**Epic:** MCP Server for AI Agent Integration
**Category:** Discovery
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent (Claude Code, Claude Desktop, custom agent)
**I want to** discover what workflow definitions are available in Kruxia Flow
**So that** I can choose the appropriate workflow to execute for a user's request

---

## Acceptance Criteria

### AC1: List All Workflow Definitions
- ✅ Agent can call `list_workflow_definitions()` to get all available workflows
- ✅ Returns workflow name, description, and namespace
- ✅ Supports pagination (limit, offset)
- ✅ Supports namespace filtering

### AC2: Get Workflow Details
- ✅ Agent can call `get_workflow_definition(name)` to get complete workflow structure
- ✅ Returns all activities with their configurations
- ✅ Returns dependency graph information
- ✅ Returns parameter schemas
- ✅ Returns settings (retry, budget, timeout)

### AC3: Search and Filter
- ✅ Can filter by namespace (e.g., "production", "staging")
- ✅ Pagination works correctly for large result sets

---

## Implementation Details

### MCP Tool: `list_workflow_definitions`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/discovery.py:18-65`

**Signature:**
```python
async def list_workflow_definitions(
    namespace: str | None = None,
    limit: int = 20,
    offset: int = 0,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns:**
```json
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
```

**API Endpoint Used:** `GET /api/v1/workflow_definitions`

---

### MCP Tool: `get_workflow_definition`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/discovery.py:67-109`

**Signature:**
```python
async def get_workflow_definition(
    name: str,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns:**
```json
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
```

**API Endpoint Used:** `GET /api/v1/workflow_definitions/:name`

---

## Usage Examples

### Example 1: Browse Available Workflows
```python
# Agent discovers what workflows exist
definitions = await list_workflow_definitions()

# Show user
print("Available workflows:")
for defn in definitions["definitions"]:
    print(f"  - {defn['name']}: {defn['description']}")
```

### Example 2: Get Workflow Details Before Execution
```python
# User asks: "Run the research assistant workflow"

# Agent first checks what it does
workflow = await get_workflow_definition("research_assistant")
print(f"This workflow has {len(workflow['activities'])} activities")
print(f"Description: {workflow['description']}")

# Then submits it
result = await submit_workflow("research_assistant", input_data)
```

### Example 3: Filter by Namespace
```python
# Only show production-ready workflows
prod_workflows = await list_workflow_definitions(namespace="production")
```

---

## Testing

**Test File:** `tests/tools/test_discovery.py`

**Coverage:**
- ✅ Client method tests for `get_workflow_definitions()`
- ✅ Client method tests for `get_workflow_definition()`
- ✅ Namespace filtering
- ✅ Pagination

**Test Results:** All tests passing (2/2)

---

## Related User Stories

- **US-MCP-2:** Explore Available Activity Types
- **US-MCP-3:** View Workflow Details (partial overlap)
- **US-MCP-4:** Validate Workflows Before Execution

---

## Documentation

- **User Guide:** `docs/implementation/mcp-userguide.md` - Section "How does an agent discover what workflows are available?"
- **Development Plan:** `docs/implementation/mcp-server-development-plan.md` - Task MCP-3
- **PRD:** `docs/implementation/mcp-server-prd.md` - Discovery Tools section

---

## Notes

- Agent should cache workflow definitions to reduce API calls
- Workflow definitions are relatively static (don't change frequently)
- Agent can proactively list workflows when user asks general questions like "What can you do with workflows?"
