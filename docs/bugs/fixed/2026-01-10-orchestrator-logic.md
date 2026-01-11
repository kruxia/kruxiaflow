# Consolidated Bug: Orchestrator Dependency Logic for Skipped Activities

**Date**: 2026-01-10
**Status**: Open
**Severity**: Critical
**Component**: Core / Orchestrator / Dependency Evaluator, Core / Workflow / Template

## Summary

This document consolidates three related bugs that share a common root cause: the orchestrator lacks proper handling for skipped activities in conditional dependency evaluation, and the template system doesn't expose activity status to conditions.

### Consolidated Bugs

| Original Bug | Symptom |
|--------------|---------|
| [../archived/2026-01-08-skipped-deps-with-false-condition-still-block.md](../archived/2026-01-08-skipped-deps-with-false-condition-still-block.md) | Skipped dependencies with false conditions still block downstream activities |
| [../archived/2026-01-08-condition-eval-on-skipped-dependents.md](../archived/2026-01-08-condition-eval-on-skipped-dependents.md) | Template evaluation fails when referencing skipped activity results |
| [../archived/2026-01-08-unconditional-deps-on-exclusive-paths.md](../archived/2026-01-08-unconditional-deps-on-exclusive-paths.md) | Unconditional dependencies on mutually exclusive activities cause hangs |

## Root Cause Analysis

### Problem 1: Activity Status Not Exposed to Templates

The template context (`ActivityContextInfo` in `template.rs:31-44`) exposes:
- `outputs` - Activity results
- `iteration_outputs` - Iteration-scoped outputs
- `iteration` - Current iteration number
- `accumulated_cost_usd` - Cost tracking

But it does **NOT** expose `status`. When an activity is skipped:
- Its `result` is `undefined`/`null`
- There's no way to check `activity.status == 'completed'` or `activity.status == 'skipped'`
- Conditions can't distinguish "not yet run" vs "skipped" vs "failed"

**Code location**: `template.rs:284-288` - status is not included in the activity context.

### Problem 2: Condition Evaluation on Skipped Dependencies

In `dependency_evaluator.rs:258-280`, conditions are evaluated for ALL dependencies in terminal state, including skipped ones. When a condition references a skipped activity's result:

```yaml
condition: "{{find_bibliography.result.rows | length > 0}}"
```

If `find_bibliography` was skipped, `result` is undefined, causing a MiniJinja template error.

**Current code flow** (lines 258-280):
1. Check if conditions exist
2. Evaluate ALL conditions via MiniJinja
3. If any fails → skip this dependency

The problem: Step 2 happens even for skipped dependencies, and the template evaluation fails.

### Problem 3: Semantic Table Clarification

The correct semantic table for dependency evaluation:

| Dependency Status | Has Condition | Condition Value | Result                    |
|-------------------|---------------|-----------------|---------------------------|
| Completed         | No            | N/A             | Satisfied                 |
| Completed         | Yes           | true            | Satisfied                 |
| Completed         | Yes           | false           | Not applicable            |
| Skipped           | No            | N/A             | **Cascades skip**         |
| Skipped           | Yes           | true            | Not applicable            |
| Skipped           | Yes           | false           | Not applicable            |
| Failed            | No            | N/A             | **Cascades skip**         |
| Failed            | Yes           | true            | Satisfied                 |
| Failed            | Yes           | false           | Not applicable            |

**Key semantic**: When activity B has an unconditional dependency on activity A that is Failed or Skipped, activity B should be **canceled** (marked as Skipped), not blocked forever.

The current implementation (lines 284-298) correctly returns `Ok(false)` for non-Completed deps:

```rust
} else {
    // No explicit conditions - this dependency is always applicable
    found_applicable_dependency = true;

    // Dependency activity must be Completed (not just Failed/Skipped)
    if dependency_state.status != WorkflowActivityStatus::Completed {
        return Ok(false); // Activity not ready - will be caught by find_skipped_activities
    }
}
```

This is then caught by `find_skipped_activities()` which marks activities as Skipped when they have all-terminal dependencies but cannot run. This **cascades** the skip/fail state through the dependency chain.

## Impact

- Workflows with mutually exclusive paths that converge (common pattern) either hang or fail with template errors
- Users must write complex defensive conditions with nested `| default()` filters
- Workarounds are fragile and require duplicating path-selection logic

## Solution Design

### Part 1: Expose Activity Status in Template Context

Add `status` to `ActivityContextInfo` and the template context, making expressions like these possible:

