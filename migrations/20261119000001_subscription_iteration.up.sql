-- The loop round a signal subscription waits in (NULL outside loops).
--
-- A subscription is one wait. Inside a loop an activity waits once per
-- round, and the ActivitySignaled / timeout events for each round must carry
-- that round's iteration: the event dedup index
-- (workflow_id, event_type, activity_key, iteration, attempt) NULLS NOT
-- DISTINCT otherwise drops every round's signal after the first.
ALTER TABLE activity_event_subscriptions ADD COLUMN iteration INTEGER;

COMMENT ON COLUMN activity_event_subscriptions.iteration IS 'Loop round of the waiting activity (NULL outside loops); stamped on the events this subscription produces';
