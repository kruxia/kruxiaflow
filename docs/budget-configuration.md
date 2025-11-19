# Budget Configuration Guide

This guide explains how to configure budget limits and cost tracking for StreamFlow workflows that use LLM activities.

## Overview

StreamFlow provides first-class budget enforcement for workflows that use usage-based activities like LLM API calls. Budget tracking operates at two levels:

1. **Activity-level budgets**: Per-activity spending limits with retry budgets
2. **Workflow-level budgets**: Total spending limits across all activities

Budget enforcement is handled by the orchestrator using real-time cost tracking and database-backed pricing data.

## Budget Configuration

### Workflow-Level Budget

Set a total budget limit for the entire workflow in the workflow definition:

```yaml
name: content_moderation_pipeline
description: Multi-step content analysis with budget control

settings:
  budget:
    limit_usd: 5.00
    on_exceeded: abort  # or "alert"

activities:
  # ... activities
```

**Budget Settings**:

| Field         | Type   | Required | Description                                    |
|---------------|--------|----------|------------------------------------------------|
| `limit_usd`   | number | Yes      | Maximum spend in USD for the workflow          |
| `on_exceeded` | string | Yes      | Action when budget exceeded: `abort` or `alert` |

**Actions**:
- `abort`: Immediately stop workflow execution when budget is exceeded
- `alert`: Log warning but continue execution (for monitoring)

### Activity-Level Budget

Control spending for individual activities (useful with retries):

```yaml
activities:
  - key: analyze_content
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-sonnet-20241022
      prompt: "Analyze this content: {{INPUT.content}}"
      max_tokens: 1000
    settings:
      retry:
        max_attempts: 3
        strategy: exponential
        base_seconds: 2
      budget:
        limit_usd: 0.50
        on_exceeded: abort
```

**Use case**: Prevent runaway costs from repeated retries or expensive LLM calls.

---

## Budget Enforcement Behavior

### Pre-Execution Check

Before executing an activity, the orchestrator:

1. Queries current workflow cost from database
2. Compares against `budget.limit_usd`
3. If budget exceeded:
   - `abort`: Fail the activity with `BudgetExceeded` error
   - `alert`: Log warning and proceed

### Post-Execution Tracking

After activity completion:

1. Worker returns token usage counts (prompt_tokens, completion_tokens, cached_tokens)
2. Orchestrator queries model pricing from database
3. Orchestrator calculates cost in USD
4. Cost is recorded in `activity_costs` table
5. Database trigger updates `workflows.total_cost_usd`

### Cost Calculation

**Formula**:
```
cost_usd = (prompt_tokens × prompt_price_per_million / 1,000,000)
         + (completion_tokens × completion_price_per_million / 1,000,000)
         + (cached_tokens × cached_price_per_million / 1,000,000)
```

**Pricing source**: PostgreSQL `llm_models` table, loaded via `streamflow seed-llm` command.

---

## Configuration Examples

### Example 1: Simple Budget Limit

Workflow with $1 total budget, abort on exceed:

```yaml
name: simple_analysis
description: Single LLM analysis with budget

settings:
  budget:
    limit_usd: 1.00
    on_exceeded: abort

activities:
  - key: analyze
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-haiku-20241022
      prompt: "Summarize: {{INPUT.text}}"
      max_tokens: 200
```

**Expected behavior**:
- Single activity runs normally if cost < $1
- Workflow fails with `BudgetExceeded` if cost > $1

---

### Example 2: Retry with Activity Budget

Activity with retries and per-activity budget:

```yaml
name: retry_with_budget
description: Retry expensive operation with budget control

activities:
  - key: extract_entities
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-sonnet-20241022
      prompt: "Extract entities from: {{INPUT.document}}"
      max_tokens: 2000
    settings:
      retry:
        max_attempts: 5
        strategy: exponential
        base_seconds: 1
      budget:
        limit_usd: 0.25  # Max $0.25 for all retry attempts
        on_exceeded: abort
```

