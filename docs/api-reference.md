# API Reference

Complete reference for all Kruxia Flow API endpoints.

**Base URL**: `http://localhost:8080` (default)

---

## Authentication

All protected endpoints require a JWT Bearer token in the Authorization header:

```
Authorization: Bearer <access_token>
```

### Obtain Access Token

```http
POST /api/v1/oauth/token
Content-Type: application/json
```

Also accepts `application/x-www-form-urlencoded` per RFC 6749.

**Request Body** (Client Credentials):
```json
{
  "grant_type": "client_credentials",
  "client_id": "your_client_id",
  "client_secret": "your_client_secret"
}
```

**Request Body** (Password Grant):
```json
{
  "grant_type": "password",
  "username": "user@example.com",
  "password": "your_password"
}
```

**Request Body** (Refresh Token):
```json
{
  "grant_type": "refresh_token",
  "refresh_token": "existing_refresh_token"
}
```

**Response** (200 OK):
```json
{
  "access_token": "eyJhbGc...",
  "token_type": "Bearer",
  "expires_in": 86400,
  "refresh_token": "optional_for_password_grant",
  "scope": null
}
```

---

## Health & Info

### Liveness Check

```http
GET /health
```

Returns `200 OK` if the service is running.

**Response**:
```json
{"status": "ok"}
```

### Readiness Check

```http
GET /health/ready
```

Returns `200 OK` if the service is ready to accept requests (database connected, etc.), or `503 Service Unavailable` if dependencies are unhealthy.

**Response**:
```json
{
  "status": "ok",
  "checks": {
    "database": "ok",
    "event_source": "ok",
    "queue": "ok"
  }
}
```

### Connection Pool Metrics

```http
GET /health/pool
```

Returns database connection pool statistics.

**Response**:
```json
{
  "max_connections": 10,
  "utilization_percent": 25.0,
  "status": "ok"
}
```

### Service Info

```http
GET /api/v1/info
```

Returns service version and configuration information.

**Response**:
```json
{
  "version": "0.2.0",
  "build_timestamp": "2025-10-30T12:34:56Z",
  "build_git_hash": "abc1234",
  "api_version": "v1",
  "features": ["workflows", "workers", "websockets"]
}
```

---

## Workflow Definitions

### Deploy Workflow Definition

```http
POST /api/v1/workflow_definitions
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "name": "payment_processing",
  "activities": [
    {
      "key": "validate_card",
      "worker": "std",
      "activity_name": "validate_card_details",
      "parameters": {
        "card_token": "tok_123"
      }
    },
    {
      "key": "authorize_payment",
      "worker": "std",
      "activity_name": "http_request",
      "parameters": {
        "url": "https://api.payment.com/authorize",
        "method": "POST"
      },
      "depends_on": [
        {"key": "validate_card"}
      ],
      "outputs": [
        {"name": "auth_id", "type": "string"}
      ]
    }
  ]
}
```

**Response** (201 Created):
```json
{
  "name": "payment_processing",
  "version": "20251105.143022.123456",
  "created_at": "2025-11-05T14:30:22.123456Z"
}
```

**Response** (200 OK) — identical version already exists:
```json
{
  "name": "payment_processing",
  "version": "20251105.143022.123456",
  "created_at": "2025-11-05T14:30:22.123456Z",
  "unchanged": true
}
```

### List Workflow Definitions

```http
GET /api/v1/workflow_definitions
Authorization: Bearer <token>
```

**Response** (200 OK):
```json
{
  "definitions": [
    {
      "name": "payment_processing",
      "version": "20251105.143022.123456",
      "activity_count": 5,
      "created_at": "2025-11-05T14:30:22.123456Z"
    }
  ],
  "total": 1
}
```

### Get Workflow Definition

```http
GET /api/v1/workflow_definitions/:name
Authorization: Bearer <token>
```

**Query Parameters**:
- `version` - Specific version to retrieve; if omitted returns latest

