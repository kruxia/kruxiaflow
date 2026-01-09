# Feature Request: Workflow Chaining

**Date**: 2026-01-08
**Status**: Proposed
**Priority**: High
**Component**: Core / Orchestrator / Workflow Definitions

## Summary

Add the ability to automatically trigger follow-up workflows when a workflow completes. This enables multi-stage processing pipelines without application-level orchestration or monolithic mega-workflows.

## Motivation

Real-world document processing pipelines typically require multiple sequential stages:

1. **Ingest**: Extract content, generate embeddings, store passages
2. **Enrich**: Resolve bibliographic metadata from external APIs (DOI.org, CrossRef, OpenAlex)
3. **Extract Citations**: Parse bibliography section, create citation records
4. **Resolve Citations**: Match extracted citations to external databases

Currently, these must be implemented as:
- **Separate workflows** triggered manually or via application-level polling (complex, error-prone)
- **Monolithic mega-workflows** combining all activities (duplicates definitions, hard to maintain, can't run individual stages)

Neither approach is ideal. Workflow chaining would provide:
- **Composability**: Build complex pipelines from simple, reusable workflows
- **Maintainability**: Each workflow is self-contained and independently testable
- **Flexibility**: Run individual stages or the full pipeline
- **Reliability**: Built-in error handling and retry semantics for the full chain

## Proposed Syntax

### Option A: `triggers` Section (Recommended)

Add a `triggers` section to workflow definitions that specifies follow-up workflows:

```yaml
name: ingest_source
namespace: researcher
description: Extract content and generate embeddings

triggers:
  on_complete:
    # Trigger enrich_bibliographic when ingestion succeeds
    - workflow: enrich_bibliographic
      condition: "{{finalize.result.rows[0].status == 'ready'}}"
      input:
        source_id: "{{INPUT.source_id}}"
        db_url: "{{INPUT.db_url}}"

    # Trigger extract_citations when ingestion succeeds
    - workflow: extract_citations
      condition: "{{finalize.result.rows[0].passage_count > 0}}"
      input:
        source_id: "{{INPUT.source_id}}"
        db_url: "{{INPUT.db_url}}"

  on_fail:
    # Optional: trigger error handling workflow
    - workflow: notify_failure
      input:
        source_id: "{{INPUT.source_id}}"
        error: "{{ERROR}}"

activities:
  # ... existing activities
```

### Option B: `chain` Section (Alternative)

Simpler syntax for linear pipelines:

```yaml
name: process_source_pipeline
namespace: researcher
description: Full document processing pipeline

chain:
  - workflow: ingest_source
    input:
      source_id: "{{INPUT.source_id}}"
      file_url: "{{INPUT.file_url}}"
      type: "{{INPUT.type}}"
      metadata: "{{INPUT.metadata}}"
      db_url: "{{INPUT.db_url}}"

  - workflow: enrich_bibliographic
    condition: "{{PREV.finalize.result.rows[0].status == 'ready'}}"
    input:
      source_id: "{{INPUT.source_id}}"
      db_url: "{{INPUT.db_url}}"

  - workflow: extract_citations
    condition: "{{PREV.finalize.result.rows[0].passage_count > 0}}"
    input:
      source_id: "{{INPUT.source_id}}"
      db_url: "{{INPUT.db_url}}"
```

### Option C: Webhook Callbacks (Simpler Implementation)

Add webhook support as a lower-level primitive:

```yaml
name: ingest_source
namespace: researcher

settings:
  callbacks:
    on_complete:
      url: "{{INPUT.callback_url}}"
      headers:
        Authorization: "Bearer {{INPUT.callback_token}}"
      body:
        workflow_id: "{{WORKFLOW_ID}}"
        status: "{{STATUS}}"
        outputs: "{{OUTPUTS}}"
```

This would allow applications to implement their own chaining logic.

## Semantic Considerations

### Error Handling

When a chained workflow fails:
1. **Default**: Mark parent workflow as completed (chained workflow failure is independent)
2. **`propagate_failure: true`**: Mark parent as failed if any chained workflow fails
3. **`wait_for_completion: true`**: Parent workflow stays "running" until all chained workflows complete

### Parallel vs Sequential Chains

```yaml
triggers:
  on_complete:
    # These run in parallel (default)
    - workflow: enrich_bibliographic
      ...
    - workflow: extract_citations
      ...

    # Or specify sequential execution
    - workflow: enrich_bibliographic
      ...
    - workflow: extract_citations
      depends_on: enrich_bibliographic
      ...
```

### Input Mapping

Chained workflows need access to:
- `{{INPUT.*}}` - Original workflow input
- `{{OUTPUTS.*}}` or `{{activity_key.result.*}}` - Activity outputs from parent workflow
- `{{WORKFLOW_ID}}` - Parent workflow ID for correlation
- `{{PREV.*}}` - Previous workflow outputs (for Option B chain syntax)

### Idempotency

Chained workflow triggers should include correlation keys to prevent duplicate triggers:

```yaml
triggers:
  on_complete:
    - workflow: enrich_bibliographic
      unique_key: "enrich_{{INPUT.source_id}}_from_{{WORKFLOW_ID}}"
```

## Implementation Notes

### Database Schema

New tables or columns to track workflow relationships:

```sql
-- Option 1: Track parent-child relationship
ALTER TABLE workflows ADD COLUMN parent_workflow_id UUID REFERENCES workflows(id);
ALTER TABLE workflows ADD COLUMN triggered_by_event TEXT; -- 'on_complete', 'on_fail', 'manual'

-- Option 2: Separate relationship table
CREATE TABLE workflow_chains (
    id UUID PRIMARY KEY,
    parent_workflow_id UUID NOT NULL REFERENCES workflows(id),
    child_workflow_id UUID NOT NULL REFERENCES workflows(id),
    trigger_type TEXT NOT NULL, -- 'on_complete', 'on_fail'
    triggered_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Event Processing

After emitting `WorkflowCompleted` or `WorkflowFailed`:
1. Load workflow definition
2. Check `triggers` section for matching conditions
3. Evaluate conditions using final workflow state
4. Submit new workflow(s) with mapped inputs
5. Record parent-child relationship

### API Changes

```bash
# Get workflow with children
GET /api/v1/workflows/{id}?include=children

# Response includes triggered workflows
{
  "workflow_id": "...",
  "status": "completed",
  "children": [
    {"workflow_id": "...", "definition_name": "enrich_bibliographic", "status": "running"},
    {"workflow_id": "...", "definition_name": "extract_citations", "status": "completed"}
  ]
}

# Cancel workflow cascade
DELETE /api/v1/workflows/{id}?cascade=true
```

## Use Cases

### 1. Document Processing Pipeline (Primary Use Case)

```
ingest_source
    ├── enrich_bibliographic (parallel)
    └── extract_citations (parallel)
            └── resolve_citations (after extraction completes)
```

### 2. ETL Pipelines

```
extract_data
    └── transform_data
            └── load_data
```

### 3. Multi-Environment Deployment

```
deploy_staging
    └── run_integration_tests
            └── deploy_production (on test success)
```

### 4. Data Quality Pipeline

```
ingest_data
    └── validate_schema
            ├── quarantine_invalid (on validation failure)
            └── process_valid (on validation success)
```

## Alternatives Considered

### 1. Nested Workflows (Rejected)

Calling workflows as activities within another workflow:

```yaml
activities:
  - key: ingest
    worker: kruxiaflow
    activity_name: workflow.run
    parameters:
      workflow: ingest_source
      input: ...
```

**Problems:**
- Requires kruxiaflow bugs to be fixed (nested workflow evaluation)
- Complex error handling semantics
- Parent workflow stays running until all nested workflows complete

### 2. Application-Level Orchestration (Current Workaround)

Application polls workflow status and triggers next workflow:

```python
while True:
    status = get_workflow_status(workflow_id)
    if status == 'completed':
        trigger_next_workflow(...)
        break
    sleep(5)
```

**Problems:**
- Polling overhead
- Application must handle failures, retries
- Not declarative, hard to understand pipeline structure
- Logic duplicated across applications

### 3. Monolithic Mega-Workflows (Current Workaround)

Combine all activities into a single workflow:

**Problems:**
- Duplicates activity definitions
- Can't run individual stages
- Hard to test and maintain
- Violates single responsibility principle

## Related Issues

- Nested workflow bugs (various in docs/bugs/)
- Webhook support (not yet implemented)

## Timeline Estimate

- **Phase 1** (2-3 days): Webhook callbacks (`on_complete` URL)
- **Phase 2** (3-5 days): Basic workflow chaining (`triggers.on_complete`)
- **Phase 3** (2-3 days): Parallel/sequential chains, error propagation
- **Phase 4** (2-3 days): API for viewing workflow trees, cascade cancel
