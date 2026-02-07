# Feature: Parallel Iteration (`parallel_for_each`)

**Date**: 2026-02-07
**Status**: Proposed
**Related**: US-3.4 (Iterative Workflows), Story 6.3 (Dynamic Parallelism), US-3.3 (Parallel Execution)

---

## Problem

Current loops in Kruxia Flow iterate **sequentially**: iteration N+1 does not begin until iteration N completes. This is correct for loops where each iteration depends on the result of the previous one (e.g., agentic research that builds on prior findings). But many real-world iteration patterns are **embarrassingly parallel** — every iteration is independent, and the total set of iterations is known before iteration begins.

Examples of parallel-eligible iteration:

- Process each item in a list returned by a previous activity (e.g., analyze each document, send each notification)
- Run the same LLM prompt N times with different parameters (e.g., generate one section per chapter outline)
- Fan out HTTP requests to a batch of URLs
- Apply a transformation to each row in a dataset

Today, the only way to achieve this is to statically define N separate activities in the workflow YAML — which doesn't work when N is dynamic (determined at runtime by a prior activity).

## Design Goals

1. **Extend, don't replace**: Sequential loops (back-edge pattern) remain for iteration where N+1 depends on N
2. **Composable**: `parallel_for_each` (inner parallelism) composes with back-edges (outer sequential loop)
3. **Declarative**: Workflow author declares *what* to iterate over, orchestrator handles *how*
4. **Self-documenting**: The field name `parallel_for_each` makes the parallel semantics explicit
5. **Consistent**: Reuse existing `iteration_scoped` output collection and template context patterns
6. **Simple**: No new activity types — `parallel_for_each` is a property on a regular activity
7. **Safe**: Concurrency limits prevent resource exhaustion; error handling is explicit

## Proposal: `parallel_for_each` Field

Add a `parallel_for_each` field to `ActivityDefinition` that accepts a template expression evaluating to a **list**. When `parallel_for_each` is present, each execution of the activity fans out over the list, scheduling all items in parallel.

The name `parallel_for_each` was chosen deliberately: `for_each` is universally understood, and the `parallel_` prefix makes the execution semantics impossible to miss. You can't read it without understanding both *what* it does (iterate over a list) and *how* (in parallel).

### YAML Syntax

```yaml
# Iterate over a list from a previous activity
- key: process_document
  activity_name: llm_prompt
  parallel_for_each: "{{fetch_documents.document_list}}"
  iteration_scoped: true
  parameters:
    model: anthropic/claude-haiku-4-5-20251001
    prompt: "Summarize: {{ITERATION.item}}"
  depends_on:
    - fetch_documents

# Iterate over a range (static count)
- key: generate_section
  activity_name: llm_prompt
  parallel_for_each: "{{range(5)}}"
  iteration_scoped: true
  parameters:
    model: anthropic/claude-sonnet-4-5-20250929
    prompt: "Write section {{ITERATION.index}}: {{outline.sections[ITERATION.index]}}"
  depends_on:
    - outline

# Iterate over a range (dynamic count from prior activity)
- key: send_notification
  activity_name: http_request
  parallel_for_each: "{{range(count_recipients.total)}}"
  iteration_scoped: true
  parameters:
    method: POST
    url: "https://api.example.com/notify"
    body:
      recipient_index: "{{ITERATION.index}}"
  depends_on:
    - count_recipients
```

### How `parallel_for_each` Relates to Sequential Loops

`parallel_for_each` and back-edge loops operate on **different dimensions** and compose naturally:

- **`parallel_for_each`** = inner parallelism: fan out over a list within a single execution of the activity
- **Back-edge** = outer sequential loop: repeat the entire activity (including its fan-out) based on a condition

| Aspect                | Sequential Loop (back-edge)                    | Parallel Iteration (`parallel_for_each`)     |
|-----------------------|------------------------------------------------|----------------------------------------------|
| Iteration dependency  | N+1 depends on N                               | All items independent within a round         |
| Iteration count       | Unknown upfront (condition-based exit)          | Known before each round begins               |
| Mechanism             | Back-edge in dependency graph                   | `parallel_for_each` field on activity        |
| Scheduling            | One at a time, re-evaluated each cycle          | All items scheduled in single batch per round |
| Template context      | `{{ACTIVITY.iteration}}`                        | `{{ITERATION.item}}`, `{{ITERATION.index}}`  |

