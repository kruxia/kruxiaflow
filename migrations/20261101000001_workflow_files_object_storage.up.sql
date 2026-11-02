-- Object storage (S3-compatible) backend for WorkflowStorage.
--
-- File metadata stays in workflow_files for every provider; only the content
-- location differs. Each row holds exactly one location:
--   oid          - PostgreSQL Large Object (postgresql provider)
--   storage_key  - object key within the configured bucket (s3 provider)
-- Keeping both providers in one table lets a deployment switch providers
-- without stranding existing files: the s3 provider still reads and deletes
-- Large Object rows written before the switch.
ALTER TABLE workflow_files ALTER COLUMN oid DROP NOT NULL;
ALTER TABLE workflow_files ADD COLUMN storage_key TEXT;
ALTER TABLE workflow_files ADD CONSTRAINT workflow_files_single_location
    CHECK ((oid IS NULL) <> (storage_key IS NULL));
