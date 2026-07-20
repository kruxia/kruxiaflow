-- A failed (dead-lettered) workflow must not hold its unique_key forever:
-- dedup applies only to non-failed workflows, so a permanently failed
-- submission can be resubmitted under the same key. The unconditional column
-- constraint is replaced by a partial unique index that excludes failed
-- workflows. Multiple failed rows may share a key; at most one non-failed row
-- can hold it.
ALTER TABLE workflows DROP CONSTRAINT workflows_unique_key_key;

CREATE UNIQUE INDEX idx_workflows_unique_key_active
ON workflows(unique_key)
WHERE unique_key IS NOT NULL AND status <> 'failed';
