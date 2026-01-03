# Cost Dashboard API Reference

This document describes the Kruxia Flow Cost Dashboard API endpoints for tracking LLM costs, budget monitoring, and model catalog queries.

## Overview

Kruxia Flow provides real-time cost tracking and analytics for workflows that use LLM activities. The Cost Dashboard API enables:

1. **Model Discovery**: Query available LLM providers and models with pricing
2. **Workflow Cost Tracking**: Monitor total costs and budget status per workflow
3. **Activity Cost History**: Detailed token usage and cost breakdown per activity
4. **Cost Analytics**: Aggregated metrics across all workflows

All costs are tracked in USD with Decimal precision to prevent floating-point errors.

---

## Authentication

All API endpoints require authentication using a Bearer token:

```bash
Authorization: Bearer <your-jwt-token>
```

To obtain a token, use the authentication endpoints (see Authentication API documentation).

---

## LLM Catalog API

### List LLM Providers

Get all available LLM providers with their capabilities.

**Endpoint**: `GET /api/v1/llm/providers`

**Response**: `200 OK`

```json
[
  {
    "name": "anthropic",
    "display_name": "Anthropic",
    "api_endpoint": "https://api.anthropic.com/v1",
    "supports_completion": true,
    "supports_embeddings": false,
    "supports_streaming": true,
    "requires_api_key": true
  },
  {
    "name": "openai",
    "display_name": "OpenAI",
    "api_endpoint": "https://api.openai.com/v1",
    "supports_completion": true,
    "supports_embeddings": true,
    "supports_streaming": true,
    "requires_api_key": true
  },
  {
    "name": "google",
    "display_name": "Google AI",
    "api_endpoint": "https://generativelanguage.googleapis.com/v1beta",
    "supports_completion": true,
    "supports_embeddings": true,
    "supports_streaming": true,
    "requires_api_key": true
  },
  {
    "name": "ollama",
    "display_name": "Ollama",
    "api_endpoint": null,
    "supports_completion": true,
    "supports_embeddings": true,
    "supports_streaming": true,
    "requires_api_key": false
  }
]
```

**Fields**:
- `name` (string): Provider identifier used in workflows (e.g., "anthropic")
- `display_name` (string): Human-readable provider name
- `api_endpoint` (string|null): Base API URL (null for Ollama - configured via env var)
- `supports_completion` (bool): Supports text completion/chat
- `supports_embeddings` (bool): Supports embedding generation
- `supports_streaming` (bool): Supports streaming responses
- `requires_api_key` (bool): Requires API key for authentication

**Performance**: Target <10ms P99 latency

**Example**:
```bash
curl -X GET http://localhost:8080/api/v1/llm/providers \
  -H "Authorization: Bearer $TOKEN"
```

---

### Search Models

Search for models by provider name, model name, or both. Supports batch lookup.

**Endpoint**: `POST /api/v1/llm/models/search`

**Request Body**:
```json
{
  "models": [
    {
      "provider": "anthropic",
      "model": null
    },
    {
      "provider": "openai",
      "model": "gpt-4o"
    }
  ]
}
```

**Search Criteria**:
- **Provider only**: Returns all models from that provider
- **Model only**: Returns models with that name from any provider
- **Both**: Returns specific model from specific provider
- **Empty array**: Returns empty results

**Response**: `200 OK`

```json
{
  "models": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "provider": "anthropic",
      "name": "claude-3-5-sonnet-20241022",
      "display_name": "Claude 3.5 Sonnet",
      "input_price_per_million": 3.00,
      "output_price_per_million": 15.00,
      "cached_input_price_per_million": 0.30,
      "supports_completion": true,
      "supports_embeddings": false,
      "context_window": 200000,
      "max_output_tokens": 8192
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440001",
      "provider": "anthropic",
      "name": "claude-3-5-haiku-20241022",
      "display_name": "Claude 3.5 Haiku",
      "input_price_per_million": 0.80,
      "output_price_per_million": 4.00,
      "cached_input_price_per_million": 0.08,
      "supports_completion": true,
      "supports_embeddings": false,
      "context_window": 200000,
      "max_output_tokens": 8192
    },
    {
      "id": "550e8400-e29b-41d4-a716-446655440002",
      "provider": "openai",
      "name": "gpt-4o",
      "display_name": "GPT-4 Omni",
      "input_price_per_million": 2.50,
      "output_price_per_million": 10.00,
      "cached_input_price_per_million": null,
      "supports_completion": true,
      "supports_embeddings": false,
      "context_window": 128000,
      "max_output_tokens": 16384
    }
  ]
}
```

