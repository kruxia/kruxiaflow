# Bug: Workflow Secrets Not Loaded from Environment Variables

**Date**: 2026-01-04
**Status**: Open
**Severity**: High
**Component**: Orchestrator / Template Context

## Summary

Workflow secrets declared in YAML with `secrets:` and accessed via `{{SECRET.xxx}}` template expressions are never populated. The `KRUXIAFLOW_SECRET_*` environment variables documented in docker-compose examples are not read by kruxiaflow.

## Symptoms

When a workflow uses `{{SECRET.db_url}}` (or any secret), the template evaluates to null/undefined, causing downstream errors:

```
WARN kruxiaflow_worker::poller: Activity execution failed
     activity_id=...
     error=Failed to parse PostgreSQL query parameters
```

The error occurs because `db_url: String` in `PostgresQueryParams` cannot deserialize from null.

## Root Cause

The `build_template_context` function in `core/src/orchestrator/orchestrator.rs` (lines 292-332) builds the template context with:
- Workflow inputs
- Activity outputs
- Workflow-level variables (id, status)

However, it **never populates the `secrets` field** of `TemplateContext`. The `with_secrets()` method exists in `core/src/workflow/template.rs:85` but is never called.

### Code Analysis

```rust
// core/src/orchestrator/orchestrator.rs
fn build_template_context(
    state: &super::workflow_state::WorkflowState,
    workflow_id: uuid::Uuid,
) -> TemplateContext {
    let mut context = TemplateContext::new();

    // Add inputs...
    // Add activity outputs...
    // Add workflow variables...

    // MISSING: context = context.with_secrets(loaded_secrets);

    context
}
```

The `TemplateContext` struct has a `secrets` field and the template engine correctly adds `SECRET` to the context, but it's always an empty HashMap.

## Impact

- Workflows cannot use secrets for sensitive data (database URLs, API keys)
- Users must pass secrets as workflow inputs, exposing them in workflow state/logs
- The documented `KRUXIAFLOW_SECRET_*` environment variable convention doesn't work

## Workaround

Pass sensitive values as workflow input parameters instead of secrets:

```yaml
# Instead of:
secrets:
  - db_url
# ...
db_url: "{{SECRET.db_url}}"

# Use:
input_schema:
  properties:
    db_url:
      type: string
# ...
db_url: "{{INPUT.db_url}}"
```

Then pass the value when triggering the workflow:
```json
{
  "definition_name": "my_workflow",
  "input": {
    "db_url": "postgres://..."
  }
}
```

## Proposed Fix

1. Add secret loading from environment variables in the orchestrator startup:

```rust
fn load_secrets_from_env() -> HashMap<String, String> {
    std::env::vars()
        .filter_map(|(key, value)| {
            key.strip_prefix("KRUXIAFLOW_SECRET_")
                .map(|suffix| (suffix.to_lowercase(), value))
        })
        .collect()
}
```

2. Update `build_template_context` to include secrets:

```rust
fn build_template_context(
    state: &WorkflowState,
    workflow_id: Uuid,
    secrets: &HashMap<String, String>,  // Add parameter
) -> TemplateContext {
    let mut context = TemplateContext::new();
    // ... existing code ...
    context = context.with_secrets(secrets.clone());
    context
}
```

3. Alternatively, secrets could be stored per-workflow in the database for better security.

## Files Affected

- `core/src/orchestrator/orchestrator.rs` - `build_template_context` function
- `core/src/workflow/template.rs` - Already has `with_secrets()` method

## References

- Discovered while debugging researcher project workflow failures
- Related docker-compose config: `KRUXIAFLOW_SECRET_DB_URL` environment variable
- Template context implementation: `core/src/workflow/template.rs:85-88`