When used together, they compose as **nested iteration**: the back-edge controls *how many rounds*, and `parallel_for_each` controls *what to fan out over in each round*.

### Composing with Back-Edge Loops

A `parallel_for_each` activity can also participate in a back-edge loop. Each outer iteration triggers a fresh parallel fan-out:

```yaml
# Agentic research: each round searches multiple sources in parallel,
# then evaluates whether more research is needed
activities:
  - key: plan_searches
    activity_name: llm_prompt
    iteration_scoped: true
    iteration_limit: 5
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        Topic: {{INPUT.topic}}
        Previous search results: {{search_sources.results | json}}
        Plan 3-5 search queries for the next research round.
        Return JSON: {"queries": ["query1", "query2", ...]}
    depends_on:
      - activity_key: evaluate
        conditions:
          - "{{evaluate.result.content | contains(substring='CONTINUE')}}"

  - key: search_sources
    activity_name: http_request
    parallel_for_each: "{{plan_searches.result.queries | last}}"  # Fan out over this round's queries
    iteration_scoped: true
    parameters:
      method: GET
      url: "https://api.search.com/search?q={{ITERATION.item}}"
    depends_on:
      - plan_searches

  - key: evaluate
    activity_name: llm_prompt
    iteration_scoped: true
    parameters:
      model: anthropic/claude-haiku-4-5-20251001
      prompt: |
        All search results from this round:
        {{search_sources.results | json}}
        Respond CONTINUE or SUFFICIENT.
    depends_on:
      - search_sources
```

**Execution flow**:

```
Round 0: plan_searches → search_sources[0,1,2] (parallel) → evaluate → CONTINUE
Round 1: plan_searches → search_sources[0,1,2,3] (parallel) → evaluate → SUFFICIENT
                                                                          ↓
                                                                   compile_report
```

In this pattern:
- `plan_searches` loops via back-edge from `evaluate` (outer sequential loop)
- `search_sources` fans out in parallel over the queries for each round (inner parallelism)
- `iteration_limit: 5` on `plan_searches` bounds the outer loop
- Each round of `search_sources` can have a different number of items

### Template Context

Inside a `parallel_for_each` activity, the following template variables are available:

| Variable                 | Type | Description                                            |
|--------------------------|------|--------------------------------------------------------|
| `{{ITERATION.item}}`     | Any  | Current item from the `parallel_for_each` list         |
| `{{ITERATION.index}}`    | u32  | Zero-based index of this item within the current round |
| `{{ITERATION.total}}`    | u32  | Total number of items in the current round             |
| `{{ACTIVITY.iteration}}` | u32  | Outer loop iteration (0 if no back-edge)               |

When iterating over `range(N)`, `ITERATION.item` equals `ITERATION.index` (both are the integer index).

### Concurrency Control

An optional `max_concurrency` field limits how many parallel items execute simultaneously:

```yaml
- key: call_api
  activity_name: http_request
  parallel_for_each: "{{get_urls.url_list}}"
  max_concurrency: 10          # At most 10 concurrent items
  iteration_scoped: true
  parameters:
    method: GET
    url: "{{ITERATION.item}}"
  depends_on:
    - get_urls
```

**Behavior**:

- When `max_concurrency` is omitted: all items are scheduled at once (true fan-out)
- When `max_concurrency` is set: the orchestrator schedules up to N items initially, then schedules the next item as each one completes (sliding window)
- This reuses the existing batch scheduling + `FOR UPDATE SKIP LOCKED` worker claim mechanism

### Error Handling

An optional `on_iteration_failure` field controls behavior when individual items fail:

```yaml
- key: process_item
  activity_name: http_request
  parallel_for_each: "{{get_items.item_list}}"
  on_iteration_failure: continue   # Default: abort
  iteration_scoped: true
  parameters:
    url: "{{ITERATION.item.url}}"
  depends_on:
    - get_items
```

| Value      | Behavior                                                                                              |
|------------|-------------------------------------------------------------------------------------------------------|
| `abort`    | (Default) Any item failure fails the entire activity                                                  |
| `continue` | Failed items are recorded as null/error in results; activity completes when all items finish |

