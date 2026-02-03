# Prompt 5: Observability Tools - COMPLETE ✅

## Summary

Implemented the 5 MCP Observability Tools that let AI agents monitor workflow executions, retrieve outputs, and analyse costs — both for running/completed workflows and as pre-execution estimates. All tools compile and all 288 existing tests pass with zero regressions.

## Deliverables

### 1. ✅ Tool Implementations — `src/mcp/tools/observability.rs`

| Tool                     | DB?  | Description                                                                                                                           |
|--------------------------|------|---------------------------------------------------------------------------------------------------------------------------------------|
| `get_workflow_status`    | Yes  | Full status by workflow ID. Optional `include_activities` flag parses the activities JSONB into per-activity status array.           |
| `list_workflows`         | Yes  | Paginated list with optional status filter. Returns total matching count alongside the page.                                         |
| `get_activity_output`    | Yes  | Output + cost + files for a single activity. Constructs `PostgresStorage` locally (pool-only, same as API server). Handles `ActivityNotFound` / `ActivityNotCompleted` as JSON payloads. |
| `get_workflow_cost`      | Yes  | Cost breakdown: total via `get_workflow_cost()` stored proc; per-activity detail via direct query on `activity_costs`; activity names cross-referenced from workflow JSONB; providers aggregated. Budget utilisation computed when limit is set. |
| `estimate_workflow_cost` | Yes  | Pre-execution estimate. Walks the deployed definition, finds `llm_prompt` activities, substitutes `{{INPUT.key}}` from `input_sample`, calls `CostCalculator::estimate_llm_cost()` for min (25 % of max_tokens) and max (100 %). Non-LLM activities are zero-cost. |

**SDK integration pattern:**
- All 5 tools: `#[mcp_tool(..., read_only_hint = true, idempotent_hint = true)]`
- `tool_box!(ObservabilityTools, [...])` generates enum + routing glue
- All tools are DB-backed; each has an async `run_*` free function called from the handler

**Helpers (private):**
- `text_response()` — same pretty-print wrapper as discovery/execution
- `parse_uuid()` — UUID parse with clear error message
- `extract_activities_array()` — normalises activities JSONB (handles both object-keyed and array formats) into a flat array with `key` injected
- `extract_activity_name_map()` — builds `HashMap<key, activity_name>` for cost annotation
- `substitute_input_template()` — replaces `{{INPUT.key}}` placeholders with stringified `input_sample` values

### 2. ✅ Tool Registry — `src/mcp/tools/mod.rs`

Exports `ObservabilityTools`. `list_tools` now returns all 12 tools (4 Discovery + 3 Execution + 5 Observability).

### 3. ✅ Handler Routing — `src/mcp/handler.rs`

Added third arm to the name-first routing match. `handle_list_tools_request` concats all three tool groups.

## Design Decisions

1. **`get_activity_output` storage**: `PostgresStorage::new(pool.clone())` constructed locally per call. It only holds a pool — no shared state. Matches how the API server creates it.

2. **Cost total**: Reuses `get_workflow_cost($1)` stored proc (same one `CostTracker` calls internally). Avoids duplicating aggregation logic.

3. **Budget source**: Read `workflow_budget_limit_usd` from `activity_costs` rows (where the orchestrator writes it). NULL when no costs have been recorded yet.

4. **Decimal → f64**: All cost fields in JSON responses use `.to_f64()` via `rust_decimal::prelude::ToPrimitive`. Matches Python MCP convention of returning numbers, not strings.

5. **Cost queries**: Use runtime `sqlx::query()` (not compile-time `query!` macro) for the two new SQL statements. The stored proc call and the `activity_costs` GROUP BY are not in the sqlx prepare cache; runtime queries avoid that dependency.

6. **Estimate range**: min = 25 % of `max_tokens`, max = 100 %. `estimated_cost_usd` = midpoint. If model pricing is missing, cost defaults to 0 with a warning log.

## Warnings (Expected)

38 warnings, all "never constructed / never used" — the HTTP transport placeholder in `server.rs` doesn't yet wire up the handler. All resolve in Prompt 7.

## Test Results

```
kruxiaflow unit tests:        220 passed, 0 failed
  └─ mcp::config::tests:        5 passed
cli integration tests:         27 passed, 0 failed
distributed deployment tests:  41 passed, 0 failed
─────────────────────────────────────────────────
Total:                        288 passed, 0 failed
```

## What Was NOT Done (Intentional)

- No new tests (per CLAUDE.md: "do not implement tests unless and until asked")
- HTTP transport not implemented (Prompt 7)
- `list_waiting_workflows` and `send_workflow_signal` not implemented (Prompt 6)

## Next Phase

**Prompt 6: Visualization & Control Tools** — `render_workflow_diagram`, `render_cost_diagram`, `send_workflow_signal`, `list_waiting_workflows`
