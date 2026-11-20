-- When the wait took its signal (NULL while waiting). This is the "signaled"
-- marker; signal_data is only the payload and stays NULL for a signal without
-- data. A signal takes only a row neither signaled nor expired, and the expiry
-- checker only an unsignaled row, so a wait ends once (#33).
ALTER TABLE activity_event_subscriptions ADD COLUMN signaled_at TIMESTAMPTZ;

-- Rows with signal_data count as signaled; NOW() stands in for the time (only
-- NULL / NOT NULL is read). A row signaled without data is indistinguishable
-- from a waiting one and stays waiting.
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
