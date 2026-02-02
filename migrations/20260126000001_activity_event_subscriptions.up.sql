-- Add 'waiting' to activity_status enum
-- Note: PostgreSQL doesn't support adding enum values at a specific position,
-- so we add it at the end. The ordering in Rust code matters, not the enum order.
ALTER TYPE activity_status ADD VALUE IF NOT EXISTS 'waiting';

-- Add new event types for activity waiting/signaled
ALTER TYPE workflow_event_type ADD VALUE IF NOT EXISTS 'ActivityWaiting';
ALTER TYPE workflow_event_type ADD VALUE IF NOT EXISTS 'ActivitySignaled';

-- Denormalized from activity_event_subscriptions so workers get signal data
-- directly from claim_next() without joining the subscriptions table.
ALTER TABLE activity_queue ADD COLUMN IF NOT EXISTS signal_data JSONB;

-- Create subscriptions table for activities waiting for signals
CREATE TABLE IF NOT EXISTS activity_event_subscriptions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    activity_key TEXT NOT NULL,
    event_name TEXT NOT NULL,
    on_timeout TEXT NOT NULL CHECK (on_timeout IN ('continue', 'skip', 'fail')),
    timeout_at TIMESTAMPTZ NOT NULL,
    signal_data JSONB,
    expired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(workflow_id, activity_key)
);

-- Index for efficient timeout checking (only check unsignaled, unexpired subscriptions)
CREATE INDEX IF NOT EXISTS idx_subscriptions_timeout
    ON activity_event_subscriptions(timeout_at)
    WHERE signal_data IS NULL AND expired_at IS NULL;

-- Index for efficient lookup when signals arrive
CREATE INDEX IF NOT EXISTS idx_subscriptions_lookup
    ON activity_event_subscriptions(workflow_id, activity_key, event_name);

COMMENT ON TABLE activity_event_subscriptions IS 'Subscriptions for activities waiting for external signals';
COMMENT ON COLUMN activity_event_subscriptions.event_name IS 'The signal event name this activity is waiting for';
COMMENT ON COLUMN activity_event_subscriptions.on_timeout IS 'Action when timeout occurs: continue (run activity), skip, or fail';
COMMENT ON COLUMN activity_event_subscriptions.timeout_at IS 'When to timeout if no signal received';
COMMENT ON COLUMN activity_event_subscriptions.signal_data IS 'Data received with the signal (NULL until signaled)';
COMMENT ON COLUMN activity_event_subscriptions.expired_at IS 'Set when timeout expires, before events are published. Allows crash recovery of expired-but-unprocessed subscriptions.';
