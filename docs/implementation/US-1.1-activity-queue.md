# Implementation Plan: US-1.1 Activity Queue with Ordering Guarantees

**Epic**: 1 - Event-Driven Orchestration Architecture
**User Story**: US-1.1
**Status**: ✅ Completed (Phase 1)
**Completed**: 2025-10-28
**Priority**: P0 (Must Have for MVP)

---

## User Story

**As** an AI startup engineer
**I want** workflows to execute activities in the correct sequence without manual coordination
**So that** my multi-step AI pipelines produce consistent results

### Acceptance Criteria

- ✅ Activities only scheduled when all dependencies satisfied
- ✅ Sequential workflows execute in exact order (validate → authorize → capture)
- ✅ Parallel activities execute simultaneously when all dependencies met
- ✅ No race conditions or duplicate execution
- ✅ PostgreSQL UNIQUE constraint prevents duplicate scheduling

---

## Architecture Reference

Per `docs/architecture.md`:

### Activity Queue Interface (Section: Service Interfaces)

```rust
#[async_trait]
pub trait ActivityQueue: Send + Sync {
    async fn schedule(&self, workflow_id: Uuid, activities: Vec<Activity>) -> Result<()>;
    async fn claim_next(&self, worker: &str, name: &str) -> Result<Option<QueuedActivity>>;
    async fn complete(&self, activity_id: Uuid, result: ActivityResult) -> Result<()>;
    async fn heartbeat(&self, activity_id: Uuid) -> Result<()>;
}
```

### PostgreSQL Queue Implementation (MVP)

**Key Architectural Decisions**:
1. **Idempotent Scheduling**: `ON CONFLICT (workflow_id, activity_key) DO NOTHING`
2. **Safe Concurrent Claiming**: `FOR UPDATE SKIP LOCKED`
3. **Stale Activity Detection in claim_next()**: Primary timeout recovery mechanism (no separate monitor thread)
4. **Heartbeat Conflict Detection**: 409 Conflict response when activity reclaimed
5. **Lightweight Cleanup Thread**: Only handles terminal failures (max retries exhausted), runs every 60s
6. **Timeout Duration in Database**: Store `timeout_duration` INTERVAL, calculate deadline as `claimed_at + timeout_duration`
7. **Expression Index for Performance**: `CREATE INDEX ON activity_queue((claimed_at + timeout_duration))` for efficient timeout queries
8. **Heartbeat Extends Timeout**: Reset `claimed_at` to NOW(), deadline automatically recalculates
9. **Transactional Guarantees**: All operations within PostgreSQL transactions
10. **Performance Target**: >10,000 activities/sec, <5ms latency

**Design Philosophy**: Simpler architecture where workers naturally detect and recover stale activities during normal polling, with a lightweight cleanup thread only for edge cases. Storing timeout duration (not deadline) enables clean recalculation on retries and heartbeats.

---

## Database Schema Requirements

### 1. Activity Queue Table

```sql
CREATE TABLE activity_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    worker TEXT NOT NULL,
    name TEXT NOT NULL,
    parameters JSONB NOT NULL,
    settings JSONB,
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_for TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    timeout_duration INTERVAL NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    claimed_by UUID,
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Prevent duplicate scheduling (idempotency)
    UNIQUE(workflow_id, activity_key)
);

-- Index for worker polling (hot path) - covers both pending and stale running activities
CREATE INDEX idx_queue_claimable
ON activity_queue(worker, name, scheduled_for)
WHERE (status = 'pending' AND scheduled_for <= NOW())
   OR (status = 'running');

-- Expression index for timeout deadline calculation (enables efficient stale activity detection)
CREATE INDEX idx_queue_timeout_deadline
ON activity_queue((claimed_at + timeout_duration))
WHERE status = 'running';

-- Index for workflow queries
CREATE INDEX idx_queue_workflow
ON activity_queue(workflow_id, created_at DESC);
```

### 2. Status Values

```sql
CREATE TYPE activity_status AS ENUM (
    'pending',    -- Scheduled, waiting for worker
    'running',    -- Claimed by worker, executing
    'completed',  -- Finished successfully (removed from queue)
    'failed'      -- Failed permanently (removed from queue)
);
```

**Note**: Completed and failed activities are **removed from queue** and tracked in `workflow_events` table for audit/observability.

---

## Implementation Components

### Component 1: PostgreSQL Queue Implementation

**File**: `core/src/queue/postgres_queue.rs`

