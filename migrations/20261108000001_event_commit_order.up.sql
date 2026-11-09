-- Commit-safe event cursor (docs/implementation/2026-11-08-commit-order-cursor-and-notify-wake.md)
--
-- `id` (UUIDv7) is assigned when an INSERT executes, but rows become visible
-- in commit order, so a consumer reading `id > cursor` can step past an event
-- whose transaction commits late and never see it. Two columns replace it as
-- the read order:
--
--   tx        the inserting transaction (instance-wide, never wraps)
--   position  insert order within the table
--
-- Consumers read `(tx, position) > cursor AND tx < pg_snapshot_xmin(...)`
-- ordered by (tx, position). Every transaction below xmin has finished, so
-- the set of rows that predicate admits never grows behind the cursor: any row
-- that becomes visible later belongs to a transaction at or above xmin, which
-- sorts after everything returned so far. Ordering by `position` alone is not
-- enough: a transaction can take its xid before another one and insert its
-- event after it, giving a lower tx a higher position.

ALTER TABLE workflow_events ADD COLUMN position bigint;
UPDATE workflow_events SET position = r.n
  FROM (SELECT id, row_number() OVER (ORDER BY id) AS n FROM workflow_events) r
  WHERE workflow_events.id = r.id;
ALTER TABLE workflow_events ALTER COLUMN position SET NOT NULL;
ALTER TABLE workflow_events ALTER COLUMN position ADD GENERATED ALWAYS AS IDENTITY;
SELECT setval(pg_get_serial_sequence('workflow_events', 'position'),
              COALESCE((SELECT max(position) FROM workflow_events), 0) + 1, false);
ALTER TABLE workflow_events ADD CONSTRAINT workflow_events_position_key UNIQUE (position);

-- Existing rows take tx 0: below every transaction id the cluster will ever
-- assign, so they are deliverable at once and sort before every new event,
-- and among themselves they keep the id order consumers already saw (the
-- position backfill above). A constant default is stored in the catalog, so
-- this does not rewrite the table; new rows then record their transaction.
ALTER TABLE workflow_events ADD COLUMN tx xid8 NOT NULL DEFAULT '0';
ALTER TABLE workflow_events ALTER COLUMN tx SET DEFAULT pg_current_xact_id();

-- Covering index for the consumer poll (orchestrator hot path). Payload is
-- not included (too large); it is fetched from the heap for the LIMIT rows.
DROP INDEX idx_events_consumer_poll;
CREATE INDEX idx_events_consumer_poll ON workflow_events (tx, position)
  INCLUDE (id, workflow_id, event_type, activity_key, timestamp, iteration);

-- Consumer cursors move from last_event_id to (last_tx, last_position).
-- A cursor whose event no longer exists restarts from the beginning; event
-- handlers are replay-idempotent, so that is slow rather than wrong.
ALTER TABLE workflow_event_consumers
  ADD COLUMN last_tx xid8 NOT NULL DEFAULT '0',
  ADD COLUMN last_position bigint NOT NULL DEFAULT 0;
UPDATE workflow_event_consumers c
  SET last_tx = e.tx, last_position = e.position
  FROM workflow_events e
  WHERE e.id = c.last_event_id;
ALTER TABLE workflow_event_consumers DROP COLUMN last_event_id;