**Fields**:
- `id` (UUID): Model unique identifier
- `provider` (string): Provider name
- `name` (string): Model name used in workflows
- `display_name` (string): Human-readable model name
- `input_price_per_million` (Decimal): Cost per 1M prompt/input tokens (USD)
- `output_price_per_million` (Decimal): Cost per 1M completion/output tokens (USD)
- `cached_input_price_per_million` (Decimal|null): Cost per 1M cached tokens (USD), if supported
- `supports_completion` (bool): Supports text completion
- `supports_embeddings` (bool): Supports embeddings
- `context_window` (int|null): Maximum context window in tokens
- `max_output_tokens` (int|null): Maximum output tokens

**Performance**: Target <20ms P99 latency for batch queries with <10 criteria

**Examples**:

```bash
# Get all Anthropic models
curl -X POST http://localhost:8080/api/v1/llm/models/search \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "models": [
      {"provider": "anthropic", "model": null}
    ]
  }'

# Get a specific model
curl -X POST http://localhost:8080/api/v1/llm/models/search \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "models": [
      {"provider": "openai", "model": "gpt-4o"}
    ]
  }'

# Batch lookup (multiple models)
curl -X POST http://localhost:8080/api/v1/llm/models/search \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "models": [
      {"provider": "anthropic", "model": "claude-3-5-sonnet-20241022"},
      {"provider": "anthropic", "model": "claude-3-5-haiku-20241022"},
      {"provider": "openai", "model": "gpt-4o"}
    ]
  }'
```

---

## Cost Tracking API

### Get Workflow Cost Summary

Get cost summary for a specific workflow, including total cost, budget status, and activity count.

**Endpoint**: `GET /api/v1/workflows/:workflow_id/cost`

**Path Parameters**:
- `workflow_id` (UUID): Workflow identifier

**Response**: `200 OK`

```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "workflow_name": "content_moderation_pipeline",
  "total_cost_usd": "0.4523",
  "budget_limit_usd": "1.00",
  "budget_remaining_usd": "0.5477",
  "total_activities": 5
}
```

**Fields**:
- `workflow_id` (UUID): Workflow identifier
- `workflow_name` (string): Workflow name from definition
- `total_cost_usd` (Decimal): Cumulative cost across all activities (USD)
- `budget_limit_usd` (Decimal|null): Budget limit from workflow settings (null if not set)
- `budget_remaining_usd` (Decimal|null): Remaining budget (null if no limit)
- `total_activities` (int): Number of cost-tracked activities executed

**Error Responses**:
- `404 Not Found`: Workflow does not exist
- `500 Internal Server Error`: Database query failed

**Performance**: Target <10ms P99 latency (uses materialized view)

**Example**:
```bash
curl -X GET http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost \
  -H "Authorization: Bearer $TOKEN"
```

---

### Get Workflow Cost History

Get detailed cost history for all activities in a workflow, including token usage breakdown.

**Endpoint**: `GET /api/v1/workflows/:workflow_id/cost/history`

**Path Parameters**:
- `workflow_id` (UUID): Workflow identifier

**Response**: `200 OK`

```json
[
  {
    "activity_key": "classify",
    "attempt": 1,
    "cost_usd": "0.0012",
    "prompt_tokens": 150,
    "output_tokens": 25,
    "total_tokens": 175,
    "cached_tokens": 0,
    "provider": "anthropic",
    "model": "claude-3-5-haiku-20241022",
    "budget_exceeded": false,
    "created_at": "2025-11-18T10:01:00Z"
  },
  {
    "activity_key": "analyze",
    "attempt": 1,
    "cost_usd": "0.4511",
    "prompt_tokens": 2500,
    "output_tokens": 800,
    "total_tokens": 3300,
    "cached_tokens": 1200,
    "provider": "anthropic",
    "model": "claude-3-5-sonnet-20241022",
    "budget_exceeded": false,
    "created_at": "2025-11-18T10:05:23Z"
  }
]
```