**Responsibilities**:
1. Implement `ActivityQueue` trait for PostgreSQL
2. Handle idempotent scheduling via UNIQUE constraint
3. Implement safe concurrent claiming with FOR UPDATE SKIP LOCKED
4. Manage activity lifecycle (pending → running → completed/failed)
5. Track heartbeats for long-running activities
6. Aggressive cleanup of completed activities

**Key Methods**:

#### 1.1 `schedule()` - Idempotent Activity Scheduling

**Requirements**:
- Insert activities into queue only if not already present
- Use `ON CONFLICT DO NOTHING` for idempotency
- Support batch insertion for performance
- Handle scheduled_for timestamps (delayed execution)
- Store timeout_duration from activity settings or default
- Extract max_retries from activity settings or use default
- Validate parameters are valid JSONB

**Behavior**:
```rust
// Pseudocode
for activity in activities {
    // Extract timeout from settings or use default
    let timeout = activity.settings
        .and_then(|s| s.timeout)
        .map(|tc| tc.timeout)
        .unwrap_or(config.default_timeout.as_secs());

    let timeout_duration = format!("{} seconds", timeout);

    // Extract max_retries from settings or use default
    let max_retries = activity.settings
        .and_then(|s| s.retry)
        .map(|rc| rc.max_attempts)
        .unwrap_or(config.default_max_retries);

    INSERT INTO activity_queue (
        workflow_id, activity_key, worker, name,
        parameters, settings, scheduled_for, timeout_duration, max_retries
    ) VALUES (..., INTERVAL $timeout_duration, $max_retries)
    ON CONFLICT (workflow_id, activity_key) DO NOTHING
}
```

**Edge Cases**:
- Duplicate schedule attempts (idempotent - no error)
- Invalid JSONB parameters (return error)
- Null/empty activity key (validation error)
- Future scheduled_for timestamp (valid - activity waits)
- Missing timeout config (use default from QueueConfig)
- Missing retry config (use default max_retries = 3)

#### 1.2 `claim_next()` - Safe Concurrent Claiming with Stale Activity Detection