**Expected behavior**:
- First attempt costs ~$0.08
- If fails, retry with exponential backoff
- If total cost across retries exceeds $0.25, stop retrying

---

### Example 3: Multi-Activity Pipeline with Global Budget

Multiple activities with workflow-level budget:

```yaml
name: content_pipeline
description: Multi-step content processing with global budget

settings:
  budget:
    limit_usd: 2.00
    on_exceeded: abort

activities:
  - key: classify
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-haiku-20241022
      prompt: "Classify: {{INPUT.content}}"
      max_tokens: 50

  - key: extract_keywords
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-haiku-20241022
      prompt: "Extract keywords: {{INPUT.content}}"
      max_tokens: 100
    depends_on:
      - classify

  - key: generate_summary
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-sonnet-20241022
      prompt: "Summarize: {{INPUT.content}}"
      max_tokens: 500
    depends_on:
      - extract_keywords
```

**Expected behavior**:
- Three activities run sequentially
- Running total tracked in `workflows.total_cost_usd`
- If cumulative cost exceeds $2.00, workflow aborts

---

### Example 4: Alert-Only Monitoring

Track costs without enforcement (monitoring):

```yaml
name: monitored_pipeline
description: Track costs but don't enforce limits

settings:
  budget:
    limit_usd: 10.00
    on_exceeded: alert  # Log warning, don't abort

activities:
  - key: analyze
    worker: builtin
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-opus-20241022
      prompt: "Deep analysis: {{INPUT.document}}"
      max_tokens: 4000
```

**Expected behavior**:
- Activity executes normally
- If cost > $10, log warning but continue
- Useful for establishing cost baselines

---

### Example 5: Multi-Provider Fallback with Budget

Cost-conscious fallback chain (cheap to expensive):

```yaml
name: fallback_with_budget
description: Try cheap models first, fallback to expensive

settings:
  budget:
    limit_usd: 1.00
    on_exceeded: abort

activities:
  - key: analyze
    worker: builtin
    activity_name: llm_prompt
    parameters:
      # Try Ollama (free), then Haiku (cheap), then Sonnet (expensive)
      model:
        - ollama/llama3.2
        - anthropic/claude-3-5-haiku-20241022
        - anthropic/claude-3-5-sonnet-20241022
      prompt: "Analyze sentiment: {{INPUT.review}}"
      max_tokens: 100
    settings:
      retry:
        max_attempts: 3
```

**Expected behavior**:
- First attempt uses Ollama (no cost)
- If Ollama unavailable, try Haiku (~$0.002)
- If Haiku fails, try Sonnet (~$0.01)
- Abort if total cost across retries exceeds $1.00

---

## Budget Status Monitoring

### Query Workflow Budget Status

Use the Cost Dashboard API to check budget status:

**Endpoint**: `GET /api/v1/workflows/:workflow_id/cost`

**Response**:
```json
{
  "workflow_id": "wf_123456",
  "total_cost_usd": "0.4523",
  "budget_limit_usd": "1.00",
  "budget_remaining_usd": "0.5477",
  "budget_used_percentage": 45.23,
  "activity_count": 5,
  "created_at": "2025-11-18T10:00:00Z",
  "updated_at": "2025-11-18T10:05:23Z"
}
```

### Query Cost History

**Endpoint**: `GET /api/v1/workflows/:workflow_id/cost/history`

**Response**:
```json
{
  "workflow_id": "wf_123456",
  "total_cost_usd": "0.4523",
  "activities": [
    {
      "activity_id": "act_001",
      "activity_key": "classify",
      "provider": "anthropic",
      "model": "claude-3-5-haiku-20241022",
      "prompt_tokens": 150,
      "completion_tokens": 25,
      "cached_tokens": 0,
      "cost_usd": "0.0012",
      "created_at": "2025-11-18T10:01:00Z"
    },
    {
      "activity_id": "act_002",
      "activity_key": "analyze",
      "provider": "anthropic",
      "model": "claude-3-5-sonnet-20241022",
      "prompt_tokens": 2500,
      "completion_tokens": 800,
      "cached_tokens": 1200,
      "cost_usd": "0.4511",
      "created_at": "2025-11-18T10:05:23Z"
    }
  ]
}
```

