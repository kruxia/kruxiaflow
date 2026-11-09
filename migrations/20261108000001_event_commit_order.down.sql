-- Restore the id-ordered cursor. Consumer positions map back through the
-- event they point at; a cursor at (0, 0) or at a vanished event maps to the
-- nil UUID (replay from the beginning).
ALTER TABLE workflow_event_consumers
  ADD COLUMN last_event_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000';
UPDATE workflow_event_consumers c
  SET last_event_id = e.id
  FROM workflow_events e
  WHERE e.tx = c.last_tx AND e.position = c.last_position;
ALTER TABLE workflow_event_consumers ALTER COLUMN last_event_id DROP DEFAULT;
ALTER TABLE workflow_event_consumers DROP COLUMN last_tx, DROP COLUMN last_position;

DROP INDEX idx_events_consumer_poll;
CREATE INDEX idx_events_consumer_poll ON workflow_events (id)
  INCLUDE (workflow_id, event_type, activity_key, timestamp, iteration);

ALTER TABLE workflow_events DROP CONSTRAINT workflow_events_position_key;
ALTER TABLE workflow_events DROP COLUMN tx;
ALTER TABLE workflow_events DROP COLUMN position;
