DROP INDEX IF EXISTS idx_workflows_running_timeout_at;

ALTER TABLE workflows DROP COLUMN IF EXISTS timeout_at;