```yaml
condition: "{{store_references_openalex.status == 'completed'}}"
condition: "{{find_bibliography.status != 'skipped'}}"
```

**Files to modify**:
- `core/src/workflow/template.rs` - Add status to `ActivityContextInfo` and context building
- `core/src/orchestrator/dependency_evaluator.rs` - Pass status in `add_activity_state()`

### Part 2: Conditions Must Check Status Before Accessing Results

**Problem 2 is solved by Part 1.** Once activity status is exposed in the template context, conditions that reference activity results should **first check the activity's status**. This is a user-facing pattern, not a special code path in the evaluator.

**Pattern**: Always check status before accessing result fields:

```yaml
# BAD: Template error if find_bibliography was skipped
condition: "{{find_bibliography.result.rows | length > 0}}"

# GOOD: Short-circuits on 'and' - second part not evaluated if status != completed
condition: "{{find_bibliography.status == 'completed' and find_bibliography.result.rows | length > 0}}"

# SIMPLEST: Just check status if you don't need to inspect results
condition: "{{find_bibliography.status == 'completed'}}"
```

**How it works**:
- MiniJinja's `and` operator short-circuits: if the left side is false, the right side is never evaluated
- `status == 'completed'` is false for skipped/failed/pending activities
- The `result` access never happens for non-completed activities
- No template error occurs because the undefined `result` is never accessed

**No special evaluator changes needed** - this leverages standard template short-circuit evaluation.

### Part 3: Unconditional Dependency Cascade Behavior

**Unconditional dependencies cascade skip/fail state.** When an activity has an unconditional dependency on a Failed or Skipped activity, the dependent activity is automatically marked as Skipped.

**Rationale**: This provides intuitive "fail-fast" behavior:
- If A fails, all activities that unconditionally depend on A are canceled
- If A is skipped (e.g., condition was false), downstream unconditional deps are also skipped
- This cascades through the entire dependency chain
- Prevents workflows from hanging with activities stuck in NotScheduled

**Semantic equivalence**:
- `depends_on: [activity_a]` means "run only if activity_a completed successfully"
- If activity_a is Failed/Skipped → this activity is Skipped (cascaded)

**User-facing pattern for converging exclusive paths**:

```yaml
# Finalize runs after whichever terminal activity completes
- key: finalize
  depends_on:
    - activity_key: store_references_openalex
      condition: "{{store_references_openalex.status == 'completed'}}"
    - activity_key: store_references_llm
      condition: "{{store_references_llm.status == 'completed'}}"
    - activity_key: mark_no_citations
      condition: "{{mark_no_citations.status == 'completed'}}"
```

This pattern uses explicit status conditions to say "run after whichever path completes" rather than relying on unconditional dependencies.

## Implementation Plan

### Phase 1: Expose Activity Status to Templates

1. **Modify `ActivityContextInfo`** (`template.rs:31-44`):
   ```rust
   pub struct ActivityContextInfo {
       pub outputs: Vec<ActivityOutput>,
       pub iteration_outputs: Option<HashMap<String, Vec<Value>>>,
       pub iteration: u32,
       pub accumulated_cost_usd: rust_decimal::Decimal,
       pub status: String,  // NEW: "completed", "failed", "skipped", "pending", "running", "not_scheduled"
   }
   ```

2. **Update `add_activity_state()` signature** (`template.rs`):
   Add a `status: WorkflowActivityStatus` parameter.

3. **Update context building** (`template.rs:284-288`):
   ```rust
   let activity_context = serde_json::json!({
       "iteration": activity_info.iteration,
       "accumulated_cost_usd": activity_info.accumulated_cost_usd.to_string(),
       "remaining_budget_usd": remaining_budget,
       "status": activity_info.status,  // NEW
   });
   ```

4. **Update `build_condition_context()`** (`dependency_evaluator.rs:440-450`):
   Pass `activity_state.status` to `add_activity_state()`.

### Phase 2: Document Status-First Pattern (No Code Changes)

Problem 2 is solved by Part 1. No code changes needed beyond exposing status.

1. **Document the pattern** - Conditions should check status before accessing results
2. **Update examples** - Show `{{dep.status == 'completed' and dep.result.field}}` pattern
3. **Explain short-circuit behavior** - MiniJinja's `and` prevents evaluation of right side

### Phase 3: No Changes to Unconditional Dependencies

Unconditional dependencies retain current behavior (require `Completed` status). Users express alternative semantics via explicit status conditions.

