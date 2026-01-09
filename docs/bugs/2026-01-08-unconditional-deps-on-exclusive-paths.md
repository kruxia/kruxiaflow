# Bug: Unconditional Dependencies on Mutually Exclusive Activities Cause Hang

**Date**: 2026-01-08
**Status**: Open
**Severity**: High
**Component**: Core / Orchestrator / Dependency Evaluator

## Summary

When an activity has unconditional `depends_on` entries for multiple mutually exclusive terminal activities (where only one path will execute), the workflow hangs waiting for skipped activities that will never complete.

## Symptoms

```yaml
activities:
  # Three mutually exclusive terminal activities
  - key: store_references_openalex
    # Runs on OpenAlex path (source has DOI)

  - key: store_references_llm
    # Runs on LLM path with bibliography

  - key: mark_no_citations
    # Runs on LLM path without bibliography

  # Finalize should run after ANY of the above complete
  - key: finalize
    depends_on:
      - store_references_openalex
      - store_references_llm
      - mark_no_citations
```

Expected behavior:
- Only one of the three paths runs (mutually exclusive)
- `finalize` runs after whichever terminal activity completes
- Skipped activities are treated as "satisfied" or "not applicable"

Actual behavior:
- One path runs (e.g., `store_references_openalex` completes)
- `finalize` waits for ALL dependencies including skipped ones
- Workflow hangs until 300s timeout:
  ```
  Found 1 stuck workflows (running > 300s), timing out
  Timing out workflow 019b9da7-ac3a-73c2-b2af-8358a5262bff (extract_citations)
  ```

## Root Cause

The dependency evaluator uses AND semantics for `depends_on` lists: ALL dependencies must be satisfied. There's no way to express OR semantics: "run after ANY of these complete".

When combined with the fix for treating Skipped as terminal, unconditional dependencies on skipped activities still block because `Skipped` status with no condition is treated as "applicable but blocking" (see semantic table in 2026-01-06-skipped-not-treated-as-terminal-state.md).

## Impact

- Cannot implement workflows with mutually exclusive paths that converge
- Common patterns like "finalize after whichever extraction method completes" fail
- Users must add redundant conditional dependencies to make paths explicit

## Suggested Fix

Option 1: Add explicit OR semantics
```yaml
depends_on:
  any_of:  # New semantic - run when ANY dependency is satisfied
    - store_references_openalex
    - store_references_llm
    - mark_no_citations
```

Option 2: Treat Skipped as "satisfied" for unconditional dependencies
- If dependency is Skipped and has no condition, treat as satisfied (not blocking)
- This makes sense because: if the activity was skipped, it will never produce results, so waiting is pointless

Option 3: Document that workflows with converging exclusive paths must use conditional dependencies

## Workaround

Use conditional dependencies that mirror the path selection logic:

```yaml
depends_on:
  - activity_key: store_references_openalex
    condition: "{{check_source.result.rows[0].doi | length > 0}}"
  - activity_key: store_references_llm
    condition: "{{check_source.result.rows[0].doi | length == 0 and (find_bibliography.result.rows | length > 0)}}"
  - activity_key: mark_no_citations
    condition: "{{check_source.result.rows[0].doi | length == 0 and (find_bibliography.result.rows | length == 0)}}"
```

This requires duplicating the path selection logic and using defensive templates (see related bug).

## Test Cases Needed

1. Activity with unconditional deps on 3 mutually exclusive activities
2. Activity with mixed conditional/unconditional deps where some are skipped
3. Converging diamond pattern: A → (B | C) → D where D depends on both B and C

## Related Issues

- 2026-01-06-skipped-not-treated-as-terminal-state.md (Skipped now treated as terminal, but semantic question remains)
- 2026-01-08-condition-eval-on-skipped-dependents.md (template errors when using conditional workaround)
