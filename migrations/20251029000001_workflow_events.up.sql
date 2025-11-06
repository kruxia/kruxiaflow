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

-- Index for workflow history queries
CREATE INDEX idx_events_workflow_id
ON workflow_events(workflow_id, id DESC);

-- Index for event type filtering
CREATE INDEX idx_events_type
ON workflow_events(event_type, id DESC);

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
    workflow_type TEXT NOT NULL,
    workflow_definition_id UUID NOT NULL REFERENCES workflow_definitions(id),
    status workflow_status NOT NULL DEFAULT 'running',
    state_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for workflow queries
CREATE INDEX idx_workflows_type_status
ON workflows(workflow_type, status, created_at DESC);

-- Index for status queries
CREATE INDEX idx_workflows_status
ON workflows(status, updated_at DESC);

-- Index for workflow definition lookups
CREATE INDEX idx_workflows_definition_id
ON workflows(workflow_definition_id);

-- Create event consumer positions table (durable checkpointing)
CREATE TABLE workflow_event_consumers (
    consumer_id TEXT PRIMARY KEY,
    last_event_id UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
