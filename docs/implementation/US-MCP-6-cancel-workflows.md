# US-MCP-6: Cancel Running Workflows

**Epic:** MCP Server for AI Agent Integration
**Category:** Execution
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** cancel running workflows
**So that** I can stop workflows that are no longer needed or were started by mistake

---

## Acceptance Criteria

### AC1: Cancel Workflow
- ✅ Agent can cancel a running workflow by ID
- ✅ Can provide optional reason for cancellation
- ✅ Returns confirmation of cancellation

### AC2: Graceful Shutdown
- ✅ Running activities are allowed to complete
- ✅ Pending activities are not started
- ✅ Workflow status changes to "canceled"

### AC3: Audit Trail
- ✅ Cancellation reason is recorded
- ✅ Timestamp of cancellation captured

---

## Implementation Details

### MCP Tool: `cancel_workflow`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/execution.py:239-275`

**Signature:**
```python
async def cancel_workflow(
    workflow_id: str,
    reason: str | None = None,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Parameters:**
- `workflow_id`: Unique identifier of workflow to cancel
- `reason`: Optional reason for audit logging

**Returns:**
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "status": "canceled",
  "message": "Workflow canceled successfully"
}
```

**API Endpoint Used:** `POST /api/v1/workflows/:id/cancel`

---

## Usage Examples

### Example 1: User Changes Mind
```python
# User: "Start processing this large dataset"
wf = await submit_workflow("data_processing", {"dataset": large_data})

# User: "Actually, cancel that - I made a mistake"
await cancel_workflow(
    workflow_id=wf["workflow_id"],
    reason="User requested cancellation - wrong dataset"
)

print("✓ Workflow canceled")
```

### Example 2: Timeout Detection
```python
# Submit workflow
wf = await submit_workflow("long_running_job", {})

# Agent monitors execution
import asyncio
timeout = 300  # 5 minutes

start_time = time.time()
while True:
    if time.time() - start_time > timeout:
        # Taking too long, cancel it
        await cancel_workflow(
            workflow_id=wf["workflow_id"],
            reason="Exceeded expected execution time of 5 minutes"
        )
        print("⚠️ Workflow canceled due to timeout")
        break

    status = await get_workflow_status(wf["workflow_id"])
    if status["status"] in ["completed", "failed", "canceled"]:
        break

    await asyncio.sleep(5)
```

### Example 3: Budget Overrun Prevention
```python
# Submit workflow without budget limit
wf = await submit_workflow("expensive_analysis", {})

# Monitor cost in real-time
while True:
    status = await get_workflow_status(wf["workflow_id"])
    if status["status"] in ["completed", "failed", "canceled"]:
        break

    cost = await get_workflow_cost(wf["workflow_id"])
    if cost["total_cost_usd"] > 5.0:
        # Cost getting too high, cancel
        await cancel_workflow(
            workflow_id=wf["workflow_id"],
            reason="Cost exceeded user's comfort level ($5.00)"
        )
        print(f"⚠️ Workflow canceled at ${cost['total_cost_usd']:.2f}")
        break

    await asyncio.sleep(10)
```

### Example 4: Error in User Input Detected
```python
# Submit workflow
wf = await submit_workflow("send_emails", {"recipients": email_list})

# Agent detects problem after submission
if len(email_list) > 1000:
    # Too many emails, probably a mistake
    await cancel_workflow(
        workflow_id=wf["workflow_id"],
        reason="Email list suspiciously large (1000+ recipients)"
    )
    print("⚠️ Canceled - please confirm you want to email 1000+ people")
```

---

## Cancellation Behavior

### Activity States After Cancellation

| Activity Status | Behavior |
|----------------|----------|
| **pending** | Will not start |
| **running** | Allowed to complete (graceful) |
| **completed** | No change (already done) |
| **failed** | No change (already failed) |

### Partial Results

- All completed activities: Results are available via `get_activity_output()`
- Running activities: May complete and have results
- Pending activities: No results

### Cost Tracking

- Cost is tracked for all activities that ran before cancellation
- `get_workflow_cost()` returns accurate cost for completed work
- No refunds for work already done

---

## Testing

**Test Coverage:**
- ✅ Cancellation request sent to correct endpoint
- ✅ Reason parameter passed correctly
- ✅ Response includes workflow_id and status

**Integration Testing:**
- Requires running Kruxia Flow instance
- Test cancellation of different workflow states
- Verify graceful shutdown behavior

---

## Edge Cases

### Already Completed/Failed
```python
# Try to cancel completed workflow
result = await cancel_workflow(completed_workflow_id)
# API may return error or success (idempotent)
# Status remains "completed"
```

### Already Canceled
```python
# Try to cancel already-canceled workflow
result = await cancel_workflow(canceled_workflow_id)
# Idempotent - returns success
```

### Non-existent Workflow
```python
try:
    await cancel_workflow("invalid-id")
except httpx.HTTPStatusError as e:
    if e.response.status_code == 404:
        print("❌ Workflow not found")
```

---

## Related User Stories

- **US-MCP-5:** Submit Workflows for Execution
- **US-MCP-7:** Monitor Workflow Execution Status
- **US-MCP-8:** Track and Estimate Workflow Costs

---

## Documentation

- **User Guide:** `docs/implementation/mcp-userguide.md` - Cancellation examples
- **Development Plan:** `docs/implementation/mcp-server-development-plan.md` - Task MCP-4
- **PRD:** `docs/implementation/mcp-server-prd.md` - Execution Tools section

---

## Future Enhancements

### Potential Additions
- **Force Cancel:** Immediately terminate running activities (non-graceful)
- **Cancel with Notification:** Cancel and send notification to user
- **Batch Cancel:** Cancel multiple workflows at once
- **Scheduled Cancel:** Cancel workflow if still running after X minutes
