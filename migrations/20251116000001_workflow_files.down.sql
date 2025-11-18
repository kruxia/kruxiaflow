-- Note: Drop table does NOT automatically clean up Large Objects.
-- They must be unlinked before dropping the table to avoid orphans.
DO $$
DECLARE
    file_record RECORD;
BEGIN
    -- Delete all Large Objects before dropping table
    FOR file_record IN SELECT oid FROM workflow_files
    LOOP
        PERFORM lo_unlink(file_record.oid);
    END LOOP;
END $$;

DROP TABLE IF EXISTS workflow_files;
