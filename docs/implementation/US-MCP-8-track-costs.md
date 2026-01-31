# US-MCP-8: Track and Estimate Workflow Costs

**Epic:** MCP Server for AI Agent Integration
**Category:** Observability
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** track workflow costs and estimate costs before execution
**So that** I can help users stay within budget and make cost-aware decisions

---

## Acceptance Criteria

### AC1: Get Workflow Cost
- ✅ Agent can retrieve cost breakdown for completed/running workflows
- ✅ Returns total cost in USD
- ✅ Breaks down cost by activity
- ✅ Breaks down cost by provider (Anthropic, OpenAI, etc.)
- ✅ Shows budget utilization if limit was set

### AC2: Estimate Cost Before Execution
- ✅ Agent can estimate cost before submitting workflow
- ✅ Returns estimated cost with min/max range
- ✅ Breaks down estimate by activity
- ✅ Lists assumptions made in estimation

### AC3: Model Pricing
- ✅ Includes pricing for major LLM providers
- ✅ Handles token-based pricing
- ✅ Accounts for input vs output token costs

---

## Implementation Details

### MCP Tool: `get_workflow_cost`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/observability.py:118-160`

**Signature:**
```python
async def get_workflow_cost(
    workflow_id: str,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns:**
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "total_cost_usd": 0.042,
  "budget_limit_usd": 0.10,
  "budget_used_percent": 42.0,
  "activities": [
    {
      "activity_key": "ask_question",
      "activity_name": "llm_prompt",
      "cost_usd": 0.0398,
      "provider": "anthropic",
      "model": "claude-sonnet-4-5-20250929",
      "tokens": {
        "prompt_tokens": 150,
        "output_tokens": 950,
        "total_tokens": 1100
      }
    },
    {
      "activity_key": "store_response",
      "activity_name": "postgres_query",
      "cost_usd": 0.0,
      "provider": "postgres"
    }
  ],
  "providers": {
    "anthropic": 0.0398,
    "postgres": 0.0
  }
}
```

**API Endpoint Used:** `GET /api/v1/workflows/:id/cost`

---

### MCP Tool: `estimate_workflow_cost`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/observability.py:162-302`

**Signature:**
```python
async def estimate_workflow_cost(
    definition_name: str,
    input_sample: dict[str, Any],
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns:**
```json
{
  "definition_name": "research_assistant",
  "estimated_cost_usd": 0.045,
  "cost_range_usd": {
    "min": 0.020,
    "max": 0.080
  },
  "activities": [
    {
      "activity_key": "ask_question",
      "activity_name": "llm_prompt",
      "estimated_cost_usd": 0.040,
      "cost_range_usd": {"min": 0.015, "max": 0.070}
    },
    {
      "activity_key": "store_response",
      "activity_name": "postgres_query",
      "estimated_cost_usd": 0.0,
      "cost_range_usd": {"min": 0.0, "max": 0.0}
    }
  ],
  "assumptions": [
    "Estimates based on typical token counts",
    "Does not account for retries or fallback models",
    "External API costs not included",
    "Assumes average-length responses"
  ],
  "note": "Actual costs may vary based on input data and response length"
}
```

---

## Model Pricing (Built-in)

### Anthropic Models (per million tokens)
| Model | Input | Output |
|-------|-------|--------|
| claude-opus-4 | $15 | $75 |
| claude-sonnet-4 | $3 | $15 |
| claude-3-5-haiku | $0.80 | $4 |

### OpenAI Models (per million tokens)
| Model | Input | Output |
|-------|-------|--------|
| gpt-4 | $10 | $30 |
| gpt-4-turbo | $5 | $15 |
| gpt-3.5-turbo | $0.50 | $1.50 |

### Other Activities
- **http_request:** $0 (free)
- **postgres_query:** $0 (free unless serverless DB)
- **embedding:** ~$0.02 per 1M tokens (OpenAI text-embedding-3-small)
- **email_send:** ~$0.0001 per email (varies by provider)

---

## Usage Examples

### Example 1: Check Cost After Completion
```python
# Workflow completed
status = await get_workflow_status(wf_id)
if status["status"] == "completed":
    # Get cost breakdown
    cost = await get_workflow_cost(wf_id)

    print(f"Total Cost: ${cost['total_cost_usd']:.4f}")
    print(f"\nCost Breakdown:")
    for activity in cost["activities"]:
        if activity["cost_usd"] > 0:
            print(f"  {activity['activity_key']}: ${activity['cost_usd']:.4f}")
            if "model" in activity:
                print(f"    Model: {activity['model']}")
                print(f"    Tokens: {activity['tokens']['total_tokens']}")
```

### Example 2: Estimate Before Submission
```python
# User: "Research Rust memory model"

# Estimate cost first
estimate = await estimate_workflow_cost(
    "research_assistant",
    {"topic": "Rust memory model"}
)

print(f"Estimated cost: ${estimate['estimated_cost_usd']:.3f}")
print(f"Cost range: ${estimate['cost_range_usd']['min']:.3f} - ${estimate['cost_range_usd']['max']:.3f}")

# Show per-activity breakdown
print("\nBreakdown:")
for activity in estimate["activities"]:
    if activity["estimated_cost_usd"] > 0:
        print(f"  {activity['activity_key']}: ${activity['estimated_cost_usd']:.3f}")

