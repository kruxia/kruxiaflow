-- Back to signal_data as the only "signaled" marker. A wait signaled without
-- data keeps counting as signaled: its signal_data becomes JSON null, which
-- is not SQL NULL. (The orchestrator takes a signal's data from its
-- ActivitySignaled event, not from this row.)
UPDATE activity_event_subscriptions
SET signal_data = 'null'::jsonb
WHERE signaled_at IS NOT NULL AND signal_data IS NULL;

DROP INDEX IF EXISTS idx_subscriptions_timeout;
CREATE INDEX idx_subscriptions_timeout
    ON activity_event_subscriptions(timeout_at)
    WHERE signal_data IS NULL AND expired_at IS NULL;

COMMENT ON COLUMN activity_event_subscriptions.signal_data IS 'Data received with the signal (NULL until signaled)';

ALTER TABLE activity_event_subscriptions DROP COLUMN IF EXISTS signaled_at;
