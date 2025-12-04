-- Add iteration field to support looping activities.
-- Looping activities need to emit multiple events for the same activity_key
-- This allows each iteration to have its own event while maintaining idempotency.
ALTER TABLE workflow_events ADD COLUMN iteration INTEGER DEFAULT NULL;

-- Drop the old unique constraint
ALTER TABLE workflow_events DROP CONSTRAINT workflow_events_workflow_id_event_type_activity_key_key;

-- Create new unique constraint that includes iteration.
-- NULLS NOT DISTINCT ensures that multiple NULL iterations are NOT allowed
-- This preserves idempotency for non-looping activities (iteration=NULL)
-- For looping activities, each iteration number creates a distinct event.
ALTER TABLE workflow_events ADD CONSTRAINT workflow_events_workflow_id_event_type_activity_key_iteration_key
    UNIQUE NULLS NOT DISTINCT (workflow_id, event_type, activity_key, iteration);

-- Covering index for event consumer polling (orchestrator hot path)
-- The event poll query: SELECT ... WHERE e.id > last_event_id ORDER BY e.id LIMIT N
-- This INCLUDE index allows index-only scans for filtering, reducing heap fetches.
-- Payload is NOT included (too large) - fetched from heap only for the LIMIT N rows.
CREATE INDEX idx_events_consumer_poll ON workflow_events (id)
INCLUDE (workflow_id, event_type, activity_key, timestamp, iteration);
