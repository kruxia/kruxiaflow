# US-MCP-7: Monitor Workflow Execution Status

**Epic:** MCP Server for AI Agent Integration
**Category:** Observability
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** monitor the status of running workflows
**So that** I can provide progress updates to users and detect when workflows complete or fail

---

## Acceptance Criteria

### AC1: Get Workflow Status
- ✅ Agent can retrieve workflow status by ID
- ✅ Returns overall status (pending, running, completed, failed, canceled)
- ✅ Returns start and completion timestamps

### AC2: Activity-Level Status
- ✅ Can optionally include all activity statuses
- ✅ Each activity shows status, start time, completion time
- ✅ Failed activities include error messages
- ✅ Shows retry count for activities

### AC3: List Workflows
- ✅ Can list all workflows with pagination
- ✅ Can filter by status
- ✅ Returns summary information for each workflow

---

## Implementation Details

### MCP Tool: `get_workflow_status`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/observability.py:18-56`

**Signature:**
```python
async def get_workflow_status(
    workflow_id: str,
    include_activities: bool = False,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns (Basic):**
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "definition_name": "research_assistant",
  "status": "running",
  "started_at": "2026-01-30T12:00:00Z",
  "completed_at": null
}
```

**Returns (With Activities):**
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "definition_name": "research_assistant",
  "status": "running",
  "started_at": "2026-01-30T12:00:00Z",
  "completed_at": null,
  "activities": [
    {
      "key": "search_docs",
      "activity_name": "http_request",
      "status": "completed",
      "started_at": "2026-01-30T12:00:01Z",
      "completed_at": "2026-01-30T12:00:03Z",
      "retry_count": 0
    },
    {
      "key": "search_reddit",
      "activity_name": "http_request",
      "status": "completed",
      "started_at": "2026-01-30T12:00:01Z",
      "completed_at": "2026-01-30T12:00:04Z",
      "retry_count": 1
    },
    {
      "key": "generate_summary",
      "activity_name": "llm_prompt",
      "status": "running",
      "started_at": "2026-01-30T12:00:05Z",
      "completed_at": null,
      "retry_count": 0
    }
  ]
}
```

**API Endpoint Used:** `GET /api/v1/workflows/:id?include_activities=true`

---

### MCP Tool: `list_workflows`

**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/observability.py:58-96`

**Signature:**
```python
async def list_workflows(
    status: str | None = None,
    limit: int = 20,
    offset: int = 0,
    ctx: Context | None = None,
) -> dict[str, Any]
```

**Returns:**
```json
{
  "workflows": [
    {
      "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
      "definition_name": "research_assistant",
      "status": "completed",
      "started_at": "2026-01-30T11:00:00Z",
      "completed_at": "2026-01-30T11:00:15Z"
    }
  ],
  "total": 1,
  "limit": 20,
  "offset": 0
}
```

**API Endpoint Used:** `GET /api/v1/workflows?status=running&limit=20&offset=0`

---

## Usage Examples

### Example 1: Simple Progress Monitoring
```python
# Submit workflow
wf = await submit_workflow("data_processing", {})

# Monitor until complete
import asyncio
while True:
    status = await get_workflow_status(wf["workflow_id"])

    print(f"Status: {status['status']}")

    if status["status"] in ["completed", "failed", "canceled"]:
        break

    await asyncio.sleep(5)  # Check every 5 seconds

if status["status"] == "completed":
    print("✓ Workflow completed successfully")
elif status["status"] == "failed":
    print(f"✗ Workflow failed: {status.get('error')}")
```

### Example 2: Detailed Progress with Activities
```python
# Submit workflow
wf = await submit_workflow("research_assistant", {"topic": "Rust"})

# Show detailed progress
while True:
    status = await get_workflow_status(wf["workflow_id"], include_activities=True)

    # Calculate progress
    completed = sum(1 for a in status["activities"] if a["status"] == "completed")
    total = len(status["activities"])
    percent = (completed / total * 100) if total > 0 else 0

    print(f"Progress: {completed}/{total} activities ({percent:.0f}%)")

    # Show running activities
    running = [a for a in status["activities"] if a["status"] == "running"]
    for activity in running:
        print(f"  ⚙️  {activity['key']} ({activity['activity_name']})")

    if status["status"] in ["completed", "failed", "canceled"]:
        break

    await asyncio.sleep(3)
```