When `on_iteration_failure: continue`, downstream activities can inspect which items succeeded:

```yaml
- key: report
  depends_on:
    - process_item
  parameters:
    succeeded: "{{process_item.results | select(attribute='status', equalto='completed') | length}}"
    failed: "{{process_item.results | select(attribute='status', equalto='failed') | length}}"
```

### Fan-In (Downstream Dependencies)

Downstream activities that `depends_on` a `parallel_for_each` activity wait for **all items in the current round** to complete before becoming ready. This is the natural fan-in behavior — identical to how an activity that depends on multiple parallel activities waits for all of them today.

```yaml
- key: aggregate
  activity_name: llm_prompt
  parameters:
    prompt: |
      Summarize these {{process_document.results | length}} document summaries:
      {{process_document.results | json}}
  depends_on:
    - process_document    # Waits for ALL items of parallel_for_each to complete
```

## Orchestrator Implementation Sketch

### Workflow Validation (at registration time)

1. If `parallel_for_each` is present:
   - Mark `is_parallel_for_each: true` in cached metadata
   - Validate that `parallel_for_each` is a valid template expression
   - If `max_concurrency` is set, validate it is > 0
   - `iteration_scoped` defaults to `true` for `parallel_for_each` activities (since collecting results is almost always desired)
2. `parallel_for_each` and back-edges are **allowed together** (inner/outer composition)
3. `parallel_for_each` and `iteration_limit` are **allowed together** (`iteration_limit` bounds the outer back-edge loop; `parallel_for_each` controls the inner fan-out per round)

### Dependency Evaluation (at runtime)

When the orchestrator evaluates whether a `parallel_for_each` activity is ready:

1. Check that all `depends_on` are satisfied (normal dependency evaluation, including back-edge conditions for the outer loop)
2. Resolve the `parallel_for_each` template expression against current workflow state
3. The expression must evaluate to a JSON array (or `range()` result)
4. Determine item count = length of the resolved list

### Scheduling (fan-out)

Once a `parallel_for_each` activity is ready:

1. Resolve the `parallel_for_each` expression to get the list of items
2. For each item in the list, resolve the activity parameters with `ITERATION.item`, `ITERATION.index`, `ITERATION.total` injected into the template context
3. Schedule all items (or up to `max_concurrency`) as separate activity instances in a single batch
4. Each instance is identified by `(workflow_id, activity_key, outer_iteration, item_index)`

This reuses the existing batch scheduling path in `orchestrator.rs` — the `activities_to_schedule` Vec simply contains one entry per item, each with its own resolved parameters.

### Activity Queue Representation

Each item is a separate row in `activity_queue`:

| workflow_id | activity_key     | iteration | item_index | parameters          | status  |
|-------------|------------------|-----------|------------|---------------------|---------|
| wf-123      | process_document | 0         | 0          | {item: "doc_a.pdf"} | pending |
| wf-123      | process_document | 0         | 1          | {item: "doc_b.pdf"} | pending |
| wf-123      | process_document | 0         | 2          | {item: "doc_c.pdf"} | pending |

When composed with a back-edge (outer loop iteration 1):

| workflow_id | activity_key     | iteration | item_index | parameters          | status  |
|-------------|------------------|-----------|------------|---------------------|---------|
| wf-123      | search_sources   | 0         | 0          | {q: "query_a"}      | completed |
| wf-123      | search_sources   | 0         | 1          | {q: "query_b"}      | completed |
| wf-123      | search_sources   | 1         | 0          | {q: "query_c"}      | pending   |
| wf-123      | search_sources   | 1         | 1          | {q: "query_d"}      | pending   |
| wf-123      | search_sources   | 1         | 2          | {q: "query_e"}      | pending   |

Workers claim and execute items independently via the existing `FOR UPDATE SKIP LOCKED` mechanism — no changes to the worker claim path needed.

### Completion (fan-in)

When an item completes:

1. Store its result in the activity state's iteration outputs (same as sequential `iteration_scoped`)
2. Check if all items **for the current round** are complete
3. If yes: mark the current round as complete, evaluate downstream dependencies (which may include back-edge conditions for the outer loop)
4. If no: continue waiting (or schedule next item if `max_concurrency` window has a slot)

