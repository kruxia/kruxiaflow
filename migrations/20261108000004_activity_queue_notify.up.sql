-- Wake workers when an activity is written as pending
-- (docs/implementation/2026-11-08-activity-wake-sse.md §2.2).
--
-- The notification is delivered when the writing transaction commits, to the
-- API processes listening on the channel, which pass a wake to the workers of
-- that type over GET /api/v1/workers/wake. The payload is the worker type and
-- is only a routing hint: a woken worker always asks the queue what is
-- claimable, and workers also poll on a timer, so a lost notification only
-- delays a claim.
--
-- Fires for rows scheduled for later too (delays, retry backoff): the wake
-- makes the worker poll, and the poll's answer tells it when the row is due.
-- Does not fire for the claim's own update (to running), heartbeats or
-- completions, which the WHEN condition filters out before the function runs.
CREATE FUNCTION activity_queue_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_notify('activity_queue', NEW.worker);
  RETURN NULL;
END $$;

CREATE TRIGGER activity_queue_notify
  AFTER INSERT OR UPDATE OF status, scheduled_for ON activity_queue
  FOR EACH ROW
  WHEN (NEW.status = 'pending'::activity_status)
  EXECUTE FUNCTION activity_queue_notify();