**Fields**:
- `activity_key` (string): Activity key from workflow definition
- `attempt` (int): Retry attempt number (1-indexed)
- `cost_usd` (Decimal): Total cost for this activity execution (USD)
- `prompt_tokens` (int|null): Number of prompt/input tokens
- `output_tokens` (int|null): Number of completion/output tokens
- `total_tokens` (int|null): Total tokens (prompt + output + cached)
- `cached_tokens` (int|null): Number of cached tokens (Anthropic only)
- `provider` (string): LLM provider used
- `model` (string): Model name used
- `budget_exceeded` (bool|null): Whether this execution exceeded activity budget
- `created_at` (DateTime): Timestamp when cost was recorded (ISO 8601)

**Notes**:
- Results ordered by creation time (oldest first)
- Returns empty array if no costs recorded
- Token fields may be null if provider didn't return them

**Performance**: Target <50ms P99 latency for workflows with <1000 activities

**Example**:
```bash
curl -X GET http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history \
  -H "Authorization: Bearer $TOKEN"
```

---

### Get Cost Analytics

Get aggregated cost analytics across all workflows within a date range.

**Endpoint**: `GET /api/v1/cost/analytics`

**Query Parameters**:
- `start_date` (ISO 8601 DateTime, optional): Start date for analytics (default: 30 days ago)
- `end_date` (ISO 8601 DateTime, optional): End date for analytics (default: now)

**Response**: `200 OK`

```json
{
  "total_workflows": 42,
  "total_cost_usd": "127.45",
  "avg_cost_per_activity": "0.3542",
  "start_date": "2025-10-19T00:00:00Z",
  "end_date": "2025-11-18T23:59:59Z"
}
```

**Fields**:
- `total_workflows` (int): Number of unique workflows with costs in date range
- `total_cost_usd` (Decimal): Sum of all activity costs (USD)
- `avg_cost_per_activity` (Decimal): Average cost per activity execution (USD)
- `start_date` (DateTime): Start of analytics period (ISO 8601)
- `end_date` (DateTime): End of analytics period (ISO 8601)

**Performance**: Target <100ms P99 latency (aggregation query)

**Examples**:

```bash
# Last 30 days (default)
curl -X GET http://localhost:8080/api/v1/cost/analytics \
  -H "Authorization: Bearer $TOKEN"

# Custom date range
curl -X GET "http://localhost:8080/api/v1/cost/analytics?start_date=2025-11-01T00:00:00Z&end_date=2025-11-18T23:59:59Z" \
  -H "Authorization: Bearer $TOKEN"

# Last 7 days
START_DATE=$(date -u -d '7 days ago' +%Y-%m-%dT%H:%M:%SZ)
END_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
curl -X GET "http://localhost:8080/api/v1/cost/analytics?start_date=$START_DATE&end_date=$END_DATE" \
  -H "Authorization: Bearer $TOKEN"
```

---

## Common Use Cases

### 1. Cost Estimation Before Running Workflow

```bash
# Step 1: Search for model pricing
curl -X POST http://localhost:8080/api/v1/llm/models/search \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "models": [
      {"provider": "anthropic", "model": "claude-3-5-sonnet-20241022"}
    ]
  }'

# Step 2: Calculate estimated cost
# Formula: (prompt_tokens × input_price_per_million / 1,000,000)
#        + (output_tokens × output_price_per_million / 1,000,000)
#
# Example: 1000 prompt tokens, 500 output tokens, Sonnet model
# Cost = (1000 × 3.00 / 1,000,000) + (500 × 15.00 / 1,000,000)
#      = 0.003 + 0.0075
#      = $0.0105
```

### 2. Monitor Workflow Budget Status

