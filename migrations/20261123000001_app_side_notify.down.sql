-- Restore the commit-notification triggers of migrations 20261108000002 and
-- 20261108000004
CREATE FUNCTION workflow_events_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_notify('workflow_events', '');
  RETURN NULL;
END $$;

CREATE TRIGGER workflow_events_notify
  AFTER INSERT ON workflow_events
  FOR EACH STATEMENT EXECUTE FUNCTION workflow_events_notify();

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
