# US-MCP-12: Human-in-the-Loop Workflow Control

**Epic:** MCP Server for AI Agent Integration
**Category:** Control
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** send signals to workflows waiting for human approval
**So that** I can enable human-in-the-loop patterns for high-stakes operations

---

## Acceptance Criteria

### AC1: Send Signal
- ✅ Agent can send named signal to waiting workflow
- ✅ Can include signal data (approval details, user info, etc.)
- ✅ Workflow resumes execution after signal

### AC2: Use Cases
- ✅ Approval gates for deployments
- ✅ Quality review checkpoints
- ✅ Manual data validation
- ✅ Escalation to human experts

---

## Implementation

**Tool:** `send_workflow_signal`
**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/control.py:18-75`
**API Endpoint:** `POST /api/v1/workflows/:id/signal`

## Example Pattern

Workflow with approval gate:
```yaml
- key: wait_for_approval
  activity_name: wait_for_signal
  parameters:
    signal_name: "deployment_approved"
    timeout_seconds: 86400
```

Agent sends approval:
```python
await send_workflow_signal(
    workflow_id=wf_id,
    signal_name="deployment_approved",
    signal_data={"approved_by": "frank"}
)
```
