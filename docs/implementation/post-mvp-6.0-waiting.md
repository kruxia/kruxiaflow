# Implementation Plan: Story 6.0 - Event-Based Activity Waiting

## Overview

Enable any activity to wait for an external signal before running via `settings.wait_for_signal`. When dependencies are met and the activity has this setting, it enters `waiting` state. When a signal arrives (or timeout occurs), the activity transitions to `pending`/`skipped`/`failed`.

## Implementation Status

| Step                          | Status      | Notes                                    |
|-------------------------------|-------------|------------------------------------------|
| Database Migration            | ✅ Complete | `20260126000001_activity_event_subscriptions.up.sql` |
| ActivityStatus enum           | ✅ Complete | Added `Waiting` variant                  |
| WorkflowActivityStatus        | ✅ Complete | Added `Waiting` variant                  |
| ActivitySettings              | ✅ Complete | Added `wait_for_signal` field            |
| WaitForSignalSettings         | ✅ Complete | New type with event_name, timeout, on_timeout |
| OnTimeout enum                | ✅ Complete | Continue, Skip, Fail variants            |
| Subscription module           | ✅ Complete | `core/src/subscription/`                 |
| Event types                   | ✅ Complete | ActivityWaiting, ActivitySignaled        |
| Workflow state                | ✅ Complete | signal_data field, event handling        |
| Template context              | ✅ Complete | SIGNAL variable for templates            |
| Signal API endpoint           | ✅ Complete | POST /api/v1/workflows/{id}/signal       |
| Queue models                  | ✅ Complete | signal_data in Activity, QueuedActivity  |
| Python SDK                    | ✅ Complete | ctx.signal, signal_data in PendingActivity |
| Orchestrator scheduling logic | ✅ Complete | Handle wait_for_signal in scheduling     |
| Timeout handler               | ✅ Complete | Process expired subscriptions            |

## Implementation Steps

### Step 1: Database Migration ✅

**File**: `migrations/20260126000001_activity_event_subscriptions.up.sql`

- Added 'waiting' to `activity_status` enum
- Added `ActivityWaiting` and `ActivitySignaled` to `workflow_event_type` enum
- Added `signal_data` column to `activity_queue` table
- Created `activity_event_subscriptions` table with indexes

### Step 2: Core Model Changes ✅

**2.1 ActivityStatus enum** (`core/src/queue/models.rs`)
- Added `Waiting` variant

**2.2 WorkflowActivityStatus** (`core/src/orchestrator/workflow_state.rs`)
- Added `Waiting` variant

**2.3 ActivitySettings** (`core/src/workflow/definition.rs`)
- Added `wait_for_signal: Option<WaitForSignalSettings>` field

**2.4 New types** (`core/src/workflow/definition.rs`):
```rust
pub struct WaitForSignalSettings {
    pub event_name: String,
    pub timeout_seconds: u64,
    pub on_timeout: OnTimeout,
}

pub enum OnTimeout {
    Continue,
    Skip,
    Fail, // default
}
```

### Step 3: Subscription Service ✅

**Created module**: `core/src/subscription/`
- `mod.rs` - module exports
- `models.rs` - ActivitySubscription, NewSubscription, SignalRequest, ExpiredSubscription
- `service.rs` - SubscriptionService trait
- `postgres_subscription.rs` - PostgreSQL implementation

**Trait methods**:
- `create_subscription(NewSubscription) -> Result<Uuid>`
- `signal_activity(SignalRequest) -> Result<Option<ActivitySubscription>>`
- `get_signal_data(workflow_id, activity_key) -> Result<Option<Value>>`
- `expire_subscriptions(limit) -> Result<Vec<ExpiredSubscription>>`
- `delete_subscription(workflow_id, activity_key) -> Result<()>`

### Step 4: Event Types ✅

**File**: `core/src/events/models.rs`

Added to `WorkflowEventType` enum:
- `ActivityWaiting` - Activity entered waiting state
- `ActivitySignaled` - Activity received signal

### Step 5: Orchestrator Changes ✅

**5.1 Dependency Evaluator** (`core/src/orchestrator/dependency_evaluator.rs`) ✅
- Handle `Waiting` status in `is_activity_ready()` - return false (already in progress)

**5.2 Activity Scheduling** (`core/src/orchestrator/orchestrator.rs`) ✅
When scheduling activity with `wait_for_signal` setting:
- Create subscription instead of scheduling to queue
- Set activity status to `Waiting`
- Publish `ActivityWaiting` event
- Skip `ActivityWaiting` events in orchestrator (observability only, like `ActivityScheduled`)

**5.3 Handle ActivitySignaled event** in `process_workflow_event()` ✅
- State transitions implemented in `apply_event_to_state()`
- Signal data stored in activity state
- Transition activity `Waiting` → `NotScheduled` (so dependency evaluator picks it up for scheduling)
- Handle `on_timeout: skip` by transitioning to `Skipped` status

