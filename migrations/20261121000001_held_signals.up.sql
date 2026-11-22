-- Signals that arrived before their activity was waiting (#42).
--
-- A wait with `wait_for_signal.hold_early: true` keeps such a signal here
-- instead of refusing it. When the activity next waits, the orchestrator
-- takes the oldest held signal for it and runs the activity with that
-- signal's data, without waiting.
--
-- The signal API and the orchestrator serialize on a transaction advisory
-- lock keyed by (workflow_id, activity_key), so a signal is either taken by
-- the open wait or held where the next wait finds it, never lost in between.
CREATE TABLE IF NOT EXISTS held_signals (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    activity_key TEXT NOT NULL,
    event_name TEXT NOT NULL,
    signal_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Oldest first, per waiting activity and event
CREATE INDEX IF NOT EXISTS idx_held_signals_next
    ON held_signals(workflow_id, activity_key, event_name, created_at, id);

COMMENT ON TABLE held_signals IS 'Signals held for an activity that was not waiting yet (wait_for_signal.hold_early); delivered oldest first when it next waits';
COMMENT ON COLUMN held_signals.event_name IS 'The signal event name, matching the activity''s wait_for_signal.event_name';
COMMENT ON COLUMN held_signals.signal_data IS 'Data sent with the signal (NULL when it had none)';
