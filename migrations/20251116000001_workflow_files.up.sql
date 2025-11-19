-- File metadata table for workflow storage
CREATE TABLE workflow_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    filename TEXT NOT NULL,
    oid OID NOT NULL,  -- PostgreSQL Large Object OID
    size BIGINT NOT NULL,
    content_type TEXT,

    UNIQUE(workflow_id, activity_key, filename)
);

-- Index for cleanup queries (BRIN for time-series efficiency)
CREATE INDEX idx_workflow_files_created
    ON workflow_files USING BRIN (created_at);
