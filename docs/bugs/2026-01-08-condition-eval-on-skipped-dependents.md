# Bug: Template Evaluation on Skipped Activities' Dependents

**Date**: 2026-01-08
**Status**: Open
**Severity**: High
**Component**: Core / Orchestrator / Dependency Evaluator

## Summary

When an activity is skipped due to a conditional dependency evaluating to false, the orchestrator still attempts to evaluate conditions on downstream activities that reference the skipped activity's results. This causes template evaluation errors because the skipped activity's `result` is undefined.

## Symptoms

When a workflow has mutually exclusive paths with conditional dependencies:

```yaml
activities:
  - key: check_source
    worker: builtin
    activity_name: postgres_query
    # Returns: {rows: [{doi: "10.xxx/yyy"}]}  # Has DOI

  - key: update_status
    depends_on:
      - check_source

  # OpenAlex path - runs if DOI exists
  - key: fetch_openalex
    depends_on:
      - activity_key: update_status
        condition: "{{check_source.result.rows[0].doi | length > 0}}"

  # LLM path - runs if no DOI
  - key: find_bibliography
    depends_on:
      - activity_key: update_status
        condition: "{{check_source.result.rows[0].doi | length == 0}}"

  # Depends on find_bibliography with condition checking its result
  - key: parse_citations
    depends_on:
      - activity_key: find_bibliography
        condition: "{{find_bibliography.result.rows | length > 0}}"
```

When source has DOI (OpenAlex path):
1. `check_source` completes with DOI
2. `update_status` completes
3. `fetch_openalex` condition is true, starts running
4. `find_bibliography` condition is false, gets **Skipped**
5. After `fetch_openalex` completes, orchestrator evaluates conditions for next activities
6. **ERROR**: Orchestrator tries to evaluate `parse_citations` condition which references `find_bibliography.result.rows`
7. Template error: `undefined value` because `find_bibliography` was skipped

```
Template evaluation error during dependency check - failing workflow
error=Failed to evaluate condition '{{find_bibliography.result.rows | length > 0}}':
Template evaluation error: undefined value (in <expression>:1)
```

## Root Cause

The orchestrator's `find_ready_activities` function evaluates conditions for ALL activities whose dependencies are in terminal states, even if those activities are on a different execution path. When the parent activity was skipped, its `result` is undefined, causing template evaluation to fail.

The evaluation happens in `dependency_evaluator.rs` when checking if an activity's dependencies are satisfied. It evaluates the condition expression without first checking if the dependency activity actually ran (vs being skipped).

## Impact

- Workflows with mutually exclusive conditional paths fail with template errors
- Common patterns like "OpenAlex path OR LLM path" cannot be implemented
- Users must add defensive template expressions with nested `| default()` filters

## Suggested Fix

Before evaluating a condition that references a dependency's result, check if the dependency was skipped. If skipped, the condition should be treated as "not applicable" without evaluating the template.

Pseudocode:
```rust
// In dependency evaluation:
if dependency_status == Skipped {
    // Don't evaluate condition, treat as not applicable
    return DependencyResult::NotApplicable;
}
// Only evaluate condition if dependency actually ran
let condition_result = evaluate_template(condition)?;
```

## Workaround

Use defensive template expressions with nested defaults:
```yaml
condition: "{{(find_bibliography.result | default({})).rows | default([]) | length > 0}}"
```

And add a path-guard condition that uses always-available data:
```yaml
condition: "{{check_source.result.rows[0].doi | length == 0 and ((find_bibliography.result | default({})).rows | default([]) | length > 0)}}"
```

## Test Cases Needed

1. Workflow with two mutually exclusive paths where path A runs and path B is skipped
2. Activity depending on skipped activity with condition referencing skipped result
3. Workflow with three-way conditional branch
4. Nested conditional paths (A → B → C where B is skipped)

## Related Issues

- May be related to: 2026-01-06-skipped-not-treated-as-terminal-state.md
- The fix in that issue treats Skipped as terminal, but doesn't prevent condition evaluation on skipped activities' dependents
