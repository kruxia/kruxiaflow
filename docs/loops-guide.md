# Kruxia Flow Loops Guide

**Version**: MVP (US-3.4)
**Last Updated**: 2025-11-21

This guide covers iterative workflows (loops) in Kruxia Flow, including patterns, best practices, and common pitfalls.

---

## Table of Contents

1. [Overview](#overview)
2. [Loop Patterns](#loop-patterns)
3. [Iteration-Scoped Storage](#iteration-scoped-storage)
4. [Template Syntax](#template-syntax)
5. [Budget Management](#budget-management)
6. [Best Practices](#best-practices)
7. [Common Pitfalls](#common-pitfalls)
8. [Examples](#examples)

---

## Overview

Loops in Kruxia Flow are created by adding a back-edge in the workflow dependency graph: a `depends_on` relationship from a later activity back to an earlier activity. This allows activities to re-execute until a condition is met or an iteration limit is reached.

### Key Concepts

- **Back-Edge**: A dependency from a later activity to an earlier activity creates a loop
- **Iteration-Scoped**: Activities that store results as arrays with a value for each iteration
- **Iteration Counter**: Track how many times a looping activity has executed
- **Budget Accumulation**: Costs accumulate across all iterations (not per-iteration)
- **Exit Mechanisms**: Loops must have at least one way to exit (condition or iteration limit)

### Loop Detection

Loops are detected during workflow validation (at registration time):

1. Workflow definition is submitted via `POST /api/v1/workflow_definitions`
2. Validator performs topological sort to identify back-edges
3. Metadata (`is_loop_activity`, `is_back_edge`) computed and cached in database
4. Orchestrator uses O(1) lookups (no runtime graph traversal needed)

---

## Loop Patterns

Kruxia Flow supports three loop patterns, each with different exit strategies:

### Pattern 1: Fixed Iterations Only

**Use When**: You know exactly how many times to loop (e.g., 12-issue newsletter subscription)

**Configuration**:
- Set `iteration_limit` on the activity
- No condition on the back-edge dependency

**Example**:
```yaml
activities:
  - key: send_monthly_issue
    iteration_scoped: true
    iteration_limit: 12          # Exactly 12 issues
    parameters:
      issue_number: "{{ACTIVITY.iteration + 1}}"
      content: "{{generate_content.text}}"
    # TODO: Schedule the activity so it repeats monthly. 
    depends_on:
      - generate_content
      - activity_key: send_monthly_issue  # Loop back unconditionally
```

**Pros**: Simple, predictable, deterministic
**Cons**: Cannot exit early based on runtime conditions

---

### Pattern 2: Conditional Only

**Use When**: Loop should continue indefinitely until a condition is met (e.g., polling, ongoing subscription)

**Configuration**:
- Set condition on the back-edge dependency
- No explicit `iteration_limit` (uses default: 100)

**Example**:
```yaml
activities:
  - key: poll_service
    iteration_scoped: false  # Only need latest status
    # No iteration_limit → uses DEFAULT_MAX_ITERATIONS (100)
    parameters:
      service_url: "{{INPUT.url}}"
    depends_on:
      - check_status
      - activity_key: check_status
        conditions:
          - "{{check_status.ready == false}}"  # Continue while not ready

  - key: check_status
    parameters:
      service_url: "{{INPUT.url}}"
    depends_on:
      - poll_service
```

**Pros**: Flexible, responds to runtime conditions
**Cons**: Risk of long-running loops (mitigated by default limit)

**Safety**: Default limit of 100 iterations prevents infinite loops. To increase:
```yaml
iteration_limit: 1000  # Custom limit for long-running workflows
```

---

### Pattern 3: Bounded Conditional (Recommended for Production)

**Use When**: You want condition-based exit with a safety bound (most production use cases)

**Configuration**:
- Set condition on the back-edge dependency
- Set explicit `iteration_limit` as safety bound

**Example**:
```yaml
activities:
  - key: perform_search
    iteration_scoped: true
    iteration_limit: 5           # Safety bound: max 5 iterations
    parameters:
      query: "{{INPUT.topic}}"
      context: "{{perform_search.results}}"
    depends_on:
      - initialize
      - activity_key: evaluate_results
        conditions:
          - "{{evaluate_results.sufficient | last == false}}"

  - key: evaluate_results
    iteration_scoped: true
    parameters:
      findings: "{{perform_search.results}}"
    outputs:
      - name: sufficient
    depends_on:
      - perform_search
```

**Pros**: Safe (bounded), flexible (condition-based), production-ready
**Cons**: Requires careful condition design

**Recommendation**: Always use Pattern 3 for production workflows to prevent runaway loops.

---

## Iteration-Scoped Storage

### What is Iteration-Scoped?

Activities marked with `iteration_scoped: true` store their outputs as arrays, with one entry per iteration. This allows downstream activities to access the entire history of results.

### Storage Format

Outputs are grouped by name as arrays:

```json
{
  "output_name": [value0, value1, value2, ...]
}
```

This format directly matches template access patterns (no transformation needed).

### When to Use Iteration-Scoped

**Use `iteration_scoped: true` when**:
- You need access to results from all iterations
- Downstream activities need to analyze trends across iterations
- You're building context (e.g., research findings from multiple searches)

**Use `iteration_scoped: false` (default) when**:
- You only need the latest result
- Storage efficiency is important (large outputs * many iterations)
- The loop is checking a simple condition (e.g., polling for readiness)

### Example: With vs Without

**With Iteration-Scoped** (stores all results):
```yaml
- key: research
  iteration_scoped: true
  iteration_limit: 5
  parameters:
    query: "{{INPUT.topic}}"
    previous_findings: "{{research.results}}"  # Array of all iterations
```

Template access:
- `{{research.results}}` → `[result0, result1, result2]`
- `{{research.results | last}}` → `result2` (latest)
- `{{research.results | length}}` → `3` (iteration count)

**Without Iteration-Scoped** (stores only latest):
```yaml
- key: poll
  iteration_scoped: false  # Default
  iteration_limit: 100
  parameters:
    url: "{{INPUT.service_url}}"
```

Template access:
- `{{poll.status}}` → `"ready"` (latest only)
- No array operations available
- Iteration counter still tracked: `{{ACTIVITY.iteration}}`

---

## Template Syntax

### Accessing Iteration Results

**All Iterations** (iteration-scoped activities only):
```yaml
"{{activity.output_name}}"  # Returns array: [val0, val1, val2, ...]
```

**Latest Iteration** (using MiniJinja `| last` filter):
```yaml
"{{activity.output_name | last}}"  # Returns: val2
```

**First Iteration**:
```yaml
"{{activity.output_name | first}}"  # Returns: val0
```

**Iteration Count**:
```yaml
"{{activity.output_name | length}}"  # Returns: 3
```

### Special Context Variables

**Current Iteration Number** (0-based):
```yaml
"{{ACTIVITY.iteration}}"  # 0, 1, 2, ...
```

**Accumulated Cost** (across all iterations):
```yaml
"{{ACTIVITY.accumulated_cost_usd}}"  # "7.50"
```

**Remaining Budget**:
```yaml
"{{ACTIVITY.remaining_budget_usd}}"  # "2.50"
```

### Conditions in Loops

**Continue While Condition is True**:
```yaml
depends_on:
  - activity_key: check
    conditions:
      - "{{check.done | last == false}}"  # Continue if not done
```

**Exit When Condition is Met**:
```yaml
depends_on:
  - activity_key: evaluate
    conditions:
      - "{{evaluate.sufficient | last == true}}"  # Exit if sufficient
```

**Multiple Conditions** (all must be true):
```yaml
depends_on:
  - activity_key: check
    conditions:
      - "{{check.ready | last == false}}"      # Not ready yet
      - "{{ACTIVITY.iteration < 10}}"           # Less than 10 iterations
      - "{{ACTIVITY.accumulated_cost_usd < 5}}" # Under $5 spent
```

---

## Budget Management

### How Budget Works with Loops

Budget limits apply to the **total accumulated cost across all iterations**, not per-iteration:

```yaml
- key: expensive_loop
  iteration_scoped: true
  iteration_limit: 10
  settings:
    budget:
      limit: 5.00    # $5 total across all iterations
      action: abort
```

- **Iteration 0**: Cost $1.50, Accumulated: $1.50 ✅️ (passes)
- **Iteration 1**: Cost $1.50, Accumulated: $3.00 ✅️ (passes)
- **Iteration 2**: Cost $1.50, Accumulated: $4.50 ✅️ (passes)
- **Iteration 3**: Cost $1.50, Accumulated: $6.00 ❌ (exceeds $5 limit → fails)

### Budget Actions

**`action: abort`** (recommended):
- Activity fails when budget exceeded
- Loop terminates
- Orchestrator marks workflow as failed

**`action: warn`**:
- Log warning but continue execution
- Budget can be exceeded
- Use for soft limits

### Budget-Aware Loop Design

**Strategy 1: Budget as Safety Bound**
```yaml
- key: research_loop
  iteration_limit: 10          # Max 10 iterations
  settings:
    budget:
      limit: 10.00             # OR max $10 spent
      action: abort
```
Loop stops at whichever limit is reached first.

**Strategy 2: Budget in Exit Condition**
```yaml
- key: research_loop
  settings:
    budget:
      limit: 10.00
      action: warn             # Don't abort, just warn
  depends_on:
    - activity_key: evaluate
      conditions:
        # Exit if sufficient OR budget running low
        - "{{evaluate.sufficient | last == true || ACTIVITY.remaining_budget_usd < 1}}"
```

---

## Best Practices

### 1. Always Provide an Exit Mechanism

❌ **Bad** (no exit mechanism):
```yaml
- key: loop_forever
  depends_on:
    - activity_key: loop_forever  # Infinite loop!
```

✅️ **Good** (Pattern 3 - condition + limit):
```yaml
- key: safe_loop
  iteration_limit: 10  # Safety bound
  depends_on:
    - activity_key: check
      conditions:
        - "{{check.done | last == false}}"  # Condition
```

### 2. Use Descriptive Iteration Counters

✅️ **Good** (user-friendly numbering):
```yaml
parameters:
  issue_number: "{{ACTIVITY.iteration + 1}}"  # 1, 2, 3 (not 0, 1, 2)
  status: "Processing iteration {{ACTIVITY.iteration + 1}} of {{settings.iteration_limit}}"
```

### 3. Set Appropriate Iteration Limits

- **Research loops**: 5-10 iterations (LLM-based research typically converges quickly)
- **Polling loops**: 100-1000 iterations (may need to wait for external services)
- **Fixed iterations**: Exact count (e.g., 12 for monthly newsletter)

### 4. Monitor Accumulated Costs

```yaml
parameters:
  prompt: |
    Current iteration: {{ACTIVITY.iteration}}
    Budget spent: ${{ACTIVITY.accumulated_cost_usd}}
    Budget remaining: ${{ACTIVITY.remaining_budget_usd}}

    (Include budget info to help LLM make cost-aware decisions)
```

### 5. Use Latest Values in Conditions

❌ **Bad** (checking entire array):
```yaml
conditions:
  - "{{check.done == false}}"  # May not work - comparing array to bool
```

✅️ **Good** (using `| last` filter):
```yaml
conditions:
  - "{{check.done | last == false}}"  # Compare latest value to bool
```

### 6. Log Iteration Progress

```yaml
parameters:
  prompt: |
    Iteration {{ACTIVITY.iteration + 1}}
    Previous findings: {{research.results | length}} iterations completed

    (Provides context for debugging and monitoring)
```

---

## Common Pitfalls

### Pitfall 1: Forgetting `| last` in Conditions

**Problem**: Iteration-scoped outputs are arrays, not single values

❌ **Wrong**:
```yaml
conditions:
  - "{{evaluate.sufficient == true}}"  # Comparing array to bool
```

✅️ **Correct**:
```yaml
conditions:
  - "{{evaluate.sufficient | last == true}}"  # Compare latest value
```

### Pitfall 2: Expecting Per-Iteration Budget Resets

**Problem**: Budget accumulates across all iterations

❌ **Wrong assumption**:
```yaml
# Each iteration costs $2, budget is $5
iteration_limit: 10  # Expecting 10 iterations? NO!
settings:
  budget:
    limit: 5.00  # Will only complete 2 iterations ($2 + $2 = $4 ≤ $5)
```

✅️ **Correct understanding**:
```yaml
# Each iteration costs $2, need 10 iterations
iteration_limit: 10
settings:
  budget:
    limit: 20.00  # $2 × 10 iterations = $20 total
```

### Pitfall 3: Mixing Iteration-Scoped and Non-Scoped Activities

**Problem**: Inconsistent array vs single-value access

```yaml
- key: search
  iteration_scoped: true  # Stores arrays

- key: evaluate
  iteration_scoped: false  # Stores single value
  depends_on:
    - activity_key: search

- key: report
  parameters:
    all_searches: "{{search.results}}"      # ✅️ Array
    latest_eval: "{{evaluate.decision}}"    # ✅️ Single value
    # NOT: "{{evaluate.decision | last}}"   # ❌ Not an array!
```

### Pitfall 4: No Safety Bound (Pattern 2 Only)

**Problem**: Relying solely on condition without iteration limit

❌ **Risky** (condition-only with default limit):
```yaml
- key: long_running_loop
  # No iteration_limit → uses DEFAULT_MAX_ITERATIONS (100)
  depends_on:
    - activity_key: check
      conditions:
        - "{{check.active | last == true}}"
  # What if check.active stays true for 200 iterations?
  # Loop stops at 100, potentially incomplete!
```

✅️ **Safer** (explicit limit):
```yaml
- key: long_running_loop
  iteration_limit: 1000  # Explicit high limit for expected behavior
  depends_on:
    - activity_key: check
      conditions:
        - "{{check.active | last == true}}"
```

### Pitfall 5: Circular Dependencies Without Back-Edge

**Problem**: Two activities depend on each other without proper loop structure

❌ **Wrong** (circular dependency):
```yaml
- key: A
  depends_on: [B]  # Needs B first

- key: B
  depends_on: [A]  # But also needs A first - DEADLOCK!
```

✅️ **Correct** (explicit back-edge):
```yaml
- key: A
  depends_on:
    - activity_key: B
      conditions: ["{{B.done | last == false}}"]
      # This is a back-edge (marked during validation)

- key: B
  depends_on: [A]  # Forward edge
```

---

## Examples

### Example 1: Agentic Research (Pattern 3)

Full example showing all loop features:

```yaml
name: agentic_research
activities:
  - key: initialize
    worker: std
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-haiku-4-20250415
      prompt: "Create a research plan for: {{INPUT.topic}}"
    outputs:
      - result

  - key: perform_search
    iteration_scoped: true       # Store all search results
    iteration_limit: 5           # Max 5 iterations (safety bound)
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        Research topic: {{INPUT.topic}}
        Research plan: {{initialize.result.content}}
        Previous findings ({{perform_search.results | length}} iterations):
        {{perform_search.results | json}}

        Current iteration: {{ACTIVITY.iteration}}
        Budget remaining: ${{ACTIVITY.remaining_budget_usd}}

        Conduct research building on previous findings.
    outputs:
      - result
    settings:
      budget:
        limit: 0.05              # $0.05 total across all iterations
        action: abort
    depends_on:
      - initialize
      - activity_key: evaluate
        conditions:
          - "{{evaluate.result.content | contains(substring='CONTINUE')}}"

  - key: evaluate
    iteration_scoped: true
    parameters:
      model: anthropic/claude-haiku-4-20250415
      prompt: |
        All findings ({{perform_search.results | length}} iterations):
        {{perform_search.results | json}}

        Evaluate if sufficient. Respond with ONLY:
        - CONTINUE (if more research needed)
        - SUFFICIENT (if ready for report)
    outputs:
      - result
    depends_on:
      - perform_search

  - key: compile_report
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        All findings from {{perform_search.results | length}} iterations:
        {{perform_search.results | json}}

        Compile comprehensive research report.
    outputs:
      - result
    depends_on:
      - activity_key: evaluate
        conditions:
          - "{{evaluate.result.content | contains(substring='SUFFICIENT')}}"
```

### Example 2: Fixed Newsletter (Pattern 1)

```yaml
name: newsletter_subscription
activities:
  - key: send_monthly_issue
    iteration_scoped: true
    iteration_limit: 12          # Exactly 12 monthly issues
    parameters:
      issue_number: "{{ACTIVITY.iteration + 1}}"
      subscriber_id: "{{INPUT.subscriber_id}}"
      content: "{{generate_content.text}}"
    depends_on:
      - generate_content
      - activity_key: send_monthly_issue  # Loop back unconditionally

  - key: generate_content
    parameters:
      month: "{{ACTIVITY.iteration + 1}}"
    depends_on:
      - send_monthly_issue
```

### Example 3: Service Polling (Pattern 2)

```yaml
name: poll_service
activities:
  - key: poll
    iteration_scoped: false      # Only need latest status
    iteration_limit: 1000        # Custom high limit
    parameters:
      url: "{{INPUT.service_url}}"
    depends_on:
      - check_status
      - activity_key: check_status
        conditions:
          - "{{check_status.ready == false}}"  # Continue while not ready

  - key: check_status
    parameters:
      url: "{{INPUT.service_url}}"
    depends_on:
      - poll

  - key: process_result
    parameters:
      data: "{{poll.data}}"      # Latest result only
    depends_on:
      - activity_key: check_status
        conditions:
          - "{{check_status.ready == true}}"  # Only when ready
```

---

## Performance Considerations

### Validation Overhead: O(1) Lookups

Kruxia Flow uses precomputed metadata for loop detection:

**Validation Time** (once, at workflow registration):
- Topological sort to detect back-edges: O(V+E)
- Mark `is_loop_activity` and `is_back_edge`: O(V+E)
- Store metadata in database with workflow definition

**Execution Time** (every activity completion):
- Check if activity is in loop: O(1) (cached metadata lookup)
- Check if dependency is back-edge: O(1) (cached metadata lookup)
- **No graph traversal in orchestrator hot path**

### Storage Efficiency

**Iteration-scoped activities** store all outputs:
- 100 iterations × 10KB per output = 1MB total
- Acceptable for most use cases
- Consider `iteration_scoped: false` if outputs are large and history not needed

**Non-iteration-scoped activities** store only latest:
- 100 iterations × 10KB per output = 10KB total (latest only)
- Use when storage efficiency matters

### Orchestrator Performance

Loops do not degrade orchestrator performance:
- Loop metadata checked via O(1) lookups (not graph traversal)
- 50+ iteration loops have negligible overhead
- Performance benchmarks validate <1ms latency per iteration check

### Running Performance Benchmarks

Kruxia Flow includes a benchmark suite to validate loop performance characteristics:

```bash
# Run the loop performance benchmark
cargo bench --bench loop_performance

# Results will show:
# - is_loop_activity: O(1) cached metadata lookup (~1.8ns)
# - is_back_edge: O(1) cached metadata lookup (~1.8ns)
# - is_loop_activity_traversal: O(V+E) graph traversal (OLD way - 11x-105x slower)
# - iteration_loop_overhead: Per-iteration overhead (~1.5ns per iteration)
```

**What the benchmark validates**:
- Cached metadata lookups remain constant time regardless of workflow size
- Graph traversal time grows linearly with workflow complexity
- Multiple iterations have negligible overhead (constant per-iteration cost)
- Performance improvement: 11x for 5 activities → 105x for 100 activities

**Detailed results**: See [docs/performance/reports/2025-11-21-LOOP-PERFORMANCE-BENCHMARK.md](../performance/reports/2025-11-21-LOOP-PERFORMANCE-BENCHMARK.md)

---

## Debugging Loops

### Enable Detailed Logging

```bash
RUST_LOG=kruxiaflow_core::orchestrator=debug,kruxiaflow_core::workflow=debug
```

### Check Iteration State

Query workflow state to inspect loop progress:

```sql
SELECT
  key,
  status,
  iteration,
  accumulated_cost_usd,
  iteration_outputs
FROM workflows
WHERE workflow_id = '<workflow-id>';
```

### Common Debug Scenarios

**Loop Not Starting**:
- Check if initial activity dependencies are met
- Verify `is_loop_activity` metadata set during validation

**Loop Not Exiting**:
- Check condition evaluation (use `| last` filter?)
- Verify iteration limit not reached
- Check budget not exceeded

**Unexpected Loop Behavior**:
- Review condition logic (AND vs OR)
- Check if iteration-scoped outputs are arrays
- Verify budget accumulation vs per-iteration cost

---

## Future Enhancements (Post-MVP)

- **Nested loops**: Loop within a loop
- **Dynamic loop targets**: Loop back to different activities based on runtime conditions
- **Parallel iterations**: Multiple iterations executing simultaneously
- **Output field extraction**: Simplified JSON field access in conditions
- **Loop replay/debugging**: Step through iterations in dashboard
- **Visual loop representation**: Show loops in workflow diagram

---

## See Also

- [Architecture Documentation](./architecture.md)
- [US-3.4 Implementation Plan](./implementation/US-3.4-iterative-workflows.md)
- [Example 6: Agentic Research](../examples/06-agentic-research.yaml)
- [YAML Syntax Reference](./yaml-syntax.md)
