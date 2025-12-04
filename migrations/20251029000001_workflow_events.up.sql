-- Create enums first
CREATE TYPE workflow_event_type AS ENUM (
    'WorkflowCreated',
    'WorkflowUpdated',
    'ActivityScheduled',
    'ActivityCompleted',
    'ActivityFailed',
    'WorkflowCompleted',
    'WorkflowFailed'
);

CREATE TYPE workflow_status AS ENUM (
    'created',
    'running',
    'completed',
    'failed',
    'paused'
);

-- Create workflow events table
CREATE TABLE workflow_events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    event_type workflow_event_type NOT NULL,
    activity_key TEXT,
    payload JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Idempotency: prevent duplicate events for same workflow+type+activity
    UNIQUE(workflow_id, event_type, activity_key)
);

-- Removed indexes (profiling showed 0 scans, wasting ~8MB):
--   - idx_events_workflow_id (workflow_id, id DESC) - workflow history queries not used
--   - idx_events_type (event_type, id DESC) - event type filtering not used

-- Create workflow_definitions table first (referenced by workflows)
CREATE TABLE workflow_definitions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL,
    activities JSONB NOT NULL,  -- Store only activities array, not full definition
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(name, created_at)  -- created_at IS the version (microsecond precision prevents collisions)
);

-- BRIN index for created_at (optimal for append-only workload with timestamp correlation)
-- Used by get_latest() queries that ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS idx_workflow_definitions_created_at ON workflow_definitions
USING brin(created_at);

-- Create workflows table
CREATE TABLE workflows (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    definition_name TEXT NOT NULL,
    workflow_definition_id UUID NOT NULL REFERENCES workflow_definitions(id),
    input JSONB NOT NULL,
    unique_key TEXT UNIQUE,
    status workflow_status NOT NULL DEFAULT 'created',
    activities JSONB NOT NULL,
    state_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for workflow queries
CREATE INDEX idx_workflows_definition_status
ON workflows(definition_name, status, created_at DESC);

-- Index for status queries
CREATE INDEX idx_workflows_status
ON workflows(status, updated_at DESC);

-- Note: idx_workflows_definition_id removed (profiling showed 0 scans)
-- Foreign key lookups use the primary key on workflow_definitions

-- Create event consumer positions table (durable checkpointing)
CREATE TABLE workflow_event_consumers (
    consumer_id TEXT PRIMARY KEY,
    last_event_id UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
