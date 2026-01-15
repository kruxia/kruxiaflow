# API Documentation Discrepancies

**Date**: 2026-01-10
**Status**: Resolved
**Resolved**: 2026-01-14
**Priority**: Medium

## Summary

Audit of API routes vs documentation reveals missing documentation and outdated references in `docs/architecture.md`.

| Category                         | Count |
|----------------------------------|-------|
| Routes in code                   | 32    |
| Routes with documentation        | 24    |
| **Routes missing documentation** | **6** |
| Routes in docs but outdated      | 10+   |

---

## Missing Documentation

Routes implemented in code but not documented:

| Route                                        | Purpose                     |
|----------------------------------------------|-----------------------------|
| `GET /health/pool`                           | Connection pool metrics     |
| `DELETE /api/v1/cache/:key`                  | Invalidate cache entry      |
| `POST /api/v1/cache/invalidate`              | Invalidate cache by pattern |
| `POST /api/v1/activities/:id/ws/complete`    | Internal: stream completion |
| `POST /api/v1/activities/:id/ws/error`       | Internal: stream error      |
| `GET /api/v1/activities/:id/ws/subscribers`  | Internal: subscriber count  |

---

## Outdated Documentation

Discrepancies in `docs/architecture.md`:

| Documented Route                               | Actual Route in Code                           | Issue               |
|------------------------------------------------|------------------------------------------------|---------------------|
| `POST /api/v1/auth/token`                      | `POST /api/v1/oauth/token`                     | Wrong path          |
| `POST /api/v1/auth/refresh`                    | -                                              | Not implemented     |
| `GET /api/v1/activities/poll`                  | `POST /api/v1/workers/poll`                    | Wrong method & path |
| `POST /api/v1/activities/{id}/start`           | -                                              | Not implemented     |
| `WS /api/v1/ws/activities/{id}`                | `GET /api/v1/activities/{id}/ws`               | Wrong path format   |
| `POST /api/v1/workflows/{id}/artifacts`        | Files via `/activities/{key}/files/{filename}` | Different pattern   |
| `GET /api/v1/workflows/{id}/artifacts/{key}`   | -                                              | Not implemented     |
| `HEAD /api/v1/workflows/{id}/artifacts/{key}`  | -                                              | Not implemented     |
| `DELETE /api/v1/workflows/{id}/artifacts/{key}`| -                                              | Not implemented     |
| `GET /api/v1/workflows/{id}/artifacts`         | Files via `/activities/{key}/output`           | Different pattern   |

---

## Well-Documented Routes

These routes have adequate documentation:

| Route                                                            | Doc Location               |
|------------------------------------------------------------------|----------------------------|
| `GET /health`, `GET /health/ready`                               | quickstart.md, US-1A.1     |
| `POST /api/v1/oauth/token`                                       | US-1A.3, quickstart.md     |
| `POST /api/v1/workflows`, `GET /api/v1/workflows`                | quickstart.md, architecture.md |
| `POST/GET /api/v1/workflow_definitions`                          | mvp-requirements.md        |
| Cost endpoints (`/cost`, `/cost/history`, `/cost/analytics`)     | cost-dashboard-api.md      |
| LLM catalog (`/llm/providers`, `/llm/models/search`)             | cost-dashboard-api.md      |
| Worker APIs (`/workers/poll`, `/heartbeat`, `/complete`, `/fail`)| US-1A.7                    |
| Output retrieval (`/output`, `/activities/{key}/output`)         | US-1A.8                    |
| WebSocket streaming (`/activities/{id}/ws`)                      | US-7.1-token-streaming.md  |

---

## Recommendations

### 1. Create dedicated API reference doc

Create `docs/api-reference.md` as a single authoritative reference for all public API routes. Currently, documentation is scattered across implementation docs (US-*) which are detailed but not user-facing.

### 2. Fix architecture.md route discrepancies

Update the following in `docs/architecture.md`:

- `/api/v1/auth/token` should be `/api/v1/oauth/token`
- `GET /api/v1/activities/poll` should be `POST /api/v1/workers/poll`
- `WS /api/v1/ws/activities/{id}` should be `GET /api/v1/activities/{id}/ws`

### 3. Document missing cache invalidation endpoints

The cache endpoints are useful for operators and should be documented:

```
DELETE /api/v1/cache/:key
POST /api/v1/cache/invalidate
```

### 4. Clarify internal vs public APIs

Streaming endpoints used by workers to publish tokens are internal:

- `POST /api/v1/activities/:id/ws/token`
- `POST /api/v1/activities/:id/ws/complete`
- `POST /api/v1/activities/:id/ws/error`
- `GET /api/v1/activities/:id/ws/subscribers`

