-- Restore the attempt-blind dedup constraint. Fails if per-attempt duplicate
-- events exist; delete the later attempts' events first.
DROP INDEX workflow_events_dedup_idx;

ALTER TABLE workflow_events
ADD CONSTRAINT workflow_events_workflow_id_event_type_activity_key_iteration_k
UNIQUE NULLS NOT DISTINCT (workflow_id, event_type, activity_key, iteration);