---

## Budget Best Practices

### 1. Set Conservative Limits

Start with conservative budgets and increase based on observed costs:

```yaml
settings:
  budget:
    limit_usd: 0.10  # Start low, increase after testing
    on_exceeded: abort
```

### 2. Use Activity Budgets for Retries

Prevent retry loops from consuming budget:

```yaml
settings:
  retry:
    max_attempts: 10
  budget:
    limit_usd: 0.25  # Cap total retry cost
    on_exceeded: abort
```

### 3. Monitor Before Enforcing

Use `alert` mode to establish baselines:

```yaml
# Development/staging
settings:
  budget:
    limit_usd: 5.00
    on_exceeded: alert  # Monitor, don't abort

# Production
settings:
  budget:
    limit_usd: 5.00
    on_exceeded: abort  # Enforce limits
```

### 4. Use Cheap Models for Development

Test with Ollama (free) or Haiku (cheap) before production:

```yaml
# Development
parameters:
  model: ollama/llama3.2

# Production
parameters:
  model: anthropic/claude-3-5-sonnet-20241022
```

### 5. Combine with max_tokens

Limit both tokens and budget:

```yaml
parameters:
  model: anthropic/claude-3-5-sonnet-20241022
  max_tokens: 1000  # Limit output tokens
settings:
  budget:
    limit_usd: 0.50  # Limit total cost
```

---

## Cost Estimation

Estimate costs before running workflows using the model catalog:

**Endpoint**: `POST /api/v1/llm/models/search`

**Request**:
```json
{
  "models": [
    {
      "provider": "anthropic",
      "model_name": "claude-3-5-sonnet-20241022"
    }
  ]
}
```

**Response**:
```json
{
  "models": [
    {
      "provider": "anthropic",
      "model_name": "claude-3-5-sonnet-20241022",
      "context_window": 200000,
      "max_output_tokens": 8192,
      "prompt_price_per_million": 3.00,
      "completion_price_per_million": 15.00,
      "cached_price_per_million": 0.30
    }
  ]
}
```

**Manual calculation**:
```
# For 1000 prompt tokens, 500 completion tokens:
cost = (1000 × 3.00 / 1,000,000) + (500 × 15.00 / 1,000,000)
     = 0.003 + 0.0075
     = $0.0105
```

---

## Troubleshooting

### Budget Exceeded Immediately

**Symptom**: Workflow fails with `BudgetExceeded` on first activity

**Causes**:
1. Budget limit too low for the model
2. Previous workflow runs already consumed budget (if reusing workflow)
3. Large prompt with expensive model

**Solutions**:
- Increase `budget.limit_usd`
- Use cheaper model (Haiku instead of Sonnet)
- Reduce `max_tokens`
- Check cost estimate before running

### Costs Higher Than Expected

**Symptom**: Activities cost more than estimated

**Causes**:
1. Cached tokens not used (first run)
2. Prompt larger than expected
3. Model generated more tokens than `max_tokens`
4. Using expensive model in fallback chain

**Solutions**:
- Check actual token counts in cost history API
- Verify prompt size
- Use cheaper models in fallback chain
- Enable prompt caching (Anthropic)

### Budget Not Enforced

**Symptom**: Workflow continues after exceeding budget

**Causes**:
1. `on_exceeded: alert` instead of `abort`
2. Budget check happens before execution (cost recorded after)

**Solutions**:
- Set `on_exceeded: abort` for enforcement
- Pre-execution check uses current cost, not estimated cost
- Add buffer to budget limits

---

## Related Documentation

- [Ollama Deployment](./ollama-deployment.md) - Self-hosted LLM for zero API costs
- [Cost Dashboard API](./cost-dashboard-api.md) - Monitoring and analytics
- [LLM Activities](./llm-activities.md) - Using LLM activities in workflows
- [Multi-Provider Fallback](./multi-provider-fallback.md) - Cost-optimized fallback chains
