# US-MCP-13: Find Workflows Awaiting Signals

**Epic:** MCP Server for AI Agent Integration
**Category:** Control
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** discover workflows that are waiting for signals
**So that** I can prompt users for approvals or decisions

---

## Acceptance Criteria

### AC1: List Waiting Workflows
- ✅ Agent can list workflows with status="waiting"
- ✅ Can filter by specific signal name
- ✅ Returns workflow ID, definition, and wait details

### AC2: Proactive Notifications
- ✅ Agent can check for workflows needing attention
- ✅ Can notify users of pending approvals
- ✅ Shows how long workflow has been waiting

---

## Implementation

**Tool:** `list_waiting_workflows`
**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/control.py:77-149`
**API Endpoint:** `GET /api/v1/workflows?status=waiting`

## Usage Pattern

Agent checks for pending approvals:
```python
waiting = await list_waiting_workflows()

if waiting["total"] > 0:
    print(f"You have {waiting['total']} workflows awaiting approval:")
    for wf in waiting["workflows"]:
        print(f"  - {wf['definition_name']} ({wf['workflow_id']})")
        print(f"    Waiting for: {wf.get('waiting_for_signal')}")
```