```bash
# Get current cost and budget status
curl -X GET http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost \
  -H "Authorization: Bearer $TOKEN"

# Response shows:
# - total_cost_usd: How much has been spent
# - budget_remaining_usd: How much budget remains
# - total_activities: Number of activities executed
```

### 3. Analyze Cost by Activity

```bash
# Get detailed breakdown of which activities cost the most
curl -X GET http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history \
  -H "Authorization: Bearer $TOKEN" \
  | jq '.[] | {activity_key, cost_usd, model, total_tokens}'

# Example output:
# {
#   "activity_key": "classify",
#   "cost_usd": "0.0012",
#   "model": "claude-3-5-haiku-20241022",
#   "total_tokens": 175
# }
```

### 4. Cost Trending and Optimization

```bash
# Get cost analytics for last 30 days
curl -X GET http://localhost:8080/api/v1/cost/analytics \
  -H "Authorization: Bearer $TOKEN"

# Identify trends:
# - avg_cost_per_activity: Is this increasing over time?
# - total_cost_usd: Total spend for the period
# - Compare against budget targets
```

### 5. Model Comparison for Cost Optimization

```bash
# Compare pricing across providers
curl -X POST http://localhost:8080/api/v1/llm/models/search \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "models": [
      {"provider": "anthropic", "model": null},
      {"provider": "openai", "model": null}
    ]
  }' | jq '.models[] | {provider, name, input_price_per_million, output_price_per_million}'

# Use cheaper models (Haiku vs Sonnet) for less critical tasks
```

---

## Cost Calculation Details

### Token Pricing Formula

```
cost_usd = (prompt_tokens × prompt_price_per_million / 1,000,000)
         + (completion_tokens × completion_price_per_million / 1,000,000)
         + (cached_tokens × cached_price_per_million / 1,000,000)
```

### Cached Tokens (Anthropic Only)

Anthropic models support prompt caching, which dramatically reduces costs for repeated prompts:

- **Standard prompt tokens**: $3.00 per million (Sonnet)
- **Cached prompt tokens**: $0.30 per million (Sonnet) - 10x cheaper

Caching is automatic when the same system prompt is used across multiple requests.

### Pricing Source

All pricing data is stored in the `llm_models` PostgreSQL table and loaded via:

```bash
kruxiaflow seed-llm config/llm_models.yaml
```

Pricing is queried at activity execution time, enabling:
- Dynamic pricing updates without worker redeployment
- Per-tenant custom pricing (future)
- Volume discounts (future)

---

## Database Schema

### Tables

**llm_providers**:
- Provider metadata (name, capabilities, API endpoint)
- Static reference data

**llm_models**:
- Model catalog with pricing per provider
- Updated via seed command

**activity_costs**:
- Per-activity cost records
- Token usage breakdown
- Populated by orchestrator after activity completion

**workflows**:
- `total_cost_usd`: Running total (updated via trigger)
- `budget_limit_usd`: Budget limit from workflow settings

### Materialized View

**workflow_cost_summary**:
- Pre-aggregated workflow cost data
- Refreshed automatically via trigger on `activity_costs` INSERT
- Enables fast cost summary queries (<10ms)

---

## Error Handling

### Common Error Responses

**404 Not Found**:
```json
{
  "error": "Workflow not found"
}
```

**500 Internal Server Error**:
```json
{
  "error": "Internal server error"
}
```

Errors are logged with tracing for debugging.

---

## Performance Targets

| Endpoint                         | P99 Latency | Optimization                    |
|----------------------------------|---------|---------------------------------|
| `GET /llm/providers`             | <10ms   | Simple table scan (4-5 rows)    |
| `POST /llm/models/search`        | <20ms   | Indexed query with UNNEST       |
| `GET /workflows/:id/cost`        | <10ms   | Materialized view query         |
| `GET /workflows/:id/cost/history`| <50ms   | Indexed by workflow_id          |
| `GET /cost/analytics`            | <100ms  | Aggregation with date index     |

---

## Related Documentation

- [Budget Configuration](./budget-configuration.md) - Setting up budget limits
- [LLM Activities Guide](./llm-activities.md) - Using LLM activities in workflows
- [Ollama Deployment](./ollama-deployment.md) - Self-hosted LLM setup
