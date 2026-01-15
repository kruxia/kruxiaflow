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

**Response** (200 OK):
```json
{
  "access_token": "eyJhbGc...",
  "token_type": "Bearer",
  "expires_in": 3600
}
```

---

## Health & Info

### Liveness Check

```http
GET /health
```

Returns `200 OK` if the service is running.

### Readiness Check

```http
GET /health/ready
```

Returns `200 OK` if the service is ready to accept requests (database connected, etc.).

### Connection Pool Metrics

```http
GET /health/pool
```

Returns database connection pool statistics.

### Service Info

```http
GET /api/v1/info
```

Returns service version and configuration information.

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
  "name": "my_workflow",
  "version": "1.0.0",
  "definition": { ... }
}
```

### List Workflow Definitions

```http
GET /api/v1/workflow_definitions
Authorization: Bearer <token>
```

### Get Workflow Definition

```http
GET /api/v1/workflow_definitions/:name
Authorization: Bearer <token>
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
  "workflow_definition": "my_workflow",
  "input": { ... }
}
```

**Response** (200 OK):
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "status": "pending"
}
```

### List Workflows

```http
GET /api/v1/workflows
Authorization: Bearer <token>
```

**Query Parameters**:
- `status` - Filter by workflow status
- `limit` - Maximum results to return
- `offset` - Pagination offset

### Get Workflow

```http
GET /api/v1/workflows/:workflow_id
Authorization: Bearer <token>
```

