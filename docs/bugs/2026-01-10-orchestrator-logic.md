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
| [2026-01-08-skipped-deps-with-false-condition-still-block.md](2026-01-08-skipped-deps-with-false-condition-still-block.md) | Skipped dependencies with false conditions still block downstream activities |
| [2026-01-08-condition-eval-on-skipped-dependents.md](2026-01-08-condition-eval-on-skipped-dependents.md) | Template evaluation fails when referencing skipped activity results |
| [2026-01-08-unconditional-deps-on-exclusive-paths.md](2026-01-08-unconditional-deps-on-exclusive-paths.md) | Unconditional dependencies on mutually exclusive activities cause hangs |

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

### Problem 3: Semantic Table Not Fully Implemented

The intended semantic table from `2026-01-06-skipped-not-treated-as-terminal-state.md`:

| Dependency Status | Has Condition | Condition Value | Result              |
|-------------------|---------------|-----------------|---------------------|
| Completed         | No            | N/A             | Satisfied           |
| Completed         | Yes           | true            | Satisfied           |
| Completed         | Yes           | false           | Not applicable      |
| Skipped           | No            | N/A             | **Should satisfy?** |
| Skipped           | Yes           | true            | Not applicable      |
| Skipped           | Yes           | false           | Not applicable      |
| Failed            | No            | N/A             | Blocks (fail path)  |
| Failed            | Yes           | true            | Satisfied           |
| Failed            | Yes           | false           | Not applicable      |

The current implementation (lines 284-298) treats unconditional dependencies as always requiring `Completed` status:

```rust
} else {
    // No explicit conditions - this dependency is always applicable
    found_applicable_dependency = true;

    // Dependency activity must be Completed (not just Failed)
    if dependency_state.status != WorkflowActivityStatus::Completed {
        return Ok(false); // Only run dependent activities on success
    }
}
```

This means unconditional `depends_on: [store_references_openalex]` on a skipped activity blocks forever.

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

### Part 2: Short-Circuit Condition Evaluation for Skipped Dependencies

Before evaluating a condition that references a dependency, check if the dependency was skipped. If skipped, treat the dependency as "not applicable" without evaluating the template.

**Pseudocode change** in `dependency_evaluator.rs`:

```rust
// Current: lines 258-280
if let Some(condition_list) = &dep_rel.conditions {
    // NEW: Short-circuit for skipped dependencies
    if dependency_state.status == WorkflowActivityStatus::Skipped {
        // Dependency was skipped - treat as not applicable regardless of condition
        // (condition likely references undefined result anyway)
        tracing::trace!(
            "Activity {} dependency {} not applicable: dependency was skipped",
            activity.key,
            dep_rel.activity_key,
        );
        continue; // Skip this dependency, check next
    }

    // Existing condition evaluation...
    let context = build_condition_context(state);
    // ...
}
```

This prevents template errors when conditions reference skipped activity results.

### Part 3: Keep Unconditional Dependencies Simple

**No change to unconditional dependency behavior.** Unconditional dependencies continue to require `Completed` status.

**Rationale**: Keeping unconditional dependencies simple and intuitive:
- `depends_on: [activity_a]` ≡ `depends_on: [{activity_key: activity_a, condition: "{{activity_a.status == 'completed'}}"}]`
- Users who need different semantics (e.g., "run after whichever path completes") use explicit status conditions
- No implicit magic behavior that users must learn

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

This is explicit, self-documenting, and follows the principle that workflow authors should clearly express their intent.

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

### Phase 2: Short-Circuit for Skipped Dependencies

1. **Add short-circuit check** (`dependency_evaluator.rs`, before line 260):
   - If `dependency_state.status == Skipped` AND conditions exist
   - Skip condition evaluation, treat dependency as not applicable
   - Continue to next dependency

2. **Add trace logging** for this case.

### Phase 3: No Changes to Unconditional Dependencies

Unconditional dependencies retain current behavior (require `Completed` status). Users express alternative semantics via explicit status conditions.

### Phase 4: Testing

Test cases needed:

1. **Status exposure in templates**:
   - Condition `{{activity.status == 'completed'}}` evaluates to true/false correctly
   - Condition `{{activity.status == 'skipped'}}` evaluates correctly
   - Condition `{{activity.status == 'failed'}}` evaluates correctly
   - All status values correctly stringified in template context

2. **Short-circuit for skipped dependencies** (Bug 2):
   - Condition referencing skipped activity's result doesn't cause template error
   - When dependency is skipped, condition evaluation is bypassed
   - Dependency treated as "not applicable" without evaluating the condition template

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

5. **Unconditional dependencies unchanged**:
   - Unconditional `depends_on: [activity_a]` blocks if activity_a is skipped
   - Confirms no implicit behavior change for unconditional deps

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

- [ ] `{{activity.status}}` returns correct status string in conditions
- [ ] All status values correctly exposed: `completed`, `failed`, `skipped`, `pending`, `running`, `not_scheduled`
- [ ] Skipped dependencies with conditions don't cause template errors (short-circuit)
- [ ] Condition `{{dep.status == 'completed'}}` is true only when dep completed
- [ ] Condition `{{dep.status == 'completed'}}` is false when dep is skipped (not applicable)
- [ ] Unconditional dependencies on skipped activities still block (unchanged behavior)
- [ ] Workflows with converging exclusive paths work when using explicit status conditions
- [ ] All existing tests pass
- [ ] New tests cover status exposure and short-circuit behavior
