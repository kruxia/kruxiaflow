# Bug: Skipped Activities Not Treated as Terminal State for Dependency Evaluation

**Date**: 2026-01-06
**Status**: Test
**Severity**: High
**Component**: Core / Orchestrator / Dependency Evaluator

## Summary

The dependency evaluator in `find_ready_activities` only considers `Completed` and `Failed` as terminal states when checking if dependencies are satisfied. This causes downstream activities with conditional dependencies to never become ready when their dependencies are in the `Skipped` state.

## Symptoms

When a workflow has conditional dependencies where some paths don't run:

```yaml
activities:
  - key: fetch_source
    worker: std
    activity_name: postgres_query
    # ...

  - key: lookup_doi_org
    worker: std
    activity_name: http_request
    depends_on:
      - activity_key: fetch_source
        condition: "{{INPUT.doi or fetch_source.result.rows[0].doi}}"
    # Only runs if DOI is available

  - key: search_openalex
    worker: std
    activity_name: http_request
    depends_on:
      - activity_key: fetch_source
        condition: "{{INPUT.title or fetch_source.result.rows[0].title}}"
    # Runs if title is available

  - key: extract_bibliography
    worker: std
    activity_name: llm_prompt
    depends_on:
      - activity_key: search_openalex
      - activity_key: lookup_doi_org
        condition: "{{INPUT.doi or fetch_source.result.rows[0].doi}}"
    # Should run after search_openalex, optionally waiting for DOI lookup
```

When there's no DOI but there is a title:
1. `fetch_source` completes
2. `lookup_doi_org` condition is false, marked as `Skipped`
3. `search_openalex` runs and completes
4. `extract_bibliography` **never becomes ready** because `lookup_doi_org` is `Skipped`, not `Completed` or `Failed`

The workflow hangs indefinitely with `extract_bibliography` stuck in `NotScheduled` state.

## Root Cause

In `core/src/orchestrator/dependency_evaluator.rs`, the terminal state check at line 241-252 only includes `Completed` and `Failed`:

```rust
// Check if dependency is in terminal state FIRST
if !matches!(
    dependency_state.status,
    WorkflowActivityStatus::Completed | WorkflowActivityStatus::Failed
) {
    // ...
    return Ok(false); // Dependency not in terminal state yet
}
```

The `Skipped` status is also a terminal state (the activity will never run), but it was not included in this check. This causes the early return when any dependency is `Skipped`, even if:
- The dependency has a condition that would make it "not applicable"
- Other applicable dependencies are satisfied

## Impact

- Workflows with conditional branches where some paths don't execute will hang
- Activities with optional conditional dependencies cannot proceed when the optional path is skipped
- This breaks common patterns like "wait for A, optionally wait for B if condition is true"

## Fix Applied

Updated the terminal state check to include `Skipped`:

```rust
// Check if dependency is in terminal state FIRST
// Terminal states: Completed, Failed, or Skipped (all are final states)
if !matches!(
    dependency_state.status,
    WorkflowActivityStatus::Completed
        | WorkflowActivityStatus::Failed
        | WorkflowActivityStatus::Skipped
) {
    // ...
    return Ok(false); // Dependency not in terminal state yet
}
```

This allows the dependency evaluation to proceed to condition checking. If the condition is false, the dependency is marked as "not applicable" and won't block the activity. If the condition is true but the dependency is `Skipped`, the activity can still proceed since the dependency has reached a terminal state.

## Files Changed

- `core/src/orchestrator/dependency_evaluator.rs`

## Test Cases Needed

1. Activity with conditional dependency where dependency is Skipped (condition false) → not applicable
2. Activity with unconditional dependency where dependency is Skipped → B is Skipped (cascaded)
3. Activity with unconditional dependency where dependency is Failed → B is Skipped (cascaded)
4. Activity with mix of Completed and Skipped dependencies (conditional) → runs if any applicable
5. Workflow with multiple conditional branches where some are taken and some are skipped
6. Cascade chain: A→B→C where A is Skipped → B and C both Skipped

## Semantic Notes

After this fix, the behavior is:

| Dep Status   | Has Cond | Cond Val | Result                     |
|--------------|----------|-------|-------------------------------|
| Completed    | No       | N/A   | Satisfied                     |
| Completed    | Yes      | true  | Satisfied                     |
| Completed    | Yes      | false | Not applicable                |
| Failed       | No       | N/A   | **Cascades skip** (B skipped) |
| Failed       | Yes      | true  | Satisfied (error handling)    |
| Failed       | Yes      | false | Not applicable                |
| Skipped      | No       | N/A   | **Cascades skip** (B skipped) |
| Skipped      | Yes      | true  | Satisfied                     |
| Skipped      | Yes      | false | Not applicable                |
| NotScheduled | Any      | Any   | Not terminal, must wait       |
| Pending      | Any      | Any   | Not terminal, must wait       |
| Running      | Any      | Any   | Not terminal, must wait       |

**Key semantic**: For unconditional dependencies, if the dependency is `Skipped` or `Failed`, the dependent activity is **canceled** (marked as Skipped), not blocked forever. This cascades through the dependency chain via `find_skipped_activities()`.

For conditional dependencies where the condition is true, the activity can proceed even if the dependency is `Skipped`.