**Response** (200 OK):
```json
{
  "name": "payment_processing",
  "version": "20251105.143022.123456",
  "activities": [
    {
      "key": "authorize_payment",
      "worker": "std",
      "activity_name": "http_request",
      "parameters": { ... },
      "depends_on": [ ... ],
      "outputs": [ ... ]
    }
  ],
  "created_at": "2025-11-05T14:30:22.123456Z"
}
```

---

## Workflows

### Submit Workflow

```http
POST /api/v1/workflows
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "definition_name": "payment_processing",
  "version": "20251105.143022.123456",
  "input": {
    "amount": 100.00,
    "card_token": "tok_123"
  },
  "unique_key": "order_12345_payment"
}
```

- `version` - Optional; uses latest if omitted
- `unique_key` - Optional; for idempotency (max 255 characters)

**Response** (201 Created):
```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "definition_name": "payment_processing",
  "definition_version": "20251105.143022.123456",
  "status": "created",
  "created_at": "2025-11-05T14:30:22.123456Z"
}
```

**Error Responses**:
- `404 Not Found` - Definition not found
- `409 Conflict` - Duplicate `unique_key`
- `422 Unprocessable Entity` - Validation error

### List Workflows

```http
GET /api/v1/workflows
Authorization: Bearer <token>
```

**Query Parameters**:
- `status` - Filter by workflow status (`created`, `running`, `completed`, `failed`, `paused`)
- `definition_name` - Filter by workflow definition
- `created_after` - ISO 8601 timestamp
- `created_before` - ISO 8601 timestamp
- `limit` - Maximum results to return (default 100, max 1000)
- `offset` - Pagination offset (default 0)

**Response** (200 OK):
```json
{
  "workflows": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "status": "running",
      "definition_name": "payment_processing",
      "created_at": "2025-11-06T10:00:00Z",
      "updated_at": "2025-11-06T10:00:05Z",
      "error_message": null
    }
  ],
  "total": 150,
  "count": 100,
  "limit": 100,
  "offset": 0
}
```

`error_message` carries a failed activity's error text (dead-letter
visibility): `GET /api/v1/workflows?definition_name=X&status=failed` lists
dead-letters with their errors, no SQL required.

### Get Workflow

```http
GET /api/v1/workflows/:workflow_id
Authorization: Bearer <token>
```

