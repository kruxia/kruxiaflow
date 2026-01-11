# Bug: Workflow Secrets Not Loaded from Environment Variables

**Date**: 2026-01-04
**Status**: Resolved
**Severity**: High
**Component**: Orchestrator / Template Context
**Resolution Date**: 2026-01-06

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

## Resolution

### Implementation

1. **Added `secrets` field to `OrchestratorConfig`** (`core/src/orchestrator/config.rs`):
   - New `secrets: HashMap<String, String>` field
   - `with_secrets()` builder method for passing secrets directly
   - `with_secrets_from_env()` builder method to load from environment

2. **Added `load_secrets_from_env()` function** (`core/src/orchestrator/config.rs:79-86`):
   ```rust
   pub fn load_secrets_from_env() -> HashMap<String, String> {
       std::env::vars()
           .filter_map(|(key, value)| {
               key.strip_prefix("KRUXIAFLOW_SECRET_")
                   .map(|suffix| (suffix.to_lowercase(), value))
           })
           .collect()
   }
   ```

3. **Updated `build_template_context()`** (`core/src/orchestrator/orchestrator.rs:292-336`):
   - Added `secrets: &HashMap<String, String>` parameter
   - Calls `context.with_secrets(secrets.clone())` to add secrets to template context

4. **Updated call sites** to pass secrets from config:
   - `handle_activity_failed()` - added secrets parameter
   - Main event processing loop passes `&config.secrets`

### Usage

Set environment variables with `KRUXIAFLOW_SECRET_` prefix:

```bash
export KRUXIAFLOW_SECRET_DB_URL="postgres://user:pass@localhost/db"
export KRUXIAFLOW_SECRET_API_KEY="sk-12345"
```

Initialize orchestrator config with secrets:

```rust
let config = OrchestratorConfig::new(pool)
    .with_secrets_from_env();
```

Access in workflow templates:

```yaml
parameters:
  db_url: "{{SECRET.db_url}}"
  api_key: "{{SECRET.api_key}}"
```

### Files Changed

- `core/src/orchestrator/config.rs`: Added secrets field, helper functions, and tests
- `core/src/orchestrator/orchestrator.rs`: Updated `build_template_context()` and call sites
- `core/tests/orchestrator_loop_tests.rs`: Updated test configs with secrets field

### Tests Added

**Environment Variable Loading (core/src/orchestrator/config.rs):**
- `test_load_secrets_from_env` - Verifies basic environment variable loading with prefix stripping and lowercase conversion
- `test_load_secrets_from_env_empty` - Verifies empty environment returns empty HashMap
- `test_load_secrets_preserves_underscores_in_name` - Verifies underscores in secret names are preserved after prefix stripping
- `test_load_secrets_preserves_special_characters_in_value` - Verifies special characters in values are preserved
- `test_load_secrets_handles_empty_value` - Verifies empty string values are preserved
- `test_load_secrets_case_insensitivity` - Verifies keys are lowercased regardless of original case
- `test_load_secrets_prefix_only_not_included` - Verifies behavior with prefix only (no suffix)
- `test_load_secrets_similar_prefix_not_matched` - Verifies similar but incorrect prefixes don't match
- `test_load_secrets_url_with_credentials` - Verifies database URLs with credentials work correctly
- `test_load_secrets_json_value` - Verifies JSON values (e.g., service account keys) are preserved

**Template Resolution (core/src/workflow/template.rs):**
- `test_resolve_secret_template` - Original test for basic secret template resolution
- `test_secrets_with_builder_pattern` - Verifies with_secrets() builder method
- `test_secrets_multiple_in_one_template` - Verifies multiple secrets in one template
- `test_secrets_as_whole_value` - Verifies secrets as entire template value
- `test_secrets_with_special_characters` - Verifies special characters in secret values
- `test_secrets_in_object` - Verifies secrets in JSON object values
- `test_secrets_missing_returns_null` - Verifies missing secrets return null (not error)
- `test_secrets_undefined_top_level_fails` - Verifies undefined top-level context fails in strict mode
- `test_secrets_empty_string_value` - Verifies empty string secrets are preserved
- `test_secrets_combined_with_inputs` - Verifies secrets and inputs work together
- `test_secrets_with_filter` - Verifies secrets work with minijinja filters
- `test_secrets_default_filter_for_missing` - Verifies default filter provides fallback
- `test_secret_context_is_object` - Verifies SECRET is exposed as an object
