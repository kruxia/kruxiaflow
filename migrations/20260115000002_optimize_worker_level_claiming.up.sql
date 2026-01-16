-- Optimize idx_queue_claimable for worker-level filtering
-- Remove 'name' column from index since we now filter by worker only, not (worker, name) pairs
-- This allows fair claiming across all activity types for a worker

-- Drop old index with 'name' column
DROP INDEX IF EXISTS idx_queue_claimable;

-- New index optimized for worker-level filtering
-- Column order: (worker, scheduled_for) allows efficient filtering by worker
-- with ORDER BY scheduled_for ASC for fair scheduling across activity types
-- Note: status is NOT included in index columns because:
-- 1. The partial index predicate already filters on status
-- 2. Benchmarking showed equivalent performance with smaller index size
CREATE INDEX idx_queue_claimable
ON activity_queue (worker, scheduled_for)
WHERE status IN ('pending', 'running');