**Response** (200 OK):
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "running",
  "definition_name": "payment_processing",
  "created_at": "2025-11-06T10:00:00Z",
  "updated_at": "2025-11-06T10:00:05Z",
  "activities": [
    {
      "activity_key": "validate_payment",
      "status": "completed",
      "outputs": {"valid": true},
      "error": null,
      "started_at": "2025-11-06T10:00:00Z",
      "completed_at": "2025-11-06T10:00:01Z"
    }
  ],
  "error_message": null,
  "state_data": {"custom_field": "value"}
}
```

**Activity Status Values**: `not_scheduled`, `pending`, `running`, `completed`, `failed`

Each activity's `error` holds the message from its most recent failure; the
workflow-level `error_message` surfaces a failed activity's error directly.

---

## Recurring Schedules

A schedule submits a workflow definition on a cadence, server-side — no
client credentials ride the recurrence. Runs are idempotent
(`unique_key = schedule:<name>:<occurrence epoch>`); missed occurrences
during downtime collapse into at most one catch-up run; a stale past-due
schedule does not fire immediately on re-enable.

### Create Schedule

```http
POST /api/v1/schedules
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "name": "cache-flush-sweep",
  "definition_name": "cache_flush_sweep",
  "input": {},
  "interval_seconds": 120,
  "overlap_policy": "skip"
}
```

- `name` - Required, unique
- `definition_name` - Required; must exist (`definition_version` pins a
  version, default: latest at fire time)
- Exactly one of `cron` / `interval_seconds` - Required. `cron` is standard
  5-field crontab (minute granularity) or 6-field with leading seconds;
  `timezone` (IANA name, default UTC) applies to cron only
- `overlap_policy` - `skip` (default: don't submit while the previous run is
  still active) or `allow`
- `enabled` - Default true

**Response** (201 Created):
```json
{
  "id": "0198c0de-1234-7abc-9def-0123456789ab",
  "name": "cache-flush-sweep",
  "definition_name": "cache_flush_sweep",
  "definition_version": null,
  "input": {},
  "cron": null,
  "timezone": null,
  "interval_seconds": 120,
  "overlap_policy": "skip",
  "enabled": true,
  "next_run_at": "2025-11-06T10:02:00Z",
  "last_run_at": null,
  "last_workflow_id": null,
  "created_by": "my_client_id",
  "created_at": "2025-11-06T10:00:00Z",
  "updated_at": "2025-11-06T10:00:00Z"
}
```

**Errors**: 400 (invalid cadence, unknown definition), 409 (name exists)

### List Schedules

```http
GET /api/v1/schedules
Authorization: Bearer <token>
```

**Response** (200 OK): `{"schedules": [ ...schedule objects... ], "count": 1}`

### Get Schedule

```http
GET /api/v1/schedules/:schedule_id
Authorization: Bearer <token>
```

### Update Schedule

```http
PATCH /api/v1/schedules/:schedule_id
Authorization: Bearer <token>
Content-Type: application/json
```

PATCH semantics — absent fields are unchanged. Providing `cron` or
`interval_seconds` replaces the cadence wholesale and recomputes
`next_run_at`; re-enabling also recomputes it.

**Request Body** (any subset):
```json
{
  "enabled": false,
  "input": {"mode": "full"},
  "cron": "0 9 * * 1",
  "timezone": "America/Chicago",
  "interval_seconds": 300,
  "overlap_policy": "allow"
}
```

**Response** (200 OK): the updated schedule object.

### Delete Schedule

```http
DELETE /api/v1/schedules/:schedule_id
Authorization: Bearer <token>
```

**Response** (204 No Content)

**Scheduler configuration** (environment):
- `KRUXIAFLOW_SCHEDULER_ENABLED` - default `true`
- `KRUXIAFLOW_SCHEDULER_TICK_INTERVAL_MS` - default `1000`
- `KRUXIAFLOW_SCHEDULER_BATCH_LIMIT` - default `100`

The loop runs inside `kruxiaflow serve` and `kruxiaflow orchestrator`;
multiple instances are safe (SKIP LOCKED claims + idempotent unique keys).

---

## Workflow Signals

### Signal an Activity

```http
POST /api/v1/workflows/:workflow_id/signal
Authorization: Bearer <token>
Content-Type: application/json
```

Sends a signal to a waiting activity within a workflow.

**Request Body**:
```json
{
  "activity_key": "wait_for_approval",
  "event_name": "approval_received",
  "data": {
    "approved": true,
    "approver": "admin@example.com"
  }
}
```

- `activity_key` - Required, not empty
- `event_name` - Required, not empty
- `data` - Optional, any JSON object

**Response** (200 OK):
```json
{
  "signaled": true,
  "message": "Activity signaled successfully"
}
```

---

## Workflow Output

### Get Workflow Output

```http
GET /api/v1/workflows/:workflow_id/output
Authorization: Bearer <token>
```

Returns combined output from the workflow.

**Response** (200 OK):
```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "output": {
    "authorization_id": "auth_123",
    "transaction_id": "txn_456"
  },
  "cost_usd": 0.025,
  "total_tokens_used": 1500
}
```

**Error Responses**:
- `400 Bad Request` - Workflow not yet completed
- `404 Not Found` - Workflow not found

### Get Activity Output

```http
GET /api/v1/workflows/:workflow_id/activities/:activity_key/output
Authorization: Bearer <token>
```

Returns output from a specific activity, including file references.

**Response** (200 OK):
```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "activity_key": "authorize_card",
  "output": {
    "authorization_id": "auth_123",
    "approved": true
  },
  "cost_usd": 0.015,
  "files": [
    {
      "name": "receipt.pdf",
      "size": 2048,
      "content_type": "application/pdf",
      "created_at": "2025-11-06T10:00:01Z"
    }
  ]
}
```

**Error Responses**:
- `400 Bad Request` - Activity not completed
- `404 Not Found` - Workflow or activity not found

### Download Activity File

```http
GET /api/v1/workflows/:workflow_id/activities/:activity_key/files/:filename
Authorization: Bearer <token>
```

**Response**:
- `Content-Type`: File's content type
- `Content-Disposition`: `attachment; filename="{filename}"`
- Body: Binary file content (streamed)

### Upload Activity File

```http
POST /api/v1/workflows/:workflow_id/activities/:activity_key/files/:filename
Authorization: Bearer <token>
```

Upload a file associated with an activity.

**Request**: Raw file content (binary body)

**Response** (200 OK):
```json
{
  "success": true,
  "name": "document.pdf",
  "size": 2048,
  "content_type": "application/pdf"
}
```

---

## Cost Tracking

### Get Workflow Cost

```http
GET /api/v1/workflows/:workflow_id/cost
Authorization: Bearer <token>
```

Returns current cost breakdown for a workflow.

**Response** (200 OK):
```json
{
  "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
  "workflow_name": "payment_processing",
  "total_cost_usd": 0.045,
  "budget_limit_usd": 1.00,
  "budget_remaining_usd": 0.955,
  "total_activities": 3
}
```

### Get Workflow Cost History

```http
GET /api/v1/workflows/:workflow_id/cost/history
Authorization: Bearer <token>
```

**Query Parameters**:
- `limit` - Maximum results to return (default 100)
- `offset` - Pagination offset (default 0)

Returns cost history over time for a workflow.

**Response** (200 OK):
```json
{
  "costs": [
    {
      "activity_key": "authorize_payment",
      "attempt": 1,
      "cost_usd": 0.015,
      "prompt_tokens": 500,
      "output_tokens": 100,
      "total_tokens": 600,
      "cached_tokens": 50,
      "provider": "openai",
      "model": "gpt-4o"
    }
  ],
  "total": 1
}
```

**Note**: `provider` and `model` are `null` for lump-sum cost rows (external
activities reporting only a total `cost_usd`) and for non-LLM cost line items.
Rows created from external `usage` entries are shaped identically to built-in
`llm_prompt` rows. Failed attempts that reported usage appear under their own
`attempt` number.

### Get Cost Analytics

```http
GET /api/v1/cost/analytics
Authorization: Bearer <token>
```

**Query Parameters**:
- `group_by` - Grouping dimension (`provider`, `model`, `activity`, `workflow`)
- `start_date` - Start of date range (ISO 8601)
- `end_date` - End of date range (ISO 8601)

Returns cost analytics aggregated by the requested dimension.

---

## LLM Catalog

### List Providers

```http
GET /api/v1/llm/providers
Authorization: Bearer <token>
```

Returns available LLM providers.

**Response** (200 OK):
```json
[
  {
    "name": "anthropic",
    "display_name": "Anthropic",
    "api_endpoint": "https://api.anthropic.com",
    "supports_completion": true,
    "supports_embeddings": false,
    "supports_streaming": true,
    "requires_api_key": true
  }
]
```

### Search Models

```http
POST /api/v1/llm/models/search
Authorization: Bearer <token>
Content-Type: application/json
```

Search for specific models by provider and model name.

**Request Body**:
```json
{
  "models": [
    {
      "provider": "anthropic",
      "model": "claude-3-5-sonnet-20241022"
    },
    {
      "provider": "openai",
      "model": "gpt-4o"
    }
  ]
}
```

**Response** (200 OK):
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
      "max_output_tokens": 4096
    }
  ]
}
```

