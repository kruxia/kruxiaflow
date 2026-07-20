-- Restore the unconditional unique constraint. Fails if multiple workflows
-- (e.g. a failed run and its resubmission) share a unique_key; those
-- duplicates must be resolved manually before downgrading.
DROP INDEX idx_workflows_unique_key_active;

ALTER TABLE workflows ADD CONSTRAINT workflows_unique_key_key UNIQUE (unique_key);
