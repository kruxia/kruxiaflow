-- Workflow-LEVEL failure reason (e.g. "Workflow timeout ... never claimed").
-- Activity-level failures carry their reason on the failed activity inside
-- the activities JSONB; API read paths prefer that and fall back to this
-- column, so a dead-letter is always self-explaining even when no activity
-- ever ran (nukumori-support-needs item 12).
ALTER TABLE workflows ADD COLUMN error_message TEXT;