---

## Worker Endpoints

### Poll for Activities

```http
POST /api/v1/workers/poll
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "worker": "std",
  "worker_id": "worker_payments_01",
  "max_activities": 5
}
```

- `worker` - Required, not empty
- `worker_id` - Required, not empty
- `max_activities` - Optional (default 1, max 100)

**Response** (200 OK):
```json
{
  "activities": [
    {
      "activity_id": "550e8400-e29b-41d4-a716-446655440000",
      "workflow_id": "660e8400-e29b-41d4-a716-446655440001",
      "activity_key": "authorize_card",
      "worker": "std",
      "activity_name": "http_request",
      "parameters": {
        "card_token": "tok_123",
        "amount": 100.00
      },
      "settings": {
        "timeout": 300,
        "max_retries": 3
      },
      "timeout_seconds": 300,
      "output_definitions": [
        {"name": "document", "type": "file"}
      ],
      "signal_data": null
    }
  ],
  "count": 1
}
```

When no activities are available, `activities` is an empty array and `count` is 0.

### Heartbeat

```http
POST /api/v1/activities/:activity_id/heartbeat
Authorization: Bearer <token>
Content-Type: application/json
```

Extends the activity timeout for long-running tasks.

**Request Body**:
```json
{
  "worker_id": "worker_payments_01"
}
```

