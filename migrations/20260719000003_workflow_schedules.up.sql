-- First-class recurring schedules: a schedule is an operational resource that
-- submits a workflow on a cadence (cron or fixed interval), server-side — no
-- client credentials ride the recurrence, so nothing stales and schedule
-- death is impossible short of engine death. Replaces app-side workarounds
-- (chaining, bucketed ensure-loops).
CREATE TABLE workflow_schedules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL UNIQUE,
    definition_name TEXT NOT NULL,
    -- NULL = resolve the latest definition version at fire time
    definition_version TEXT,
    input JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Exactly one of cron / interval_seconds (enforced below).
    -- cron is standard 5-field crontab (minute granularity) or 6-field with
    -- leading seconds; timezone applies to cron evaluation only (default UTC).
    cron TEXT,
    timezone TEXT,
    interval_seconds BIGINT,
    -- skip: don't submit while the previous run is still non-terminal
    -- (next_run_at still advances); allow: always submit
    overlap_policy TEXT NOT NULL DEFAULT 'skip'
        CHECK (overlap_policy IN ('skip', 'allow')),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    next_run_at TIMESTAMPTZ NOT NULL,
    last_run_at TIMESTAMPTZ,
    last_workflow_id UUID,
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (cron IS NOT NULL AND interval_seconds IS NULL)
        OR (cron IS NULL AND interval_seconds IS NOT NULL)
    ),
    CHECK (interval_seconds IS NULL OR interval_seconds >= 1),
    CHECK (timezone IS NULL OR cron IS NOT NULL)
);

-- The scheduler tick claims due schedules by next_run_at
CREATE INDEX idx_workflow_schedules_due
ON workflow_schedules(next_run_at)
WHERE enabled;

CREATE TRIGGER trigger_workflow_schedules_updated_at
BEFORE UPDATE ON workflow_schedules
FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
