-- Drop indexes
DROP INDEX IF EXISTS idx_queue_timeout_check;
DROP INDEX IF EXISTS idx_queue_claimable;

-- Drop table
DROP TABLE IF EXISTS activity_queue;

-- Drop enum type
DROP TYPE IF EXISTS activity_status;
