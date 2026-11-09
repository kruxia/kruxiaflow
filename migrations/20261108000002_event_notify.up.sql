-- Wake event consumers when new events commit
-- (docs/implementation/2026-11-08-commit-order-cursor-and-notify-wake.md §2.2).
--
-- NOTIFY is queued inside the inserting transaction and delivered at commit,
-- to every connection listening at that moment; a rolled-back insert never
-- notifies, and identical notifications in one transaction collapse into one.
-- The notification is a wake-up only: the payload is empty, consumers always
-- poll from their cursor, and they also poll on a timer, so a notification
-- lost on a dropped connection only delays delivery.
CREATE FUNCTION workflow_events_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_notify('workflow_events', '');
  RETURN NULL;
END $$;

CREATE TRIGGER workflow_events_notify
  AFTER INSERT ON workflow_events
  FOR EACH STATEMENT EXECUTE FUNCTION workflow_events_notify();