### Example 3: Monitor Multiple Workflows
```python
# User has multiple workflows running
running_workflows = await list_workflows(status="running")

print(f"You have {running_workflows['total']} workflows running:")
for wf in running_workflows["workflows"]:
    print(f"  - {wf['definition_name']} ({wf['workflow_id']})")

    # Get detailed status for each
    detail = await get_workflow_status(wf["workflow_id"], include_activities=True)
    completed = sum(1 for a in detail["activities"] if a["status"] == "completed")
    total = len(detail["activities"])
    print(f"    Progress: {completed}/{total}")
```

### Example 4: Error Detection and Reporting
```python
status = await get_workflow_status(wf_id, include_activities=True)

if status["status"] == "failed":
    print("✗ Workflow failed")

    # Find which activity failed
    failed = [a for a in status["activities"] if a["status"] == "failed"]

    for activity in failed:
        print(f"\nFailed Activity: {activity['key']}")
        print(f"  Type: {activity['activity_name']}")
        print(f"  Error: {activity.get('error')}")
        print(f"  Retries: {activity['retry_count']}")

        # Suggest fixes based on error
        if "rate limit" in activity.get("error", "").lower():
            print("  💡 Suggestion: Add delay or increase retry backoff")
        elif "timeout" in activity.get("error", "").lower():
            print("  💡 Suggestion: Increase timeout setting")
```

### Example 5: Real-time Updates to User
```python
# Submit workflow
wf = await submit_workflow("long_pipeline", {})

print(f"Started workflow {wf['workflow_id']}")

# Stream progress updates to user
last_completed = 0
while True:
    status = await get_workflow_status(wf["workflow_id"], include_activities=True)

    completed = sum(1 for a in status["activities"] if a["status"] == "completed")

    # Only notify on progress changes
    if completed > last_completed:
        newly_completed = [a for a in status["activities"]
                          if a["status"] == "completed"][last_completed:completed]

        for activity in newly_completed:
            print(f"✓ Completed: {activity['key']}")

        last_completed = completed

    if status["status"] != "running":
        break

    await asyncio.sleep(2)
```

---

## Status Values

### Workflow Status
- **pending**: Created but not yet started
- **running**: Currently executing
- **completed**: All activities successful
- **failed**: One or more activities failed
- **canceled**: Canceled by user/agent

### Activity Status
- **pending**: Not yet started (waiting for dependencies)
- **running**: Currently executing
- **completed**: Finished successfully
- **failed**: Encountered error (after all retries)
- **skipped**: Skipped due to conditional logic

---

## Testing

**Test Coverage:**
- ✅ Get workflow status by ID
- ✅ Include activities parameter
- ✅ List workflows with status filter
- ✅ Pagination parameters

**Integration Testing:**
- Requires running workflows to test
- See: `docs/implementation/mcp-server-test-plan.md`

---

## Performance

- **Status Check:** <50ms (single DB query)
- **With Activities:** <100ms (joins activity table)
- **List Workflows:** <200ms for 100 workflows

**Best Practice:** Don't poll more frequently than every 2-3 seconds

---

## Related User Stories

- **US-MCP-5:** Submit Workflows for Execution
- **US-MCP-6:** Cancel Running Workflows
- **US-MCP-9:** Retrieve Activity Outputs
- **US-MCP-10:** Visualize Workflows

---

## Documentation

- **User Guide:** `docs/implementation/mcp-userguide.md` - Monitoring examples
- **Development Plan:** `docs/implementation/mcp-server-development-plan.md` - Task MCP-5
- **PRD:** `docs/implementation/mcp-server-prd.md` - Observability Tools section

---

## Future Enhancements

### Potential Additions
- **WebSocket Streaming:** Real-time status updates without polling
- **Status Change Callbacks:** Agent registers callback for status changes
- **Progress Percentage:** Built-in progress calculation
- **ETA Calculation:** Estimated time to completion
- **Activity Logs:** Stream activity logs in real-time