### Phase 4: Testing

Test cases needed:

1. **Status exposure in templates**:
   - Condition `{{activity.status == 'completed'}}` evaluates to true/false correctly
   - Condition `{{activity.status == 'skipped'}}` evaluates correctly
   - Condition `{{activity.status == 'failed'}}` evaluates correctly
   - All status values correctly stringified in template context

2. **Status-first pattern prevents template errors** (Bug 2):
   - Condition `{{dep.status == 'completed' and dep.result.field}}` works when dep is skipped
   - MiniJinja short-circuits: `status == 'completed'` is false → right side not evaluated
   - No template error because `result` is never accessed for skipped activities
   - Pattern documented and examples provided

3. **Status-based conditional dependencies** (Bugs 1 & 3):
   - `condition: "{{dep.status == 'completed'}}"` is true only when dep completed
   - `condition: "{{dep.status == 'completed'}}"` is false when dep is skipped → not applicable
   - Activity with status conditions on multiple deps runs when any condition is true
   - Activity is skipped when all conditional deps are not applicable

4. **Converging exclusive paths** (end-to-end):
   ```yaml
   - key: finalize
     depends_on:
       - activity_key: path_a
         condition: "{{path_a.status == 'completed'}}"
       - activity_key: path_b
         condition: "{{path_b.status == 'completed'}}"
   ```
   - When path_a runs and path_b is skipped: finalize runs after path_a
   - When path_b runs and path_a is skipped: finalize runs after path_b
   - When both are skipped: finalize is also skipped

5. **Unconditional dependencies cascade skip/fail**:
   - Unconditional `depends_on: [activity_a]` is Skipped if activity_a is Skipped
   - Unconditional `depends_on: [activity_a]` is Skipped if activity_a is Failed
   - Cascade propagates through the dependency chain

### Phase 5: Documentation

1. Update `docs/architecture.md` with:
   - Activity status available in templates
   - Semantic table for dependency evaluation
   - Best practices for conditional dependencies

2. Update workflow YAML documentation with examples:
   ```yaml
   # Preferred: explicit status check
   depends_on:
     - activity_key: path_a
       condition: "{{path_a.status == 'completed'}}"
     - activity_key: path_b
       condition: "{{path_b.status == 'completed'}}"
   ```

## Migration Considerations

This is a backwards-compatible enhancement:
- Existing workflows without `.status` conditions continue to work unchanged
- Unconditional dependency behavior is unchanged (requires `Completed`)
- No breaking changes to workflow YAML syntax
- Workflows that previously hung on skipped deps can be fixed by adding explicit status conditions

## Related Files

- `core/src/workflow/template.rs` - Template context and ActivityContextInfo
- `core/src/orchestrator/dependency_evaluator.rs` - Dependency evaluation logic
- `core/src/workflow/workflow_state.rs` - WorkflowActivityStatus enum
- `core/src/orchestrator/orchestrator.rs` - Event processing that calls evaluator

## Acceptance Criteria

- [x] `{{activity.status}}` returns correct status string in conditions
- [x] All status values correctly exposed: `completed`, `failed`, `skipped`, `pending`, `running`, `not_scheduled`
- [x] Status-first pattern `{{dep.status == 'completed' and dep.result.field}}` works for skipped deps
- [x] MiniJinja short-circuits `and` operator - right side not evaluated when left is false
- [x] Condition `{{dep.status == 'completed'}}` is true only when dep completed
- [x] Condition `{{dep.status == 'completed'}}` is false when dep is skipped (not applicable)
- [x] Unconditional dependencies on skipped activities cascade (activity is Skipped)
- [x] Workflows with converging exclusive paths work when using explicit status conditions
- [x] All existing tests pass
- [x] New tests cover status exposure and short-circuit behavior

## Status

**Fixed** - 2026-01-10

### Implementation Summary

1. **Added status field to template context** (`template.rs`):
   - Modified `add_activity_state()` to accept status parameter
   - Added status to `ActivityContextInfo` struct
   - Exposed status in both activity context and ACTIVITY context

2. **Updated context building** (`dependency_evaluator.rs`, `orchestrator.rs`):
   - Added `status_to_string()` helper function
   - Modified `build_condition_context()` to pass status
   - Modified orchestrator's context building to include status

3. **Comprehensive test coverage**:
   - 14 new tests in `template.rs` for status exposure and short-circuit behavior
   - 13 new tests in `dependency_evaluator.rs` for conditional dependencies and cascading
   - All 237 library tests pass
