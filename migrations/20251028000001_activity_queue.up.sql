-- Activity status enum
CREATE TYPE activity_status AS ENUM (
    'pending',    -- Scheduled, waiting for worker
    'running',    -- Claimed by worker, executing
    'completed',  -- Finished successfully (will be removed from queue)
    'failed'      -- Failed permanently (will be removed from queue)
);

-- Activity queue table with timeout duration and retry tracking
CREATE TABLE activity_queue (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    worker TEXT NOT NULL,
    name TEXT NOT NULL,
    parameters JSONB NOT NULL,
    settings JSONB,
    status activity_status NOT NULL DEFAULT 'pending',
    scheduled_for TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    timeout_duration INTERVAL NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Prevent duplicate scheduling (idempotency)
    UNIQUE(workflow_id, activity_key)
);

-- Index for worker polling (hot path) - covers both pending and running activities
-- Note: We can't use NOW() in the WHERE clause as it's not immutable
-- The query will filter on scheduled_for at runtime
CREATE INDEX idx_queue_claimable
ON activity_queue(worker, name, status, scheduled_for)
WHERE status IN ('pending', 'running');

-- Index for timeout queries (stale activity detection)
-- Note: Expression index with (claimed_at + timeout_duration) would require custom immutable function
-- For MVP, we use simple indexes and let the query planner handle the expression
CREATE INDEX idx_queue_timeout_check
ON activity_queue(status, claimed_at)
WHERE status = 'running';

-- Note: idx_queue_workflow removed (profiling showed 0 scans, wasting ~1MB)
-- Workflow activity lookups use the unique constraint on (workflow_id, activity_key)
