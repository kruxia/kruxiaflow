-- Rows stored in object storage cannot be represented once oid is NOT NULL
-- again, so their metadata is removed. The objects themselves stay in the
-- bucket (this migration has no object storage client); remove them with the
-- bucket's own tools if needed.
DELETE FROM workflow_files WHERE storage_key IS NOT NULL;
ALTER TABLE workflow_files DROP CONSTRAINT workflow_files_single_location;
ALTER TABLE workflow_files DROP COLUMN storage_key;
ALTER TABLE workflow_files ALTER COLUMN oid SET NOT NULL;