These should be marked as internal or excluded from public API docs.

### 5. Remove or mark as post-MVP

The following documented endpoints are not implemented:

- `POST /api/v1/auth/refresh` - token refresh
- `POST /api/v1/activities/{id}/start` - explicit activity claim
- Artifact endpoints (`/artifacts/`) - different implementation exists via `/files/`

---

## Actual API Routes (Reference)

### Public Routes (No Authentication)

| Method | Path                              | Handler                   |
|--------|-----------------------------------|---------------------------|
| GET    | /health                           | liveness_handler          |
| GET    | /health/ready                     | readiness_handler         |
| GET    | /health/pool                      | pool_metrics_handler      |
| GET    | /api/v1/info                      | service_info_handler      |
| POST   | /api/v1/oauth/token               | token_handler             |
| GET    | /api/v1/activities/:id/ws         | activity_stream_handler   |
| GET    | /api/v1/docs                      | ReDoc UI                  |
| GET    | /api/v1/openapi.json              | OpenAPI spec              |

### Protected Routes (Require JWT Bearer Token)

| Method | Path                                                      | Handler                      |
|--------|-----------------------------------------------------------|------------------------------|
| POST   | /api/v1/workflow_definitions                              | deploy_workflow_definition   |
| GET    | /api/v1/workflow_definitions                              | list_workflow_definitions    |
| GET    | /api/v1/workflow_definitions/:name                        | get_workflow_definition      |
| POST   | /api/v1/workflows                                         | submit_workflow              |
| GET    | /api/v1/workflows                                         | list_workflows               |
| GET    | /api/v1/workflows/:workflow_id                            | get_workflow                 |
| GET    | /api/v1/workflows/:workflow_id/cost                       | get_workflow_cost            |
| GET    | /api/v1/workflows/:workflow_id/cost/history               | get_workflow_cost_history    |
| GET    | /api/v1/cost/analytics                                    | get_cost_analytics           |
| GET    | /api/v1/workflows/:workflow_id/output                     | get_workflow_output          |
| GET    | /api/v1/workflows/:id/activities/:key/output              | get_activity_output          |
| GET    | /api/v1/workflows/:id/activities/:key/files/:filename     | download_activity_file       |
| GET    | /api/v1/llm/providers                                     | list_providers               |
| POST   | /api/v1/llm/models/search                                 | search_models                |
| DELETE | /api/v1/cache/:key                                        | invalidate_cache_key         |
| POST   | /api/v1/cache/invalidate                                  | invalidate_cache_pattern     |
| POST   | /api/v1/workers/poll                                      | poll_activities              |
| POST   | /api/v1/activities/:id/heartbeat                          | heartbeat_activity           |
| POST   | /api/v1/activities/:id/complete                           | complete_activity            |
| POST   | /api/v1/activities/:id/fail                               | fail_activity                |
| POST   | /api/v1/activities/:id/ws/token                           | publish_stream_token         |
| POST   | /api/v1/activities/:id/ws/complete                        | publish_stream_complete      |
| POST   | /api/v1/activities/:id/ws/error                           | publish_stream_error         |
| GET    | /api/v1/activities/:id/ws/subscribers                     | get_subscriber_count         |

---

## Source Files

- Route definitions: `api/src/routes.rs`
- Handler exports: `api/src/handlers/mod.rs`

---

## Resolution

**Resolved 2026-01-14**

### Changes Made

1. **Updated `api/src/openapi.rs`** - Added missing endpoint to utoipa OpenAPI spec:
   - Added `pool_metrics_handler` to paths (GET /health/pool)
   - Added `PoolMetricsResponse` to schemas

2. **Created `docs/api-reference.md`** - Comprehensive API reference documenting all 32 routes with request/response examples

3. **Fixed `docs/architecture.md` discrepancies**:
   - `/api/v1/auth/token` → `/api/v1/oauth/token` (4 occurrences)
   - `GET /api/v1/activities/poll` → `POST /api/v1/workers/poll` (3 occurrences)
   - `WS /api/v1/ws/activities/{id}` → `GET /api/v1/activities/{id}/ws` (4 occurrences)
   - Removed unimplemented `POST /api/v1/auth/refresh`
   - Removed unimplemented `POST /api/v1/activities/{id}/start`
   - Updated artifact section to reflect actual file system API

4. **Updated `docs/SUMMARY.md`** - Added API Reference to documentation index

### Notes

- Internal streaming endpoints (`/ws/token`, `/ws/complete`, `/ws/error`, `/ws/subscribers`) documented in API reference as internal
- Cache invalidation endpoints now documented
- All route discrepancies between docs and code resolved
