# Bug: Skipped Dependencies with False Conditions Still Block Downstream Activities

**Date**: 2026-01-08
**Status**: Consolidated
**Consolidated Into**: [2026-01-10-orchestrator-logic.md](2026-01-10-orchestrator-logic.md)
**Severity**: Critical
**Component**: Core / Orchestrator / Dependency Evaluator

> **Note**: This bug has been consolidated with two related bugs into a unified analysis and implementation plan. The root cause was identified as activity status not being exposed to template conditions. See the consolidated report for the implementation plan.

## Summary

When an activity has conditional dependencies on multiple activities (some Skipped, some Completed), the orchestrator incorrectly blocks scheduling even when:
- The Skipped activities have conditions that evaluate to false (should be "not applicable")
- The Completed activity has a condition that evaluates to true (should satisfy)

This appears to be a bug in how the dependency evaluator handles Skipped dependencies with conditions.

## Evidence from Production Workflow

Workflow `019b9d98-7648-7cc2-8547-9fba56220ea9` demonstrates this bug:

### Activity States (from `workflows.activities` JSONB):
```json
{
  "check_source": {"status": "completed", "outputs": [{"value": {"rows": [{"doi": ""}]}}]},
  "find_bibliography": {"status": "completed", "outputs": [{"value": {"rows": []}}]},
  "mark_no_citations": {"status": "completed"},
  "store_references_openalex": {"status": "skipped"},
  "store_references_llm": {"status": "skipped"},
  "finalize": {"status": "not_scheduled"}  // BUG: should be scheduled!
}
```

### Event Processing Confirmed:
All events were processed by orchestrator (verified via `workflow_event_consumers.last_event_id`):
```sql
SELECT event_type, activity_key, processed FROM workflow_events
WHERE workflow_id = '019b9d98-7648-7cc2-8547-9fba56220ea9';

-- All events show processed = true, including:
-- ActivityCompleted | mark_no_citations | t
```

### Finalize Dependencies:
```yaml
depends_on:
  - activity_key: store_references_openalex
    condition: "{{check_source.result.rows[0].doi | length > 0}}"  # FALSE (doi="")
  - activity_key: store_references_llm
    condition: "{{...find_bibliography.result...rows...length > 0}}"  # FALSE (rows=[])
  - activity_key: mark_no_citations
    condition: "{{...doi...length == 0 and ...rows...length == 0}}"  # TRUE
```

### Expected Behavior:
Per the semantic table in `2026-01-06-skipped-not-treated-as-terminal-state.md`:

| Dependency Status | Has Condition | Condition Value | Result |
|-------------------|---------------|-----------------|--------|
| Skipped | Yes | false | Not applicable |
| Completed | Yes | true | Applicable, satisfied |

All three dependencies should be satisfied:
1. `store_references_openalex`: Skipped + condition=false → Not applicable
2. `store_references_llm`: Skipped + condition=false → Not applicable
3. `mark_no_citations`: Completed + condition=true → Satisfied

**Result: `finalize` should be scheduled!**

### Actual Behavior:
- `finalize` remains `not_scheduled`
- Workflow times out after 300s
- `finalize` only runs after WorkflowFailed timeout event

## Root Cause Hypothesis

The dependency evaluator may have one of these bugs:

1. **Early termination on Skipped**: When iterating through dependencies, if any is Skipped, evaluation stops before checking conditions
2. **Condition evaluation failure**: Conditions on Skipped dependencies may fail to evaluate (template error) and default to "blocking"
3. **AND vs OR logic error**: All dependencies must be "satisfied or not applicable", but the logic may require "at least one satisfied"

## Impact

- Workflows with converging paths from mutually exclusive branches always time out
- Common patterns like "finalize after whichever path completes" are broken
- The documented workaround (conditional dependencies) doesn't work

## Test Case

```yaml
activities:
  - key: check
    # Determines which path to take

  - key: path_a
    depends_on:
      - activity_key: check
        condition: "{{check.result.value == 'A'}}"

  - key: path_b
    depends_on:
      - activity_key: check
        condition: "{{check.result.value == 'B'}}"

  - key: finalize
    depends_on:
      - activity_key: path_a
        condition: "{{check.result.value == 'A'}}"
      - activity_key: path_b
        condition: "{{check.result.value == 'B'}}"

# When check.result.value = 'B':
# - path_a is Skipped, condition false → not applicable
# - path_b is Completed, condition true → satisfied
# - finalize SHOULD run but DOES NOT
```

## Database Queries for Debugging

```sql
-- Check activity states
SELECT jsonb_pretty(activities)
FROM workflows
WHERE id = '<workflow_id>';

-- Verify events were processed
SELECT we.event_type, we.activity_key,
       (SELECT last_event_id FROM workflow_event_consumers WHERE consumer_id = 'orchestrator') > we.id as processed
FROM workflow_events we
WHERE we.workflow_id = '<workflow_id>'
ORDER BY we.id;
```

## Related Issues

- 2026-01-06-skipped-not-treated-as-terminal-state.md (defines semantic table)
- 2026-01-08-unconditional-deps-on-exclusive-paths.md (similar symptoms, different root cause)
- 2026-01-08-condition-eval-on-skipped-dependents.md (may be related)