**Response** (200 OK):
```json
{
  "acknowledged": true,
  "next_heartbeat_seconds": 30
}
```

### Complete Activity

```http
POST /api/v1/activities/:activity_id/complete
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "worker_id": "worker_payments_01",
  "output": {
    "authorization_id": "auth_123",
    "approved": true
  },
  "cost_usd": null,
  "usage": [
    {
      "provider": "anthropic",
      "model": "claude-sonnet-5",
      "input_tokens": 12034,
      "output_tokens": 512,
      "cache_read_tokens": 9800,
      "cache_creation_tokens": 0,
      "cache_storage_token_hours": null,
      "cost_usd": null
    }
  ]
}
```

- `worker_id` - Required, not empty
- `output` - Required, must be a JSON object
- `cost_usd` - Optional, non-negative. Without `usage`: total activity cost,
  recorded as one cost row. With `usage`: cost **not covered by the entries**
  (e.g., a paid non-LLM API) — never repeat entry costs here.
- `usage` - Optional, one entry per LLM call made inside the activity. Each
  entry becomes an `activity_costs` row that counts against budgets. Per entry:
  - `provider`, `model` - Required, matched against the `llm_models` catalog
  - `input_tokens` - Prompt tokens, including cache reads
  - `output_tokens` - Completion tokens
  - `cache_read_tokens` - Tokens served from cache (billed at the cached-input
    price when the server computes cost)
  - `cache_creation_tokens` - Tokens written to cache (billed at the catalog's
    `cache_write_price_per_million` — e.g., 1.25x input for Anthropic — falling
    back to the input price for models without one)
  - `cache_storage_token_hours` - Optional; context-cache storage consumed, in
    token-hours (tokens held x hours held, fractional — e.g., 100k tokens held
    for 120s = 3.33). Billed at the catalog's
    `cache_storage_price_per_million_token_hours` (e.g., Gemini explicit
    caching); a model without a storage price records this component at 0 and
    returns a warning — there is no fallback price for a time-based dimension
  - `cost_usd` - Optional explicit cost for this call; overrides server-side
    computation

**Response** (200 OK):
```json
{
  "acknowledged": true,
  "warnings": [
    "unknown model 'acme/foo-1': not in the llm_models catalog; its usage will be recorded with cost 0 — supply cost_usd per entry or update the catalog"
  ]
}
```

A completion never fails because of usage metadata: an unknown provider/model
records the entry at cost 0 and returns a warning (the work is already done —
rejecting the request would time out the lease and re-run a completed,
possibly side-effectful activity). `warnings` is omitted when empty.

### Fail Activity

