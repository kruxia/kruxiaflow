-- Event dedup must be attempt-aware. The old UNIQUE(workflow_id, event_type,
-- activity_key, iteration) allowed only ONE ActivityFailed event per activity
-- EVER: the first (retryable) failure occupied the slot and every later
-- failure event — including the terminal one — was silently dropped by
-- publish's ON CONFLICT DO NOTHING, leaving the workflow stuck 'running'
-- until a restart or workflow timeout reconciled it.
--
-- The replacement includes the payload's 'attempt' (set by per-attempt
-- publishers: worker-reported failures, retry scheduling, timeout failures).
-- Every other event type carries no attempt (NULL), and NULLS NOT DISTINCT
-- preserves the existing idempotency semantics for those exactly.
ALTER TABLE workflow_events
DROP CONSTRAINT workflow_events_workflow_id_event_type_activity_key_iteration_k;

CREATE UNIQUE INDEX workflow_events_dedup_idx
ON workflow_events (workflow_id, event_type, activity_key, iteration, ((payload->>'attempt')))
NULLS NOT DISTINCT;