**5.4 Timeout handler** (`core/src/orchestrator/orchestrator.rs`) ✅
Added `check_and_handle_expired_subscriptions()` to `timeout_checker_task`:
- `on_timeout: continue` → Publish `ActivitySignaled` event (activity proceeds without signal data)
- `on_timeout: skip` → Publish `ActivitySignaled` event with `on_timeout: skip` payload (activity set to Skipped)
- `on_timeout: fail` → Publish `ActivityFailed` event with `SIGNAL_TIMEOUT` error code

**5.5 Function signatures** ✅
- Added `subscription_service: Arc<dyn SubscriptionService>` to `run_orchestrator`, `process_workflow_event`, and `timeout_checker_task`
- Added `InternalError` variant to `OrchestratorError`

### Step 6: Workflow State ✅

**File**: `core/src/orchestrator/workflow_state.rs`

- Added `signal_data: Option<Value>` to `ActivityState` struct
- Handle `ActivityWaiting` and `ActivitySignaled` in `apply_event_to_state()`

### Step 7: Template Context ✅

**File**: `core/src/workflow/template.rs`

- Added `signal: Option<Value>` to `TemplateContext`
- Added `SIGNAL` to MiniJinja context in `to_minijinja_value()`

### Step 8: Signal API Endpoint ✅

**New file**: `api/src/handlers/signals.rs`

```rust
pub struct SignalActivityRequest {
    pub activity_key: String,
    pub event_name: String,
    pub data: Option<serde_json::Value>,
}

// POST /api/v1/workflows/{workflow_id}/signal
pub async fn signal_activity(...) -> ApiResult<Json<SignalActivityResponse>>
```

**Route registration**: `api/src/routes.rs`

### Step 9: Poll Response Changes ✅

**File**: `api/src/handlers/workers.rs`

- Added `signal_data: Option<Value>` to `PendingActivityResponse`
- Include signal data when returning activities that were waiting

### Step 10: Python SDK ✅

**10.1 ActivityContext** (`py/kruxiaflow/worker/context.py`)
```python
signal: dict[str, Any] | None = None
```

**10.2 PendingActivity** (`py/kruxiaflow/worker/client.py`)
```python
signal_data: dict[str, Any] | None = None
```

**10.3 Pass signal data** to ActivityContext when creating context from PendingActivity

## Critical Files

| File                                          | Changes                              | Status |
|-----------------------------------------------|--------------------------------------|--------|
| `core/src/queue/models.rs`                    | Add `Waiting` to ActivityStatus      | ✅     |
| `core/src/workflow/definition.rs`             | Add WaitForSignalSettings            | ✅     |
| `core/src/orchestrator/orchestrator.rs`       | Scheduling logic + timeout handler   | ✅     |
| `core/src/orchestrator/workflow_state.rs`     | State transitions                    | ✅     |
| `core/src/orchestrator/dependency_evaluator.rs` | Handle Waiting status              | ✅     |
| `core/src/events/models.rs`                   | New event types                      | ✅     |
| `core/src/subscription/`                      | New module (4 files)                 | ✅     |
| `api/src/handlers/signals.rs`                 | New signal endpoint                  | ✅     |
| `api/src/routes.rs`                           | Register signal route                | ✅     |
| `py/kruxiaflow/worker/context.py`             | Add signal field                     | ✅     |
| `py/kruxiaflow/worker/client.py`              | Add signal_data to PendingActivity   | ✅     |

## Remaining Work

All implementation steps are complete. The feature is ready for integration testing.

## Verification

1. **Unit tests**: Test WaitForSignalSettings parsing, OnTimeout enum
2. **Integration tests**:
   - Activity with wait_for_signal enters Waiting state
   - Signal API transitions Waiting → Pending
   - on_timeout: continue schedules activity with null signal
   - on_timeout: skip skips activity
   - on_timeout: fail fails activity
3. **E2E test**: Workflow with wait_for_signal activity → signal → completion
4. **Python test**: Activity receives signal data in ctx.signal

## Example Usage

### Workflow Definition
```yaml
name: approval_workflow
activities:
  - key: wait_for_approval
    worker: processor
    activity_name: process_approval
    settings:
      wait_for_signal:
        event_name: approval_received
        timeout_seconds: 86400  # 24 hours
        on_timeout: fail
    parameters:
      data: "{{SIGNAL.approval_data}}"
```

### Signal API Call
```bash
curl -X POST /api/v1/workflows/{workflow_id}/signal \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "activity_key": "wait_for_approval",
    "event_name": "approval_received",
    "data": {"approved": true, "approver": "admin@example.com"}
  }'
```

### Python Worker
```python
@worker.activity("process_approval")
async def process_approval(ctx: ActivityContext, params: dict) -> dict:
    if ctx.signal:
        approved = ctx.signal.get("approved", False)
        approver = ctx.signal.get("approver")
        return {"status": "approved" if approved else "rejected", "by": approver}
    return {"status": "no_signal"}
```
