-- Drop indexes first
DROP INDEX IF EXISTS idx_events_consumer_poll;

-- Revert to original unique constraint (without iteration support)
ALTER TABLE workflow_events DROP CONSTRAINT workflow_events_workflow_id_event_type_activity_key_iteration_key;

ALTER TABLE workflow_events ADD CONSTRAINT workflow_events_workflow_id_event_type_activity_key_key
    UNIQUE(workflow_id, event_type, activity_key);

-- Remove iteration column
ALTER TABLE workflow_events DROP COLUMN iteration;
