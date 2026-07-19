# US-MCP-5: Submit Workflows for Execution

**Epic:** MCP Server for AI Agent Integration
**Category:** Execution
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** submit workflow definitions for execution with input parameters and budget limits
**So that** I can orchestrate complex multi-step processes for users with cost protection

---

## Acceptance Criteria

### AC1: Submit Workflow
- ✅ Agent can submit workflow by definition name
- ✅ Can provide input parameters as dictionary
- ✅ Returns workflow_id for tracking
- ✅ Returns initial status (pending/running)

### AC2: Budget Protection
- ✅ Can specify optional budget_limit_usd
- ✅ Workflow aborts if budget exceeded
- ✅ Agent can warn user about costs before submission

### AC3: Error Handling
- ✅ Clear error messages if definition doesn't exist
- ✅ Clear error messages if input validation fails
- ✅ API errors are propagated to agent

---

## Implementation Details

### MCP Tool: `submit_workflow`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/execution.py:191-237`

**Signature:**
```python
async def submit_workflow(
    definition_name: str,
    input: dict[str, Any],
    budget_limit_usd: float | None = None,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Parameters:**
- `definition_name`: Name of workflow definition to execute (e.g., "weather_report")
- `input`: Input parameters matching workflow's expected inputs
- `budget_limit_usd`: Optional hard budget limit in USD

**Returns:**
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "status": "pending",
  "definition_name": "weather_report",
  "submitted_at": "2026-01-30T12:00:00Z"
}
```

**API Endpoint Used:** `POST /api/v1/workflows`

**Request Payload:**
```json
{
  "workflow_definition": "weather_report",
  "input": {
    "city": "San Francisco",
    "state": "CA"
  },
  "budget_limit_usd": 0.10
}
```

---

## Usage Examples

### Example 1: Simple Submission
```python
# User: "Get the weather for San Francisco"

# Agent submits workflow
result = await submit_workflow(
    definition_name="weather_report",
    input={
        "city": "San Francisco",
        "state": "CA"
    }
)

workflow_id = result["workflow_id"]
print(f"Started workflow {workflow_id}")

# Monitor progress
status = await get_workflow_status(workflow_id)
```

### Example 2: With Budget Limit
```python
# User: "Research Rust memory model, but don't spend more than $0.10"

# Agent checks estimated cost first
estimate = await estimate_workflow_cost(
    "research_assistant",
    {"topic": "Rust memory model"}
)

print(f"Estimated cost: ${estimate['estimated_cost_usd']:.3f}")

if estimate["estimated_cost_usd"] > 0.10:
    print("⚠️ Estimated cost exceeds budget. Switching to cheaper model...")
    # Modify workflow or ask user

# Submit with budget limit
result = await submit_workflow(
    definition_name="research_assistant",
    input={"topic": "Rust memory model"},
    budget_limit_usd=0.10  # Hard limit
)

# Check actual cost after completion
cost = await get_workflow_cost(result["workflow_id"])
print(f"Actual cost: ${cost['total_cost_usd']:.4f}")
```

### Example 3: Error Handling
```python
try:
    result = await submit_workflow(
        definition_name="nonexistent_workflow",
        input={}
    )
except httpx.HTTPStatusError as e:
    if e.response.status_code == 404:
        print("❌ Workflow definition not found")
        print("Available workflows:")
        defs = await list_workflow_definitions()
        for d in defs["definitions"]:
            print(f"  - {d['name']}")
    elif e.response.status_code == 400:
        print("❌ Invalid input parameters")
        # Get workflow definition to see expected inputs
        defn = await get_workflow_definition("workflow_name")
        print(f"Expected inputs: {defn.get('parameters')}")
```

