# Workflow Storage HTTP API for External Workers

**Date**: 2026-01-09
**Status**: Proposed
**Priority**: Medium

## Problem Statement

External workers (like researcher-worker) that poll kruxiaflow for activities don't have direct access to workflow storage. They can only pass data via activity parameters, which are stored in the PostgreSQL event/queue tables.

This creates problems for large data:
- Activity parameters are serialized as JSON in database rows
- Large parameters bloat the event log and queue tables
- Memory spikes when serializing/deserializing large JSON

Current workaround: The embedding activity (builtin worker) streams embeddings to workflow storage, and external workers download via the existing `GET /api/v1/workflows/{id}/activities/{key}/files/{filename}` endpoint.

However, external workers cannot **upload** to workflow storage, so they must return large outputs (like extracted passages) as parameters.

## Proposed Solution

Add HTTP endpoints for external workers to upload files to workflow storage:

### Upload Endpoint

```
POST /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
Content-Type: application/octet-stream (or application/x-ndjson for JSONL)

<streaming body>
```

Response:
```json
{
  "workflow_id": "uuid",
  "activity_key": "string",
  "filename": "string",
  "size": 12345,
  "content_type": "application/x-ndjson"
}
```

### Authentication

Use existing OAuth bearer token (same as other worker endpoints).

### Streaming Upload

Support chunked transfer encoding for streaming uploads without buffering entire file in memory.

## Use Cases

### 1. PDF Extraction (researcher-worker)

Currently:
```yaml
- key: extract_content
  outputs:
    - passages  # Large array in parameters (~5MB for big docs)
```

With upload endpoint:
```yaml
- key: extract_content
  outputs:
    - passages_file  # Reference: postgres://{workflow_id}/{activity_key}/passages.jsonl
    - metadata       # Small, stays in parameters
```

### 2. Any External Worker with Large Outputs

External workers processing images, audio, or other large data could stream results to workflow storage instead of bloating parameters.

## Implementation

1. Add `POST /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}` route
2. Handler streams request body to `WorkflowStorage::upload_file()`
3. Return file metadata on success
4. External workers upload before completing activity, return file reference in output

## Compatibility

- Existing download endpoint unchanged
- Activities can return either inline data or file references
- Consumer activities handle both formats (already implemented in researcher's store_passages)

## Alternatives Considered

1. **Embed kruxiaflow-worker in external workers**: Requires major refactoring, tight coupling
2. **S3/MinIO direct upload**: Adds external dependency, requires presigned URLs
3. **Keep large data in parameters**: Current approach, causes bloat and memory issues

## Related

- Feature: Streaming Embeddings to Workflow Storage (implemented)
- Bug: Embedding OOM on Large Result Serialization (fixed)