### State Representation

The `ActivityState` for a `parallel_for_each` activity tracks:

```rust
pub struct ActivityState {
    pub key: String,
    pub status: WorkflowActivityStatus,              // Overall status
    pub iteration: u32,                               // Outer loop iteration (0 if no back-edge)
    pub parallel_item_count: Option<u32>,              // Items in current round (set at fan-out)
    pub parallel_items_completed: u32,                 // How many items have finished this round
    pub parallel_items_failed: u32,                    // How many items have failed this round
    pub iteration_outputs: Option<serde_json::Value>,  // Array of results (ordered by item_index)
    pub accumulated_cost_usd: Decimal,                 // Sum across all items and rounds
    // ...existing fields...
}
```

## Example: Complete Workflow

```yaml
name: parallel_document_analysis
description: Fetch a list of documents, analyze each in parallel, then aggregate

activities:
  # Step 1: Get the list of documents to process
  - key: fetch_documents
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.example.com/documents?project={{INPUT.project_id}}"
    outputs:
      - result    # Returns {documents: [{id: "...", url: "..."}, ...]}

  # Step 2: Analyze each document in parallel (max 5 at a time)
  - key: analyze_document
    activity_name: llm_prompt
    parallel_for_each: "{{fetch_documents.result.documents}}"
    max_concurrency: 5
    on_iteration_failure: continue
    iteration_scoped: true
    parameters:
      model: anthropic/claude-haiku-4-5-20251001
      prompt: |
        Analyze document {{ITERATION.index + 1}} of {{ITERATION.total}}:
        Title: {{ITERATION.item.title}}
        Content: {{ITERATION.item.content}}

        Provide a structured analysis with key findings and risk assessment.
      max_tokens: 1000
    outputs:
      - result
    settings:
      budget:
        limit: 0.50           # $0.50 total across ALL items
        action: abort
    depends_on:
      - fetch_documents

  # Step 3: Aggregate all analyses into a report (waits for all items)
  - key: compile_report
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        Compile a comprehensive analysis report from {{analyze_document.results | length}} document analyses:
        {{analyze_document.results | json}}

        Identify cross-cutting themes, highest-risk items, and recommendations.
      max_tokens: 4000
    outputs:
      - result
    settings:
      budget:
        limit: 0.10
        action: abort
    depends_on:
      - analyze_document
```

## Example: Composed with Back-Edge Loop

```yaml
name: iterative_parallel_research
description: >
  Multi-round research where each round searches multiple sources in parallel.
  Outer loop (back-edge) controls rounds; inner fan-out parallelizes per-round work.

activities:
  - key: plan_round
    activity_name: llm_prompt
    iteration_scoped: true
    iteration_limit: 5              # Max 5 outer rounds
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        Topic: {{INPUT.topic}}
        Round: {{ACTIVITY.iteration}}
        Previous results: {{search_sources.results | json}}
        Plan search queries for this round. Return JSON: {"queries": [...]}
    depends_on:
      - activity_key: evaluate
        conditions:
          - "{{evaluate.result.content | contains(substring='CONTINUE')}}"

  - key: search_sources
    activity_name: http_request
    parallel_for_each: "{{plan_round.result.queries | last}}"
    max_concurrency: 3
    iteration_scoped: true
    parameters:
      method: GET
      url: "https://api.search.com/search?q={{ITERATION.item}}"
    depends_on:
      - plan_round

  - key: evaluate
    activity_name: llm_prompt
    iteration_scoped: true
    parameters:
      model: anthropic/claude-haiku-4-5-20251001
      prompt: |
        Search results from this round: {{search_sources.results | json}}
        Respond CONTINUE or SUFFICIENT.
    depends_on:
      - search_sources

  - key: compile_report
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        All research across {{plan_round.results | length}} rounds:
        {{search_sources.results | json}}
        Compile final report.
    depends_on:
      - activity_key: evaluate
        conditions:
          - "{{evaluate.result.content | contains(substring='SUFFICIENT')}}"
```

## Python SDK

