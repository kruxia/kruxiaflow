-- Per-workflow deadline from the definition's top-level settings.timeout
-- (seconds), fixed at submission: timeout_at = created_at + timeout. NULL
-- means the workflow has no timeout and runs until it completes or fails;
-- there is no server-wide workflow timeout.
--
-- The orchestrator's timeout checker reads only the partial index, so its
-- cost follows the number of running workflows that HAVE a deadline, not all
-- running workflows (which may include workflows meant to run for years).
ALTER TABLE workflows ADD COLUMN timeout_at TIMESTAMPTZ;

-- Unfinished workflows whose definition sets a timeout
UPDATE workflows w
SET timeout_at = w.created_at + make_interval(secs => (d.settings->>'timeout')::float8)
FROM workflow_definitions d
WHERE d.id = w.workflow_definition_id
  AND w.status NOT IN ('completed', 'failed')
  AND jsonb_typeof(d.settings->'timeout') = 'number';

CREATE INDEX idx_workflows_running_timeout_at
ON workflows(timeout_at)
WHERE status = 'running' AND timeout_at IS NOT NULL;
