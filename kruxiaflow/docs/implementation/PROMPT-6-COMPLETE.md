# Prompt 6: Visualization & Control Tools - COMPLETE ✅

## Summary

Implemented the 4 MCP Visualization & Control Tools that let AI agents render Mermaid diagrams of workflow structure and costs, send signals to waiting activities, and discover which workflows are blocked on signals. All tools compile and all 288 existing tests pass with zero regressions.

## Deliverables

### 1. ✅ Tool Implementations — `src/mcp/tools/visualization.rs`

| Tool                    | DB?  | Description                                                                                                                                  |
|-------------------------|------|----------------------------------------------------------------------------------------------------------------------------------------------|
| `render_workflow_diagram` | Yes  | Mermaid `flowchart TD` from deployed definition deps. Optional `workflow_id` adds per-activity status colours (green/amber/red/orange/blue/grey). |
| `render_cost_diagram`   | Yes  | Mermaid `flowchart TD` cost tree: purple root (total) → teal activity nodes. Reuses stored-proc + `activity_costs` GROUP BY from Prompt 5.  |

**Mermaid generation details:**
- Node IDs sanitised (non-alphanumeric → underscore) to avoid parser issues with dots/hyphens in activity keys
- Status colour map: Completed=#28a745, Running=#ffc107, Failed=#dc3545, Waiting=#fd7e14, Pending=#17a2b8, Skipped=#adb5bd, NotScheduled=#6c757d
- `render_workflow_diagram` supports two modes: definition-only (static deps, no colours) and execution (status colours from workflow JSONB)
- Activities sorted highest-cost-first in cost diagram

### 2. ✅ Tool Implementations — `src/mcp/tools/control.rs`

| Tool                      | DB?  | Description                                                                                                                                                       |
|---------------------------|------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `send_workflow_signal`    | Yes  | Delivers a signal via `PostgresSubscriptionService::signal_activity()`, then publishes `ActivitySignaled` event via `PostgresEventSource::publish()` (best-effort). |
| `list_waiting_workflows`  | Yes  | Runtime SQL on `activity_event_subscriptions` JOIN `workflows`. Filters by `signal_data IS NULL` (unsignaled) and optionally by `event_name`. Groups by workflow. |

**SDK integration pattern:**
- `render_workflow_diagram` / `render_cost_diagram`: `read_only_hint = true, idempotent_hint = true`
- `send_workflow_signal`: `read_only_hint = false, destructive_hint = true, idempotent_hint = false`
- `list_waiting_workflows`: `read_only_hint = true, idempotent_hint = true`
- `tool_box!(VisualizationTools, [...])` and `tool_box!(ControlTools, [...])` generate enum + routing glue

### 3. ✅ Tool Registry — `src/mcp/tools/mod.rs`

Exports `VisualizationTools` and `ControlTools`. `list_tools` now returns all 16 tools (4 Discovery + 3 Execution + 5 Observability + 2 Visualization + 2 Control).

### 4. ✅ Handler Routing — `src/mcp/handler.rs`

Added two new arms to the name-first routing match. `handle_list_tools_request` concats all five tool groups.

## Design Decisions

1. **Mermaid generated in-process.** The Python MCP visualization directories are empty — no reference implementation exists. Mermaid is straightforward string building; no external dependency needed.

2. **Node ID sanitisation.** Activity keys may contain dots, hyphens, or other characters that break Mermaid's flowchart parser. A `node_id()` helper maps non-alphanumeric chars to underscores. Used consistently in node declarations, edges, and style directives.

3. **Signal publish is best-effort.** `signal_activity()` is the authoritative action — it writes `signal_data` into the subscription row. The `EventSource::publish()` call notifies the orchestrator immediately, but even if it fails the orchestrator will discover the signal on its next poll cycle. A warning is logged on publish failure; the tool still returns success.

4. **`list_waiting_workflows` queries subscriptions directly.** Rather than fetching all running workflows and scanning their activities JSONB, we query `activity_event_subscriptions` with a `signal_data IS NULL` filter. This is precise (only truly open subscriptions) and avoids N+1 lookups. Uses runtime `sqlx::query()` (non-macro) to avoid prepare-cache dependency.

5. **Cost diagram reuses Prompt-5 queries verbatim.** The stored-proc total and `activity_costs` GROUP BY are copied directly. Per-activity aggregation across provider/model is done in a single simpler GROUP BY (just `activity_key`) since the diagram only needs one node per activity.

6. **Duplicate private helpers.** Each module defines its own `text_response` and `parse_uuid` (same pattern as execution.rs and observability.rs). Avoids a shared utils module for two trivial functions.

## Warnings (Expected)

57 warnings, all "never constructed / never used" — the HTTP transport placeholder in `server.rs` doesn't yet wire up the handler. All resolve in Prompt 7.

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

## Next Phase

**Prompt 7: Production Hardening** — HTTP transport wiring, auth middleware, rate limiting, metrics, audit logging. Resolves all placeholder warnings.
