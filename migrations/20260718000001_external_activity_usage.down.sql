-- Revert external activity usage reporting changes.
-- Rows written with NULL provider/model (lump-sum and non-LLM line items)
-- are backfilled with 'unknown' so the NOT NULL constraints can be restored.
UPDATE activity_costs SET provider = 'unknown' WHERE provider IS NULL;
UPDATE activity_costs SET model = 'unknown' WHERE model IS NULL;

ALTER TABLE activity_costs
    ALTER COLUMN provider SET NOT NULL,
    ALTER COLUMN model SET NOT NULL;

ALTER TABLE workflow_definitions
    DROP COLUMN settings;