```python
from kruxiaflow import Workflow, Activity, Dependency

workflow = Workflow(
    name="parallel_document_analysis",
    activities=[
        Activity(
            key="fetch_documents",
            activity_name="http_request",
            parameters={"method": "GET", "url": "https://api.example.com/documents"},
            outputs=["result"],
        ),
        Activity(
            key="analyze_document",
            activity_name="llm_prompt",
            parallel_for_each="{{fetch_documents.result.documents}}",
            max_concurrency=5,
            on_iteration_failure="continue",
            iteration_scoped=True,
            parameters={
                "model": "anthropic/claude-haiku-4-5-20251001",
                "prompt": "Analyze: {{ITERATION.item.title}}\n{{ITERATION.item.content}}",
            },
            outputs=["result"],
            depends_on=[Dependency(activity_key="fetch_documents")],
        ),
        Activity(
            key="compile_report",
            activity_name="llm_prompt",
            parameters={
                "model": "anthropic/claude-sonnet-4-5-20250929",
                "prompt": "Compile report from: {{analyze_document.results | json}}",
            },
            depends_on=[Dependency(activity_key="analyze_document")],
        ),
    ],
)
```

## Relationship to Story 6.3 (Dynamic Parallelism / Map/Reduce)

Story 6.3 in post-mvp.md proposes a `type: map` / `type: reduce` construct with dedicated activity types. This `parallel_for_each` feature **replaces and simplifies** that design:

| Story 6.3 (original)                   | `parallel_for_each` (this proposal)                 |
|-----------------------------------------|-----------------------------------------------------|
| New `type: map` activity type           | Regular activity with `parallel_for_each` field     |
| New `type: reduce` activity type        | Regular activity with `depends_on` (existing fan-in)|
| `over:` field on map type               | `parallel_for_each:` field on any activity          |
| `max_concurrency` on map type           | `max_concurrency` on any `parallel_for_each` activity|
| Separate `from:` on reduce type         | Standard `depends_on` (no special syntax)           |
| Cannot compose with sequential loops    | Composes naturally with back-edge loops             |

The `parallel_for_each` approach is simpler because it doesn't introduce new activity types or new dependency semantics. It composes naturally with all existing features (budget tracking, retry, back-edge loops, conditions on downstream dependencies, etc.). Story 6.3 should be updated to reference this design.

## Implementation Scope

### Phase 1: Core `parallel_for_each`

- `parallel_for_each` field on `ActivityDefinition` (YAML, JSON, Python SDK)
- `range()` template function for count-based iteration
- Orchestrator fan-out: resolve list, schedule all items
- Orchestrator fan-in: wait for all items in a round, collect results
- `ITERATION.item`, `ITERATION.index`, `ITERATION.total` template variables
- `iteration_scoped: true` default for `parallel_for_each` activities
- Composition with back-edge loops (outer sequential, inner parallel)
- `item_index` column in `activity_queue` table

### Phase 2: Concurrency and Error Control

- `max_concurrency` field with sliding window scheduling
- `on_iteration_failure: continue | abort`
- Partial results collection (nulls for failed items)
- Progress tracking: `parallel_items_completed` / `parallel_item_count` in workflow state

### Phase 3: Observability

- Per-item status in workflow state API
- Cost accumulation across items and rounds (reuse existing `accumulated_cost_usd`)
- Item-level retry (individual failed items can be retried independently)

## Validation Rules

1. `parallel_for_each` expression must resolve to a JSON array at runtime (runtime error if not)
2. Empty array (`[]`) results in the activity round being marked `Completed` immediately with empty results
3. `max_concurrency` must be > 0 if specified
4. `on_iteration_failure` is only valid when `parallel_for_each` is present
5. When composed with a back-edge, `iteration_limit` bounds the outer loop (not the inner fan-out)

## Budget Interaction

Budget limits apply **across all items and all rounds** (same as sequential loops):

- `budget.limit: 0.50` means the total cost of ALL items across ALL rounds cannot exceed $0.50
- When budget is exceeded mid-item, the failing item is aborted
- If `on_iteration_failure: abort` (default), remaining items in the round are cancelled
- If `on_iteration_failure: continue`, remaining items continue but the budget-exceeded item is recorded as failed
- `{{ACTIVITY.accumulated_cost_usd}}` reflects the running total across all completed items and rounds