**Response** (200 OK):
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "status": "completed",
  "created_at": "2025-01-10T10:00:00Z",
  "completed_at": "2025-01-10T10:05:00Z",
  "activities": [...]
}
```

---

## Workflow Output

### Get Workflow Output

```http
GET /api/v1/workflows/:workflow_id/output
Authorization: Bearer <token>
```

Returns combined output from all completed activities.

### Get Activity Output

```http
GET /api/v1/workflows/:workflow_id/activities/:activity_key/output
Authorization: Bearer <token>
```

Returns output from a specific activity, including file references.

### Download Activity File

```http
GET /api/v1/workflows/:workflow_id/activities/:activity_key/files/:filename
Authorization: Bearer <token>
```

**Response**:
- `Content-Type`: File's content type
- `Content-Disposition`: `attachment; filename="{filename}"`
- Body: Binary file content (streamed)

---

## Cost Tracking

### Get Workflow Cost

```http
GET /api/v1/workflows/:workflow_id/cost
Authorization: Bearer <token>
```

Returns current cost breakdown for a workflow.

### Get Workflow Cost History

```http
GET /api/v1/workflows/:workflow_id/cost/history
Authorization: Bearer <token>
```

Returns cost history over time for a workflow.

### Get Cost Analytics

```http
GET /api/v1/cost/analytics
Authorization: Bearer <token>
```

**Query Parameters**:
- `start_date` - Start of date range
- `end_date` - End of date range
- `group_by` - Grouping (e.g., `workflow`, `activity`, `model`)

---

## LLM Catalog

### List Providers

```http
GET /api/v1/llm/providers
Authorization: Bearer <token>
```

Returns available LLM providers (Anthropic, OpenAI, Google, etc.).

### Search Models

```http
POST /api/v1/llm/models/search
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "provider": "anthropic",
  "capabilities": ["chat", "vision"]
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
  "worker": "my_worker",
  "name": "activity_name"
}
```

**Response** (200 OK):
```json
{
  "activity_id": "550e8400-e29b-41d4-a716-446655440000",
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "activity_key": "process_data",
  "parameters": { ... }
}
```

**Response** (204 No Content): No activities available.

### Heartbeat

```http
POST /api/v1/activities/:activity_id/heartbeat
Authorization: Bearer <token>
```

Extends the activity timeout for long-running tasks.

### Complete Activity

```http
POST /api/v1/activities/:activity_id/complete
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "output": { ... }
}
```

### Fail Activity

```http
POST /api/v1/activities/:activity_id/fail
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "error": "Error description",
  "error_code": "VALIDATION_ERROR"
}
```

---

## Token Streaming (WebSocket)

### Connect to Activity Stream

```
GET /api/v1/activities/:activity_id/ws?token=<jwt>
```

WebSocket upgrade request for real-time token streaming from LLM activities.

**Message Types**:

Token event:
```json
{"type": "token", "content": "Hello"}
```

Complete event:
```json
{"type": "complete", "content": "Full response text"}
```

Error event:
```json
{"type": "error", "error": "Error message"}
```

### Internal Streaming Endpoints

These endpoints are used internally by workers to publish streaming events:

| Method | Path                                       | Description            |
|--------|-------------------------------------------|------------------------|
| POST   | /api/v1/activities/:id/ws/token           | Publish token          |
| POST   | /api/v1/activities/:id/ws/complete        | Signal stream complete |
| POST   | /api/v1/activities/:id/ws/error           | Signal stream error    |
| GET    | /api/v1/activities/:id/ws/subscribers     | Get subscriber count   |

---

## Cache Management

### Invalidate Cache Key

```http
DELETE /api/v1/cache/:key
Authorization: Bearer <token>
```

Invalidates a specific cache entry.

### Invalidate Cache by Pattern

```http
POST /api/v1/cache/invalidate
Authorization: Bearer <token>
Content-Type: application/json
```

**Request Body**:
```json
{
  "pattern": "workflow:*"
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

All endpoints return standard error responses:

**400 Bad Request**:
```json
{
  "error": "validation_error",
  "message": "Invalid request body",
  "details": { ... }
}
```

**401 Unauthorized**:
```json
{
  "error": "unauthorized",
  "message": "Invalid or expired token"
}
```

**404 Not Found**:
```json
{
  "error": "not_found",
  "message": "Workflow not found"
}
```

**500 Internal Server Error**:
```json
{
  "error": "internal_error",
  "message": "An unexpected error occurred"
}
```

---

## Route Summary

### Public Routes (No Authentication)

| Method | Path                     | Description                 |
|--------|--------------------------|----------------------------|
| GET    | /health                  | Liveness check              |
| GET    | /health/ready            | Readiness check             |
| GET    | /health/pool             | Connection pool metrics     |
| GET    | /api/v1/info             | Service information         |
| POST   | /api/v1/oauth/token      | Obtain access token         |
| GET    | /api/v1/activities/:id/ws| WebSocket streaming         |
| GET    | /api/v1/docs             | API documentation (ReDoc)   |
| GET    | /api/v1/openapi.json     | OpenAPI specification       |

### Protected Routes (Require JWT)

| Method | Path                                                      | Description                  |
|--------|-----------------------------------------------------------|------------------------------|
| POST   | /api/v1/workflow_definitions                              | Deploy workflow definition   |
| GET    | /api/v1/workflow_definitions                              | List workflow definitions    |
| GET    | /api/v1/workflow_definitions/:name                        | Get workflow definition      |
| POST   | /api/v1/workflows                                         | Submit workflow              |
| GET    | /api/v1/workflows                                         | List workflows               |
| GET    | /api/v1/workflows/:workflow_id                            | Get workflow                 |
| GET    | /api/v1/workflows/:workflow_id/cost                       | Get workflow cost            |
| GET    | /api/v1/workflows/:workflow_id/cost/history               | Get workflow cost history    |
| GET    | /api/v1/cost/analytics                                    | Get cost analytics           |
| GET    | /api/v1/workflows/:workflow_id/output                     | Get workflow output          |
| GET    | /api/v1/workflows/:id/activities/:key/output              | Get activity output          |
| GET    | /api/v1/workflows/:id/activities/:key/files/:filename     | Download activity file       |
| GET    | /api/v1/llm/providers                                     | List LLM providers           |
| POST   | /api/v1/llm/models/search                                 | Search LLM models            |
| DELETE | /api/v1/cache/:key                                        | Invalidate cache key         |
| POST   | /api/v1/cache/invalidate                                  | Invalidate cache by pattern  |
| POST   | /api/v1/workers/poll                                      | Poll for activities          |
| POST   | /api/v1/activities/:id/heartbeat                          | Activity heartbeat           |
| POST   | /api/v1/activities/:id/complete                           | Complete activity            |
| POST   | /api/v1/activities/:id/fail                               | Fail activity                |
| POST   | /api/v1/activities/:id/ws/token                           | Publish stream token         |
| POST   | /api/v1/activities/:id/ws/complete                        | Signal stream complete       |
| POST   | /api/v1/activities/:id/ws/error                           | Signal stream error          |
| GET    | /api/v1/activities/:id/ws/subscribers                     | Get subscriber count         |