```http
POST /api/v1/activities/:activity_id/fail
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "worker_id": "worker_payments_01",
  "error": {
    "code": "PAYMENT_DECLINED",
    "message": "Card was declined by the bank",
    "retryable": false
  },
  "cost_usd": null,
  "usage": []
}
```

- `worker_id` - Required, not empty
- `error.code` - Required, not empty
- `error.message` - Required, not empty
- `error.retryable` - Whether the activity should be retried
- `cost_usd`, `usage` - Optional, same semantics as on completion. A failed
  attempt that made LLM calls still spent the money; it is recorded under the
  failing attempt number **before** any retry is scheduled, so the retry's
  budget check already sees the spend.

**Response** (200 OK):
```json
{
  "acknowledged": true,
  "will_retry": false,
  "warnings": []
}
```

---

## Token Streaming (WebSocket)

### Connect to Activity Stream (by ID)

```
GET /api/v1/activities/:activity_id/ws?token=<jwt>
```

WebSocket upgrade request for real-time token streaming from LLM activities. Authentication is via the `token` query parameter since WebSocket upgrade bypasses HTTP middleware.

### Connect to Activity Stream (by Key)

```
GET /api/v1/workflows/:workflow_id/activities/:activity_key/ws?token=<jwt>
```

Same as above, but resolves the activity by `workflow_id` + `activity_key` instead of the internal `activity_id`. This is the preferred endpoint for frontends that know the workflow ID and activity key from the workflow definition but not the queue-internal UUID. The handler retries the lookup briefly (3 x 100ms) to handle the race where a client connects before the activity is scheduled.

**Message Types**:

Token event:
```json
{
  "type": "token",
  "text": "Hello",
  "index": 0,
  "timestamp": "2025-11-06T10:00:00Z"
}
```

Complete event:
```json
{
  "type": "complete",
  "activity_id": "550e8400-e29b-41d4-a716-446655440000",
  "result": {"content": "Full response text"},
  "timestamp": "2025-11-06T10:00:05Z"
}
```

Error event:
```json
{
  "type": "error",
  "activity_id": "550e8400-e29b-41d4-a716-446655440000",
  "error": "Error message",
  "timestamp": "2025-11-06T10:00:05Z"
}
```

### Internal Streaming Endpoints

These endpoints are used internally by workers to publish streaming events:

#### Publish Stream Token

```http
POST /api/v1/activities/:activity_id/ws/token
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "text": "Hello",
  "index": 0
}
```

**Response** (200 OK):
```json
{
  "subscribers": 3
}
```

#### Signal Stream Complete

```http
POST /api/v1/activities/:activity_id/ws/complete
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "result": {"content": "Final response text"}
}
```

Closes all WebSocket connections for the activity.

#### Signal Stream Error

```http
POST /api/v1/activities/:activity_id/ws/error
Authorization: Bearer <token>
Content-Type: application/json
```

```json
{
  "error": "Rate limit exceeded"
}
```

Closes all WebSocket connections for the activity.

#### Get Subscriber Count

```http
GET /api/v1/activities/:activity_id/ws/subscribers
Authorization: Bearer <token>
```

**Response** (200 OK):
```json
{
  "count": 1
}
```

---

## Cache Management

### Invalidate Cache Key

```http
DELETE /api/v1/cache/:key
Authorization: Bearer <token>
```

Invalidates a specific cache entry.

**Response** (200 OK):
```json
{
  "success": true,
  "count": 1
}
```

### Invalidate Cache by Pattern

```http
POST /api/v1/cache/invalidate
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "pattern": "std.llm_prompt:*"
}
```

**Pattern Syntax** (Redis glob-style):
- `*` matches any characters
- `?` matches exactly one character
- `[abc]` matches one character from set

**Response** (200 OK):
```json
{
  "success": true,
  "count": 42
}
```

---

## API Documentation

### OpenAPI Specification

```http
GET /api/v1/openapi.json
```

