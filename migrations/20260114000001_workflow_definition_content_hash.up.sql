-- Add content_hash column for idempotent workflow deployment
-- The hash enables detecting duplicate definitions without comparing full JSONB content
-- Uses BYTEA (32 bytes) instead of TEXT (64 bytes hex) for 50% storage savings

ALTER TABLE workflow_definitions
ADD COLUMN content_hash BYTEA;

-- Index for fast lookup by name and content_hash
-- Used by idempotent deploy to check if identical definition already exists
CREATE INDEX idx_workflow_definitions_name_hash
ON workflow_definitions(name, content_hash)
WHERE content_hash IS NOT NULL;
