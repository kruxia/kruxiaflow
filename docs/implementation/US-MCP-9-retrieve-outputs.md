# US-MCP-9: Retrieve Activity Outputs

**Epic:** MCP Server for AI Agent Integration
**Category:** Observability
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** retrieve the outputs of specific activities in a workflow
**So that** I can access intermediate results and provide them to users

---

## Acceptance Criteria

### AC1: Get Activity Output
- ✅ Agent can retrieve output of any completed activity
- ✅ Returns full output structure (varies by activity type)
- ✅ Works for all activity types (http_request, llm_prompt, etc.)

### AC2: Output Formats
- ✅ http_request: Returns response, status_code, headers
- ✅ llm_prompt: Returns result, cost_usd, model, usage
- ✅ postgres_query: Returns rows, row_count
- ✅ embedding: Returns embeddings, dimensions, cost_usd

---

## Implementation

**Tool:** `get_activity_output`
**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/observability.py:98-116`
**API Endpoint:** `GET /api/v1/workflows/:id/activities/:key/output`
