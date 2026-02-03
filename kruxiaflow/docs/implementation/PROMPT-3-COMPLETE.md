# Prompt 3: Discovery Tools - COMPLETE ✅

## Summary

Implemented the 4 MCP Discovery Tools that let AI agents discover deployed workflows, activity types, and workflow authoring documentation. All tools compile and all 288 existing tests pass with zero regressions.

## Deliverables

### 1. ✅ Tool Implementations — `src/mcp/tools/discovery.rs`

| Tool | DB? | Description |
|------|-----|-------------|
| `list_workflow_definitions` | Yes | Lists latest version of every deployed workflow (name, version, activity count, created_at). Optional name-prefix filter. |
| `get_workflow_definition` | Yes | Full workflow structure by name + optional version. Returns activities, dependencies, settings. Not-found returns error payload (not Rust Err). |
| `list_activities` | No | Static catalog of 8 built-in activity types (echo, http_request, llm_prompt, embedding, postgres_query, postgres_transaction, email_send, script) with parameters, outputs, and settings. |
| `get_workflow_authoring_guide` | No | Static comprehensive guide: YAML structure, template expressions, dependency patterns (sequential, parallel, fan-in, conditional), settings, and 3 complete worked examples. |

**SDK integration pattern used:**
- `#[mcp_tool(...)]` macro on each tool struct — generates JSON Schema from struct fields automatically
- `tool_box!(DiscoveryTools, [...])` macro — generates the enum, `tools()` (Vec<Tool>), and `TryFrom<CallToolRequestParams>`
- DB tools are async free functions called from the handler; static tools have sync `call_tool(&self)` methods
- All responses: `CallToolResult::text_content` with pretty-printed JSON (matches Python MCP server convention)

### 2. ✅ Tool Registry — `src/mcp/tools/mod.rs`

Exports `DiscoveryTools` enum. `DiscoveryTools::tools()` returns the 4 tool definitions for the `list_tools` MCP response.

### 3. ✅ ServerHandler Implementation — `src/mcp/handler.rs`

Implements `rust_mcp_sdk::mcp_server::ServerHandler` for `KruxiaFlowMcpHandler`:
- `handle_list_tools_request` → returns `DiscoveryTools::tools()`
- `handle_call_tool_request` → parses via `DiscoveryTools::try_from(params)`, routes to the correct tool

### 4. ✅ Dependency — `Cargo.toml`

Added `async-trait = { workspace = true }` (required by ServerHandler trait).

### 5. ✅ Cleanup — `src/mcp/server.rs`

Removed unused `KruxiaFlowMcpHandler` import (handler is referenced from the trait impl, not server.rs directly).

## Database Access Pattern

`list_workflow_definitions` and `get_workflow_definition` use `WorkflowDefinitionRepository` from `kruxiaflow-core` — the same repository the API handlers use. No new SQL queries introduced. Version strings (`YYYYmmdd.HHMMSS.uuuuuu`) come pre-formatted from `StoredWorkflowDefinition.version`.

## Warnings (Expected)

13 warnings, all expected because the HTTP transport in `server.rs` is still a placeholder and doesn't yet construct `KruxiaFlowMcpHandler` or call the tools. These will resolve in Prompt 7 (Production Hardening / HTTP transport).

## Test Results

```
kruxiaflow unit tests:        220 passed, 0 failed
  └─ mcp::config::tests:        5 passed (all MCP config tests)
cli integration tests:         27 passed, 0 failed
distributed deployment tests:  41 passed, 0 failed
─────────────────────────────────────────────────
Total:                        288 passed, 0 failed
```

## What Was NOT Done (Intentional)

- No new tests written (per CLAUDE.md: "do not implement tests unless and until asked")
- HTTP transport not implemented (placeholder — Prompt 7)
- Authentication/rate limiting middleware not implemented (Prompt 7)
- No changes to the Python MCP server

## Next Phase

**Prompt 4: Execution Tools** — validate_workflow, submit_workflow, cancel_workflow