# Ask user for approval
print("\nDo you want to proceed? (estimated cost shown above)")
# User: "yes"

# Submit with budget limit (pad by 2x for safety)
await submit_workflow(
    "research_assistant",
    input={"topic": "Rust memory model"},
    budget_limit_usd=estimate["cost_range_usd"]["max"] * 2
)
```

### Example 3: Budget Monitoring
```python
# Submit with tight budget
wf = await submit_workflow(
    "expensive_workflow",
    input=data,
    budget_limit_usd=1.0
)

# Monitor cost in real-time
import asyncio
while True:
    status = await get_workflow_status(wf["workflow_id"])
    if status["status"] in ["completed", "failed", "canceled"]:
        break

    cost = await get_workflow_cost(wf["workflow_id"])

    print(f"Current cost: ${cost['total_cost_usd']:.4f}")
    print(f"Budget used: {cost['budget_used_percent']:.1f}%")

    if cost["budget_used_percent"] > 80:
        print("⚠️  Warning: Approaching budget limit")

    await asyncio.sleep(10)

# Final cost
final_cost = await get_workflow_cost(wf["workflow_id"])
print(f"\nFinal cost: ${final_cost['total_cost_usd']:.4f}")
```

### Example 4: Cost Comparison
```python
# Compare costs across multiple workflows
workflows = await list_workflows(status="completed", limit=10)

for wf_summary in workflows["workflows"]:
    cost = await get_workflow_cost(wf_summary["workflow_id"])

    print(f"{wf_summary['definition_name']}: ${cost['total_cost_usd']:.4f}")

    # Show most expensive activity
    if cost["activities"]:
        most_expensive = max(cost["activities"], key=lambda a: a["cost_usd"])
        if most_expensive["cost_usd"] > 0:
            print(f"  Most expensive: {most_expensive['activity_key']} (${most_expensive['cost_usd']:.4f})")
```

### Example 5: Provider Cost Analysis
```python
# Analyze costs by provider
cost = await get_workflow_cost(wf_id)

print("Cost by Provider:")
for provider, amount in cost["providers"].items():
    if amount > 0:
        print(f"  {provider}: ${amount:.4f}")

# Calculate percentage
total = cost["total_cost_usd"]
for provider, amount in cost["providers"].items():
    if amount > 0:
        pct = (amount / total * 100) if total > 0 else 0
        print(f"  {provider}: {pct:.1f}%")
```

---

## Cost Estimation Logic

### LLM Activities
1. **Extract model from workflow definition**
2. **Assume typical token counts:**
   - Input: ~100 tokens (conservative)
   - Output: `max_tokens` parameter or 1024 default
3. **Look up pricing** in built-in table
4. **Calculate:**
   ```python
   cost = (input_tokens / 1_000_000 * input_price) + \
          (output_tokens / 1_000_000 * output_price)
   ```
5. **Generate range:**
   - Min: 50% of estimate (shorter response)
   - Max: 200% of estimate (longer prompt/response)

### Embedding Activities
- **Assume:** ~500 tokens per text
- **Pricing:** $0.02 per 1M tokens (OpenAI text-embedding-3-small)
- **Calculation:** `cost = 0.00001` per embedding

### Other Activities
- **http_request, postgres_query, postgres_transaction:** $0
- **email_send:** $0.0001 per email (rough estimate)
- **script:** $0 (compute not metered)

---

## Testing

**Test Coverage:**
- ✅ Cost breakdown includes all activities
- ✅ Provider breakdown calculated correctly
- ✅ Budget percentage calculated correctly
- ✅ Estimation logic for different activity types
- ✅ Cost range generation

**Schema Tests:**
- ✅ WorkflowCost Pydantic schema (100% coverage)
- ✅ ActivityCost schema
- ✅ TokenUsage schema
- ✅ WorkflowCostEstimate schema

---

## Limitations

### Estimation Accuracy
- **Typical accuracy:** ±50% (depends on response length variation)
- **Not accounted for:**
  - Retries (could multiply cost)
  - Model fallback (could reduce cost)
  - Actual prompt length (varies with input)
  - Response length (varies with request complexity)

### Real-time Tracking
- Cost is updated after each activity completes
- Running activities don't show partial costs
- Small delay (<5s) between activity completion and cost update

---

## Related User Stories

- **US-MCP-5:** Submit Workflows (uses budget limits)
- **US-MCP-7:** Monitor Workflow Status
- **US-MCP-11:** Visualize Cost Breakdowns

---

## Documentation

- **User Guide:** `docs/implementation/mcp-userguide.md` - Q&A Section: "How does the agent handle workflow costs?"
- **Development Plan:** `docs/implementation/mcp-server-development-plan.md` - Task MCP-5
- **PRD:** `docs/implementation/mcp-server-prd.md` - Cost tracking as competitive advantage

---

## Future Enhancements

### Potential Additions
- **Historical Cost Trends:** Track costs over time
- **Cost Alerts:** Notify when workflows exceed expected cost
- **Cost Optimization Suggestions:** Suggest cheaper models
- **Cost Forecasting:** Predict monthly costs based on usage patterns
- **Custom Pricing:** Allow users to define their own pricing (for custom models)
