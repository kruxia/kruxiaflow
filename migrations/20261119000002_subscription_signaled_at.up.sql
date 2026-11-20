-- When a signal wait took its signal (NULL while it still waits).
--
-- Why: "signaled" used to be read off signal_data IS NULL, but signal_data is
-- the payload, and a signal sent without data stores SQL NULL there. Such a
-- wait still looked unsignaled until the orchestrator deleted its row:
--   - a second signal was accepted (signaled: true) and its data was lost,
--     because its ActivitySignaled event is dropped by the event dedup index;
--   - the expiry checker could still expire the wait, so with
--     on_timeout: fail a step whose signal arrived in time failed (#33).
-- signaled_at is now the marker. A signal matches only a row that is neither
-- signaled nor expired, and the expiry checker only a row not signaled, so a
-- wait ends once: by its signal or by its timeout.
ALTER TABLE activity_event_subscriptions ADD COLUMN signaled_at TIMESTAMPTZ;

-- Rows already signaled with data count as signaled. The signal time was not
-- recorded; NOW() stands in for it (only NULL / NOT NULL is ever read). A row
-- signaled without data cannot be told from a waiting one and stays waiting,
-- as it already behaved.
UPDATE activity_event_subscriptions
SET signaled_at = NOW()
WHERE signal_data IS NOT NULL;

-- The expiry checker's index covers open waits only: not signaled, not expired
DROP INDEX IF EXISTS idx_subscriptions_timeout;
CREATE INDEX idx_subscriptions_timeout
    ON activity_event_subscriptions(timeout_at)
    WHERE signaled_at IS NULL AND expired_at IS NULL;

COMMENT ON COLUMN activity_event_subscriptions.signaled_at IS 'When the wait took its signal (NULL while waiting); the "signaled" marker, since signal_data stays NULL for a signal without data';
COMMENT ON COLUMN activity_event_subscriptions.signal_data IS 'Data received with the signal (NULL until signaled, and after a signal without data)';