### Example 4: Complete Workflow Lifecycle
```python
# 1. User asks to do something
# User: "Process this data and email me the results"

# 2. Agent finds appropriate workflow
workflows = await list_workflow_definitions()
data_processing = [w for w in workflows["definitions"]
                   if "data" in w["name"].lower()]

# 3. Validate before submission
workflow_yaml = "..."  # Draft or existing
validation = await validate_workflow(workflow_yaml)
if not validation["valid"]:
    # Fix errors
    pass

# 4. Estimate cost
estimate = await estimate_workflow_cost(
    "data_processing",
    {"dataset": sample_data}
)

# 5. Get user approval if cost is significant
if estimate["estimated_cost_usd"] > 0.50:
    # Show estimate to user
    pass

# 6. Submit workflow
wf = await submit_workflow(
    definition_name="data_processing",
    input={
        "dataset": user_data,
        "email": "user@example.com"
    },
    budget_limit_usd=1.0
)

# 7. Monitor and report
print(f"Processing started: {wf['workflow_id']}")

# 8. Show progress with diagram
diagram = await render_workflow_diagram(
    workflow_id=wf["workflow_id"],
    include_status=True
)

# 9. Wait for completion or continue conversation
status = await get_workflow_status(wf["workflow_id"])
```

---

## Budget Protection

### How It Works

1. **Pre-submission:** Agent estimates cost using `estimate_workflow_cost()`
2. **Submission:** Agent includes `budget_limit_usd` parameter
3. **Runtime:** Kruxia Flow tracks costs as activities execute
4. **Abort:** If total cost exceeds limit, workflow stops immediately
5. **Post-execution:** Agent retrieves actual cost with `get_workflow_cost()`

### Budget Exceeded Behavior

**Workflow Status:** `failed`
**Error Message:** `"Budget limit exceeded: spent $0.12 of $0.10 limit"`

**Activities:**
- Completed activities: Results are available
- Running activities: May complete (race condition)
- Pending activities: Not started

### Model Fallback Strategy

Workflows can specify model arrays for automatic fallback:
```yaml
activities:
  - key: analyze
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-opus-4    # Try first (expensive)
        - anthropic/claude-sonnet-4  # Fall back if budget tight
        - anthropic/claude-haiku     # Fall back if still too expensive
      prompt: "..."
```

Agent can also modify workflow before submission to use cheaper models.

---

## Testing

**Test Coverage:**
- ✅ Successful workflow submission
- ✅ Budget limit parameter passed correctly
- ✅ API endpoint called with correct payload
- ✅ Error handling (not found, invalid input)

**Integration Testing:**
- Requires running Kruxia Flow instance
- See: `docs/implementation/mcp-server-test-plan.md`

---

## Performance

- **Submission Time:** <100ms (just creates workflow record)
- **No Blocking:** Agent doesn't wait for workflow to complete
- **Async Execution:** Workflow runs independently

---

## Related User Stories

- **US-MCP-4:** Validate Workflows Before Execution
- **US-MCP-7:** Monitor Workflow Execution Status
- **US-MCP-8:** Track and Estimate Workflow Costs
- **US-MCP-9:** Retrieve Activity Outputs

---

## Documentation

- **User Guide:** `docs/implementation/mcp-userguide.md` - Multiple examples throughout
- **Development Plan:** `docs/implementation/mcp-server-development-plan.md` - Task MCP-4
- **PRD:** `docs/implementation/mcp-server-prd.md` - Execution Tools section

---

## Security Considerations

### Input Validation
- Kruxia Flow API validates input parameters against workflow schema
- MCP server doesn't validate input (trusts API)
- Agent should validate user input before submission

### Budget Enforcement
- Budget limit enforced server-side (can't be bypassed)
- Budget tracking includes all LLM and metered activities
- No way to exceed budget once set

### Authentication
- JWT token required (configured in MCP server settings)
- Token passed in Authorization header
- API rejects requests without valid token

---

## Future Enhancements

### Potential Additions
- **Scheduled Execution:** Submit workflow to run at specific time
- **Recurring Workflows:** Submit workflow to run on schedule
- **Workflow Templates:** Submit with template parameters
- **Dry Run Mode:** Submit for cost estimation without execution