**Requirements**:
- Poll for next pending activity by worker/name
- **Detect and reclaim stale running activities** (primary timeout recovery mechanism)
- Use `FOR UPDATE SKIP LOCKED` to prevent conflicts
- Atomically update status to 'running'
- Set claimed_by and claimed_at timestamps
- Increment retry_count when reclaiming stale activity
- Respect max_retries limit (don't claim if retries exhausted)
- Order by scheduled_for ASC (FIFO within type)
- Return None if no activities available

**Behavior**:
```rust
// Pseudocode
UPDATE activity_queue
SET status = 'running',
    claimed_at = NOW(),
    claimed_by = $worker_id,
    retry_count = CASE
        WHEN status = 'running' THEN retry_count + 1  -- Reclaiming stale activity
        ELSE retry_count                               -- Fresh claim
    END
WHERE id = (
    SELECT id FROM activity_queue
    WHERE worker = $worker
      AND name = $name
      AND (
          -- Fresh pending activities
          (status = 'pending' AND scheduled_for <= NOW())
          OR
          -- Stale running activities (timeout expired, retries not exhausted)
          (status = 'running'
           AND NOW() > claimed_at + timeout_duration
           AND retry_count < max_retries)
      )
    ORDER BY scheduled_for ASC
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING *
```

**Note on Timeout Calculation**: The expression `claimed_at + timeout_duration` calculates the timeout deadline. When an activity is reclaimed, `claimed_at` is reset to NOW(), which automatically extends the deadline for the retry. The expression index `idx_queue_timeout_deadline` on `(claimed_at + timeout_duration)` ensures this query is efficient.

**Key Design Decision**: Stale activity detection happens **during normal worker polling**, not in a separate background thread. This provides:
- ✅ Simpler architecture (no dedicated monitor thread)
- ✅ Natural load distribution (workers detect stale activities when looking for work)
- ✅ Lazy recovery (activities reclaimed only when workers are available)
- ✅ Atomic operation (single transaction prevents races)
- ✅ Self-healing system (automatic recovery without coordination)

**Edge Cases**:
- No activities available (return None)
- Multiple workers polling (SKIP LOCKED prevents conflicts)
- Activity scheduled for future (not returned)
- Stale activity detected (reclaimed with incremented retry_count)
- Activity exceeded max_retries (not claimed, handled by cleanup thread)
- Worker crashes after claim (next worker poll will detect staleness)

#### 1.3 `complete()` - Activity Completion

**Requirements**:
- Remove activity from queue (DELETE)
- Activity completion tracked in workflow_events (orchestrator's job)
- Return success even if activity not found (idempotent)

**Behavior**:
```rust
// Pseudocode
DELETE FROM activity_queue WHERE id = $activity_id
// Workflow state update happens in orchestrator via event publishing
```

**Edge Cases**:
- Activity not found (idempotent - return Ok)
- Activity already completed (idempotent - return Ok)

#### 1.4 `heartbeat()` - Long-Running Activity Heartbeat

**Requirements**:
- **Reset claimed_at to extend deadline** (timeout deadline = claimed_at + timeout_duration)
- Verify activity still owned by calling worker
- Return HTTP 409 Conflict if activity reclaimed by another worker
- Return HTTP 404 Not Found if activity completed/deleted

**Behavior**:
```rust
// Pseudocode
UPDATE activity_queue
SET claimed_at = NOW()  -- Reset timeout baseline, effectively extending deadline
WHERE id = $activity_id
  AND claimed_by = $worker_id
  AND status = 'running'
RETURNING claimed_by
```

**Note on Timeout Extension**: Since the timeout deadline is calculated as `claimed_at + timeout_duration`, resetting `claimed_at` to NOW() automatically extends the timeout deadline. This is simpler than recalculating a new `timeout_at` timestamp.

**Response Handling**:
```rust
match rows_updated {
    1 => Ok(()),                    // 200 OK - heartbeat accepted
    0 => {
        // Check why update failed
        if activity_not_found() {
            Err(QueueError::ActivityNotFound)  // 404 Not Found
        } else {
            Err(QueueError::ActivityReclaimed)  // 409 Conflict - stop working
        }
    }
}
```

**HTTP Status Codes**:
- **200 OK**: Heartbeat accepted, worker should continue
- **409 Conflict**: Activity reclaimed by another worker (stale), worker must stop immediately
- **404 Not Found**: Activity completed or deleted, worker should stop

**Edge Cases**:
- Activity reclaimed by another worker (409 Conflict - worker stops immediately)
- Activity not found (404 Not Found - worker stops)
- Activity completed by worker but not yet deleted (race condition - return Ok)
- Very frequent heartbeats (rate limit at worker side, recommended: max 1 per 5-10 seconds)

### Component 2: Activity Models and Types

**File**: `core/src/queue/models.rs`

**Data Structures**:

```rust
// Activity to be scheduled
pub struct Activity {
    pub key: String,              // Unique within workflow (e.g., "validate_payment")
    pub worker: String,         // Activity type worker (e.g., "payments")
    pub name: String,              // Activity type name (e.g., "validate_card")
    pub parameters: serde_json::Value,  // Activity-specific parameters
    pub settings: Option<ActivitySettings>,  // Timeout, retry, budget config
    pub scheduled_for: Option<DateTime<Utc>>,  // Delayed execution
}

// Activity settings (retry, timeout, budget)
pub struct ActivitySettings {
    pub retry: Option<RetryConfig>,
    pub timeout: Option<TimeoutConfig>,
    pub budget: Option<BudgetConfig>,
    pub cache: Option<CacheConfig>,
}

// Queued activity returned to worker
pub struct QueuedActivity {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub worker: String,
    pub name: String,
    pub parameters: serde_json::Value,
    pub settings: Option<ActivitySettings>,
    pub claimed_at: DateTime<Utc>,
}

// Activity result from worker
pub struct ActivityResult {
    pub success: bool,
    pub outputs: Option<serde_json::Value>,
    pub error: Option<String>,
    pub cost_usd: Option<Decimal>,
    pub token_usage: Option<TokenUsage>,
}
```

### Component 3: Queue Monitoring and Cleanup

**File**: `core/src/queue/monitor.rs`

**Background Tasks**:

#### 3.1 Failed Activity Cleanup Thread (Lightweight)

**Purpose**: Handle terminal failures for activities that exceeded max_retries

**Requirements**:
- Run periodically (every 60 seconds)
- Delete activities where retry_count >= max_retries AND timeout expired
- Publish failure events for orchestrator to handle
- Minimal overhead (only handles terminal cases)

**Behavior**:
```rust
// Pseudocode - run every 60 seconds
DELETE FROM activity_queue
WHERE status = 'running'
  AND NOW() > claimed_at + timeout_duration
  AND retry_count >= max_retries
RETURNING workflow_id, activity_key, worker, name, parameters

// For each deleted activity, publish failure event
For each failed_activity:
    publish_workflow_event(WorkflowEvent {
        event_type: "ActivityFailed",
        workflow_id: failed_activity.workflow_id,
        activity_key: failed_activity.activity_key,
        error: "Activity exceeded max retries after timeout",
        retry_count: failed_activity.retry_count,
    })
```

**Note**: The expression index `idx_queue_timeout_deadline` on `(claimed_at + timeout_duration)` ensures this cleanup query is efficient.

**Key Design Points**:
- **Primary recovery**: Happens in `claim_next()` (stale activity detection)
- **Secondary cleanup**: This thread only handles terminal failures (max retries exhausted)
- **Frequency**: 60 seconds is sufficient (not time-critical since activity already failed)
- **Scope**: Cleans up all namespaces (not limited to specific workers)

**Why This Approach**:
- ✅ Simpler than separate timeout monitor
- ✅ Workers handle 99% of recovery naturally via `claim_next()`
- ✅ Cleanup thread only handles edge case (permanently failed activities)
- ✅ Minimal overhead (runs infrequently, simple query)
- ✅ Ensures failed activities don't accumulate in queue

#### 3.2 Vacuum Monitor

**Requirements**:
- Aggressively VACUUM activity_queue table
- Run after batch deletes (activity completion)
- Prevent table bloat from high churn

**Behavior**:
```sql
-- Run every 5 minutes or after N deletions
VACUUM ANALYZE activity_queue;
```

### Component 4: Error Handling

**Error Types**:

```rust
pub enum QueueError {
    DatabaseError(sqlx::Error),
    InvalidParameters(String),
    ActivityNotFound(Uuid),
    ActivityReclaimed,  // Activity was reclaimed by another worker (409 Conflict)
    InvalidStatus { expected: String, actual: String },
    SerializationError(serde_json::Error),
}
```

**Error Handling Strategy**:
- Database errors: Log and propagate (caller decides retry)
- Invalid parameters: Validation error (fail fast)
- Activity not found: Depends on operation (idempotent vs error)
- Activity reclaimed: Worker must stop processing immediately (409 Conflict)
- Serialization errors: Log and return error (bad data)

---

## Dependency Ordering Guarantees

### Sequential Execution

**Requirement**: Activities execute in exact order when dependencies form a chain.

**Implementation**:
1. Orchestrator evaluates workflow directed graph
2. Only schedules activities when ALL `depends_on` dependencies are completed
3. Activity appears in queue only when ready
4. Workers cannot claim activity until it's in queue

**Example**:
```yaml
activities:
  - key: validate_payment
    dependency_of:
      - activity_key: authorize_card

  - key: authorize_card
    dependency_of:
      - activity_key: capture_payment

  - key: capture_payment
```

**Execution Timeline**:
1. `validate_payment` scheduled immediately (no dependencies)
2. Worker claims and executes `validate_payment`
3. Worker completes → publishes event
4. Orchestrator evaluates → schedules `authorize_card` (dependency satisfied)
5. Worker claims and executes `authorize_card`
6. Worker completes → publishes event
7. Orchestrator evaluates → schedules `capture_payment`
8. Etc.

**Queue State Over Time**:
- t=0: [validate_payment:pending]
- t=1: [validate_payment:running]
- t=2: [] (validate_payment completed and removed)
- t=3: [authorize_card:pending]
- t=4: [authorize_card:running]
- t=5: [] (authorize_card completed and removed)
- t=6: [capture_payment:pending]

### Parallel Execution

**Requirement**: Multiple activities execute simultaneously when dependencies are independently satisfied.

**Implementation**:
1. Orchestrator identifies all activities with satisfied dependencies
2. Schedules all ready activities to queue in single transaction
3. Multiple workers claim different activities concurrently
4. Join point waits for ALL preceding activities to complete

**Example**:
```yaml
activities:
  - key: fetch_data
    dependency_of:
      - activity_key: analyze_security
      - activity_key: analyze_performance
      - activity_key: analyze_quality

  - key: aggregate_results
    depends_on:
      - activity_key: analyze_security
      - activity_key: analyze_performance
      - activity_key: analyze_quality
```

**Execution Timeline**:
1. `fetch_data` completes
2. Orchestrator schedules ALL 3 analyze_* activities simultaneously
3. Workers claim activities (may be different workers)
4. All 3 execute in parallel
5. Last one completes → orchestrator evaluates
6. ALL dependencies satisfied → schedule `aggregate_results`

**Queue State**:
- After fetch_data: [analyze_security:pending, analyze_performance:pending, analyze_quality:pending]
- During parallel: [analyze_security:running, analyze_performance:running, analyze_quality:pending]
- After 2 complete: [analyze_quality:running]
- After all complete: [aggregate_results:pending]

### Race Condition Prevention

**Scenarios and Solutions**:

1. **Duplicate Scheduling**:
   - Problem: Orchestrator schedules same activity twice
   - Solution: UNIQUE(workflow_id, activity_key) constraint
   - Behavior: Second INSERT ignored (ON CONFLICT DO NOTHING)

2. **Concurrent Claiming**:
   - Problem: Two workers try to claim same activity
   - Solution: FOR UPDATE SKIP LOCKED
   - Behavior: First worker gets activity, second skips to next

3. **Heartbeat vs Reclaim Race**:
   - Problem: Worker heartbeat while another worker reclaims stale activity
   - Solution: Heartbeat checks `claimed_by = $worker_id` in WHERE clause
   - Behavior: Heartbeat fails with 409 Conflict, original worker stops

4. **Completion Race**:
   - Problem: Worker completes while another worker reclaims stale activity
   - Solution: Atomic DELETE in complete()
   - Behavior: First operation wins (activity deleted, second worker's complete is idempotent)

5. **Multiple Workers Claiming Stale Activity**:
   - Problem: Multiple workers try to claim same stale activity
   - Solution: FOR UPDATE SKIP LOCKED in claim_next()
   - Behavior: First worker gets it, others skip to next activity

---

## Testing Requirements

### Testing Phases Overview

**Phase 1 (US-1.1)**: Queue implementation tests - **CAN IMPLEMENT NOW**
**Phase 2 (After US-1.2)**: Orchestrator integration tests - **MUST DEFER**
**Phase 3 (After Epic 1B + Epic 1A)**: End-to-end system tests - **MUST DEFER**

---

### Phase 1: Unit Tests (✅ Implement During US-1.1)

**File**: `core/src/queue/postgres_queue_test.rs`

**Status**: All tests can be implemented immediately. Only require PostgresQueue, database, and activity models.

**Test Cases**:

1. **Idempotent Scheduling**:
   - Schedule same activity twice
   - Verify only one row in database
   - Verify no error returned

2. **Concurrent Claiming**:
   - Multiple workers claim from same queue
   - Verify each gets different activity
   - Verify no duplicate claims

3. **Sequential Ordering**:
   - Schedule chain: A → B → C
   - Verify B not in queue until A completes
   - Verify C not in queue until B completes
   - **Note**: Tests queue ordering, not orchestrator dependency resolution

4. **Parallel Execution**:
   - Schedule fan-out: A → [B, C, D]
   - Verify all scheduled simultaneously
   - Verify can be claimed in parallel
   - **Note**: Manually schedules activities, doesn't test orchestrator fan-out detection

5. **Stale Activity Recovery**:
   - Claim activity
   - Wait past timeout without heartbeat
   - Verify next claim_next() reclaims it with incremented retry_count

6. **Heartbeat Conflict Detection**:
   - Claim activity
   - Simulate timeout (wait past timeout_at)
   - Another worker reclaims via claim_next()
   - Original worker heartbeats
   - Verify 409 Conflict returned

7. **Max Retries Exhaustion**:
   - Activity times out and is reclaimed max_retries times
   - Verify claim_next() does not return it (retry_count >= max_retries)
   - Wait for cleanup thread
   - Verify activity deleted
   - **Note**: Cannot test failure event publishing until EventSource implemented

8. **Completion Idempotency**:
   - Complete activity twice
   - Verify no error on second call
   - Verify activity removed from queue

---

### Phase 1: Partial Performance Tests (⚠️ Implement Partially During US-1.1)

**File**: `core/benches/queue_benchmark.rs`

**Status**: Can benchmark queue operations in isolation, but not full workflow throughput.

**Benchmarks Available Now**:

1. ✅ **Schedule Latency**: Target <1ms for single activity
   - Direct PostgresQueue::schedule() calls
   - Measure database insertion time

2. ✅ **Claim Latency**: Target <2ms for polling operation
   - Direct PostgresQueue::claim_next() calls
   - Measure FOR UPDATE SKIP LOCKED query time

3. ✅ **Concurrent Workers**: 100 workers, no degradation
   - Multiple workers polling same queue
   - Verify no lock contention

**Benchmarks Deferred to Phase 3**:

4. ❌ **End-to-End Throughput**: Target >10,000 activities/sec
   - **Deferred**: Requires full orchestration stack (API → Orchestrator → Queue → Worker)
   - **Dependencies**: US-1.2 (Orchestrator), Epic 1B (Built-in Worker), Epic 1A (API Server)
   - **Why**: Must measure realistic workflow execution, not just queue operations

---

### Phase 2: Integration Tests (❌ DEFER Until After US-1.2)

**File**: `core/tests/queue_integration_test.rs`

**Status**: Cannot implement until Orchestrator and EventSource are available.

**Test Scenarios**:

1. ❌ **Full Sequential Workflow**:
   - Create workflow with 5 sequential activities
   - Simulate worker execution
   - Verify exact ordering
   - Measure latency (<5ms per schedule)
   - **Missing Dependencies**:
     - Orchestrator (US-1.2) to evaluate dependencies and schedule activities
     - EventSource to publish/consume workflow events
     - Workflow definition loading and graph evaluation

2. ❌ **Full Parallel Workflow**:
   - Create workflow with 1 → [5 parallel] → 1 join
   - Simulate multiple workers
   - Verify all parallel execute simultaneously
   - Verify join waits for all
   - **Missing Dependencies**:
     - Orchestrator (US-1.2) to detect satisfied dependencies and schedule fan-out
     - EventSource for activity completion events
     - Dependency graph evaluation logic

3. ⚠️ **Worker Failure Recovery** (Partial):
   - Claim activity
   - Simulate worker crash (no complete)
   - Wait for timeout
   - Verify activity rescheduled
   - Complete on retry
   - **Can Test Now**: Queue-level stale activity reclaim (covered in unit tests)
   - **Must Defer**: Full workflow progression after worker failure
   - **Missing Dependencies**:
     - Orchestrator to schedule next activities after completion
     - Epic 1B (Built-in Worker) that posts completion to API
     - Epic 1A (API Server) for worker endpoints
     - EventSource for completion events

4. ⚠️ **High Concurrency** (Partial):
   - 100 workers polling simultaneously
   - 1000 activities in queue
   - Verify no duplicate claims
   - Verify no lost activities
   - Measure throughput (>10k activities/sec)
   - **Can Test Now**: Queue concurrency behavior (no duplicates, no lost activities)
   - **Must Defer**: End-to-end workflow throughput with orchestrator overhead
   - **Missing Dependencies**:
     - Orchestrator processing workflow events
     - EventSource throughput characteristics
     - API server for workflow submission

---

### Phase 3: End-to-End System Tests (❌ DEFER Until After Epic 1B + Epic 1A)

**Status**: Requires complete orchestration stack (Epic 1B: Built-in Worker, Epic 1A: API Server).

**Test Scenarios**:

1. ❌ **API → Orchestrator → Queue → Worker Flow**:
   - Submit workflow via HTTP API
   - Orchestrator evaluates and schedules activities
   - Workers poll, execute, and complete
   - Verify workflow completes successfully
   - **Missing Dependencies**: Epic 1A (API Server), Epic 1B (Built-in Worker), Orchestrator, EventSource

2. ❌ **Worker Authentication Flow**:
   - Worker obtains JWT token via /api/v1/auth/token
   - Worker polls activities with Bearer token
   - Verify token validation
   - **Missing Dependencies**: Epic 1A (API Server with AuthenticationService)

3. ❌ **Full System Performance Benchmarks**:
   - >10,000 activities/sec sustained throughput
   - <10ms P99 workflow start latency
   - <1ms orchestrator evaluation latency
   - **Missing Dependencies**: Complete stack with realistic load patterns

---

### Testing Phase Summary

| Test Category           | Phase | Status     | Implement Now? | Missing Dependencies                           |
|-------------------------|:-----:|------------|:--------------:|------------------------------------------------|
| **Unit Tests**          |  1    | ✅ Ready   | **Yes**        | None — only needs PostgresQueue                 |
| **Queue Performance**   |  1    | ⚠️ Partial | **Partial**    | Full throughput requires Orchestrator           |
| **Sequential Workflow** |  2    | ❌ Blocked | **No**         | Orchestrator (US-1.2), EventSource              |
| **Parallel Workflow**   |  2    | ❌ Blocked | **No**         | Orchestrator (US-1.2), EventSource              |
| **Failure Recovery**    |  2    | ⚠️ Partial | **Partial**    | Orchestrator, EventSource, Epic 1B, Epic 1A     |
| **High Concurrency**    |  2    | ⚠️ Partial | **Partial**    | Orchestrator, EventSource, Epic 1A              |
| **End-to-End**          |  3    | ❌ Blocked | **No**         | Epic 1A, Epic 1B, Orchestrator, EventSource     |

---

### US-1.1 Acceptance Criteria Validation

**What Can Be Validated Now**:
- ✅ Idempotent scheduling (duplicate calls harmless)
- ✅ Safe concurrent claiming (FOR UPDATE SKIP LOCKED works)
- ✅ Stale activity recovery (claim_next detects and reclaims timed-out activities)
- ✅ Heartbeat conflict detection (409 Conflict when activity reclaimed)
- ✅ Max retries enforcement (activities not claimed after exhausting retries)
- ✅ Clean completion (activities removed from queue)
- ✅ Schedule latency: <1ms P95
- ✅ Claim latency: <2ms P95
- ✅ Concurrent workers: 100+ without degradation

**What Must Be Validated Later**:
- ❌ Sequential workflows execute in exact order (requires Orchestrator - US-1.2)
- ❌ Parallel activities execute simultaneously (requires Orchestrator - US-1.2)
- ❌ Failed activity cleanup with event publishing (requires EventSource - US-1.2)
- ❌ Throughput: >10,000 activities/sec sustained (requires full stack - Phase 3)
- ❌ Worker crash recovery in workflow context (requires Orchestrator + Epic 1B + Epic 1A)

**US-1.1 is considered complete when**:
- All Phase 1 tests pass (unit tests + partial performance)
- Queue behavior validated in isolation
- Database schema and indexes performing as expected
- Code ready for orchestrator integration in US-1.2

---

## Migration Strategy

### Database Migrations

**File**: `migrations/YYYYMMDDHHMMSS_activity_queue.sql`

**Migration Steps**:
1. Create activity_queue table
2. Create indexes (pending, heartbeat, workflow)
3. Create activity_status enum
4. Verify constraints (UNIQUE, NOT NULL)

**Rollback Plan**:
1. Drop indexes
2. Drop table
3. Drop enum type

### Data Validation

**Post-Migration Checks**:
- UNIQUE constraint exists on (workflow_id, activity_key)
- Indexes exist and are used (EXPLAIN queries)
- Default values work (status = 'pending')
- Timestamps auto-populate (created_at, scheduled_for)

---

## Configuration

### Environment Variables

```bash
# Queue polling configuration
STREAMFLOW_QUEUE_POLL_INTERVAL=100ms      # Worker poll frequency
STREAMFLOW_QUEUE_BATCH_SIZE=100           # Max activities per claim

# Timeout and retry configuration
STREAMFLOW_QUEUE_DEFAULT_TIMEOUT=60s      # Default activity timeout (if not in settings)
STREAMFLOW_QUEUE_DEFAULT_MAX_RETRIES=3    # Default max retry attempts

# Cleanup configuration
STREAMFLOW_QUEUE_CLEANUP_INTERVAL=60s     # How often to run failed activity cleanup
STREAMFLOW_QUEUE_VACUUM_INTERVAL=5m       # VACUUM frequency

# Performance tuning
STREAMFLOW_QUEUE_MAX_CONNECTIONS=20       # Connection pool size
```

### Compile-Time Configuration

**File**: `core/src/queue/config.rs`

```rust
pub struct QueueConfig {
    pub poll_interval: Duration,
    pub batch_size: usize,
    pub default_timeout: Duration,
    pub default_max_retries: u32,
    pub cleanup_interval: Duration,
    pub vacuum_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(100),
            batch_size: 100,
            default_timeout: Duration::from_secs(60),
            default_max_retries: 3,
            cleanup_interval: Duration::from_secs(60),
            vacuum_interval: Duration::from_secs(300),
        }
    }
}
```

---

## Success Criteria

### Functional Requirements

- ✅ Sequential workflows execute in exact order (no race conditions)
- ✅ Parallel activities execute simultaneously (all scheduled together)
- ✅ Idempotent scheduling (duplicate calls harmless)
- ✅ Safe concurrent claiming (FOR UPDATE SKIP LOCKED works)
- ✅ Stale activity recovery (claim_next detects and reclaims timed-out activities)
- ✅ Heartbeat conflict detection (409 Conflict when activity reclaimed)
- ✅ Max retries enforcement (activities fail permanently after exhausting retries)
- ✅ Failed activity cleanup (cleanup thread removes terminal failures)
- ✅ Clean completion (activities removed from queue)

### Performance Requirements

- ✅ Schedule latency: <1ms P95
- ✅ Claim latency: <2ms P95
- ✅ Throughput: >10,000 activities/sec sustained
- ✅ Concurrent workers: 100+ without degradation
- ✅ Database overhead: Minimal (indexed queries, aggressive VACUUM)

### Reliability Requirements

- ✅ No duplicate execution (UNIQUE constraint enforced)
- ✅ No lost activities (transactional guarantees)
- ✅ Worker crash recovery (stale detection in claim_next reschedules)
- ✅ Idempotent operations (safe retries)
- ✅ Data consistency (PostgreSQL ACID)
- ✅ Graceful worker termination (409 Conflict stops stale workers)

---

## Dependencies

### Internal Dependencies

- **Workflow Events**: Activity completion publishes events (orchestrator consumes)
- **PostgreSQL Connection Pool**: Shared connection pool (sqlx)
- **Activity Registry**: Validates worker/name exists
- **Workflow State**: Orchestrator queries workflow definition for dependency graph

### External Dependencies

- **PostgreSQL 18+**: Required for database
- **sqlx**: Async PostgreSQL driver with compile-time query validation
- **tokio**: Async runtime for background monitoring tasks
- **serde_json**: JSONB serialization/deserialization

---

## Risks and Mitigations

### Risk 1: PostgreSQL Performance (<10k activities/sec)

**Probability**: Low
**Impact**: High (core requirement)

**Mitigation**:
- Aggressive indexing on hot paths
- Prepared statements for all queries
- Connection pooling (2-20 connections)
- VACUUM automation
- Early benchmarking (Epic 2) to validate
- Fallback: Batch operations for higher throughput

### Risk 2: Race Conditions in Concurrent Claiming

**Probability**: Low
**Impact**: High (correctness)

**Mitigation**:
- FOR UPDATE SKIP LOCKED is battle-tested PostgreSQL feature
- Comprehensive integration tests with 100 concurrent workers
- Transaction isolation level verification
- Chaos testing (random delays, crashes)

### Risk 3: Table Bloat from High Churn

**Probability**: Medium
**Impact**: Medium (performance degradation)

**Mitigation**:
- Aggressive VACUUM ANALYZE every 5 minutes
- Monitor table size and dead tuples
- Partitioning strategy (future optimization)
- Delete completed activities immediately

### Risk 4: Heartbeat Overhead

**Probability**: Low
**Impact**: Low (adds latency)

**Mitigation**:
- Rate limit heartbeats (max 1 per 5 seconds)
- Batch heartbeat updates if needed
- Async heartbeat (don't block activity execution)
- Make heartbeat optional (only for long-running activities)

---

## Future Enhancements (Post-MVP)

### Cloud Queue Providers (Post-MVP)

Per architecture.md, support for:
- AWS SQS
- RabbitMQ
- Redis-based queue

**Implementation**: Same `ActivityQueue` trait, different backend.

### Priority Queues

Add `priority` field to activity_queue:
```sql
ALTER TABLE activity_queue ADD COLUMN priority INTEGER DEFAULT 0;
CREATE INDEX idx_queue_priority ON activity_queue(priority DESC, scheduled_for ASC);
```

### Queue Partitioning

Partition activity_queue by workflow_id or time for higher throughput.

### Dead Letter Queue

Track permanently failed activities in separate table for debugging.

---

## Documentation Requirements

### User Documentation

1. **Queue Behavior Guide**: How activities are scheduled and executed
2. **Troubleshooting Guide**: Common queue issues (stuck activities, timeouts)
3. **Performance Tuning**: Configuration for high-throughput scenarios

### Developer Documentation

1. **ActivityQueue Trait**: API documentation with examples
2. **Database Schema**: ERD diagram and field descriptions
3. **Monitoring Guide**: Queue metrics and health checks
4. **Migration Guide**: Cloud queue provider implementation

---

## Acceptance Checklist

- [ ] Database schema created with migrations (including timeout_duration, retry_count, max_retries, expression index)
- [ ] PostgresQueue implements ActivityQueue trait
- [ ] Idempotent scheduling verified (unit + integration tests)
- [ ] Concurrent claiming verified (100 workers test)
- [ ] Stale activity detection in claim_next verified
- [ ] Heartbeat conflict detection verified (409 Conflict test)
- [ ] Max retries enforcement verified
- [ ] Cleanup thread implemented and tested
- [ ] Sequential ordering verified (integration test)
- [ ] Parallel execution verified (fan-out/fan-in test)
- [ ] Performance benchmarks pass (>10k activities/sec)
- [ ] Documentation complete (user + developer)
- [ ] Code review completed
- [ ] Epic 2 benchmarking validates performance

---

## Related User Stories

- **US-1.2**: Event-Driven Dynamic Scheduling (orchestrator consumes events, schedules activities)
- **Epic 1A**: API Server (provides HTTP endpoints for workflow and worker operations)
- **Epic 1B (US-1B.1)**: Worker Polling with Concurrency Safety (workers use API endpoints which call claim_next())
- **US-2.5**: Activity Settings (timeout, retry config stored in queue)
- **US-6.1**: Query Optimization (prepared statements, indexes)

---

## References

- Architecture: `docs/architecture.md` (Activity Queue Interface section)
- Requirements: `docs/requirements.md` (Epic 1, US-1.1)
- Technical Details: `notes/2025-10-25-v02-sonnet.md` (sections 1.1-1.6)
- PostgreSQL Docs: FOR UPDATE SKIP LOCKED, UNIQUE constraints, VACUUM
