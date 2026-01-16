-- Revert: Restore original idx_queue_claimable index with 'name' column
DROP INDEX IF EXISTS idx_queue_claimable;

CREATE INDEX idx_queue_claimable
ON activity_queue (worker, name, status, scheduled_for)
WHERE status IN ('pending', 'running');
