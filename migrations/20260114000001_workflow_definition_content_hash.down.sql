-- Remove content_hash column and index
DROP INDEX IF EXISTS idx_workflow_definitions_name_hash;
ALTER TABLE workflow_definitions DROP COLUMN IF EXISTS content_hash;