Returns the OpenAPI 3.0 specification.

### ReDoc UI

```http
GET /api/v1/docs
```

Interactive API documentation viewer.

---

## Error Responses

All endpoints return structured error responses:

**400 Bad Request**:
```json
{
  "error": {
    "code": "BAD_REQUEST",
    "message": "Invalid request body",
    "details": { ... }
  }
}
```

**401 Unauthorized**:
```json
{
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Invalid or expired token"
  }
}
```

**404 Not Found**:
```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Workflow not found"
  }
}
```

**409 Conflict**:
```json
{
  "error": {
    "code": "CONFLICT",
    "message": "Duplicate unique_key"
  }
}
```

**422 Unprocessable Entity**:
```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Request validation failed",
    "details": {
      "field_errors": {
        "email": ["Invalid email format"]
      }
    }
  }
}
```

**500 Internal Server Error**:
```json
{
  "error": {
    "code": "INTERNAL_ERROR",
    "message": "An unexpected error occurred"
  }
}
```

---

## Route Summary

### Public Routes (No Authentication)

| Method | Path                      | Description             |
|--------|---------------------------|-------------------------|
| GET    | /health                   | Liveness check          |
| GET    | /health/ready             | Readiness check         |
| GET    | /health/pool              | Connection pool metrics |
| GET    | /api/v1/info              | Service information     |
| POST   | /api/v1/oauth/token       | Obtain access token     |
| GET    | /api/v1/activities/:id/ws | WebSocket streaming     |
| GET    | /api/v1/docs              | API documentation       |
| GET    | /api/v1/openapi.json      | OpenAPI specification   |

### Protected Routes (Require JWT)

| Method | Path                                                  | Description                |
|--------|-------------------------------------------------------|----------------------------|
| POST   | /api/v1/workflow_definitions                          | Deploy workflow definition |
| GET    | /api/v1/workflow_definitions                          | List workflow definitions  |
| GET    | /api/v1/workflow_definitions/:name                    | Get workflow definition    |
| POST   | /api/v1/workflows                                     | Submit workflow            |
| GET    | /api/v1/workflows                                     | List workflows             |
| GET    | /api/v1/workflows/:workflow_id                        | Get workflow               |
| POST   | /api/v1/workflows/:workflow_id/signal                 | Signal an activity         |
| GET    | /api/v1/workflows/:workflow_id/output                 | Get workflow output        |
| GET    | /api/v1/workflows/:id/activities/:key/output          | Get activity output        |
| GET    | /api/v1/workflows/:id/activities/:key/files/:filename | Download activity file     |
| POST   | /api/v1/workflows/:id/activities/:key/files/:filename | Upload activity file       |
| GET    | /api/v1/workflows/:workflow_id/cost                   | Get workflow cost          |
| GET    | /api/v1/workflows/:workflow_id/cost/history           | Get workflow cost history  |
| GET    | /api/v1/cost/analytics                                | Get cost analytics         |
| GET    | /api/v1/llm/providers                                 | List LLM providers         |
| POST   | /api/v1/llm/models/search                             | Search LLM models          |
| DELETE | /api/v1/cache/:key                                    | Invalidate cache key       |
| POST   | /api/v1/cache/invalidate                              | Invalidate cache pattern   |
| POST   | /api/v1/workers/poll                                  | Poll for activities        |
| POST   | /api/v1/activities/:id/heartbeat                      | Activity heartbeat         |
| POST   | /api/v1/activities/:id/complete                       | Complete activity          |
| POST   | /api/v1/activities/:id/fail                           | Fail activity              |
| POST   | /api/v1/activities/:id/ws/token                       | Publish stream token       |
| POST   | /api/v1/activities/:id/ws/complete                    | Signal stream complete     |
| POST   | /api/v1/activities/:id/ws/error                       | Signal stream error        |
| GET    | /api/v1/activities/:id/ws/subscribers                 | Get subscriber count       |
