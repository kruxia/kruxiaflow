-- Revert to original unique constraint (without iteration support)
ALTER TABLE activity_queue DROP CONSTRAINT activity_queue_workflow_id_activity_key_iteration_key;

ALTER TABLE activity_queue ADD CONSTRAINT activity_queue_workflow_id_activity_key_key
    UNIQUE(workflow_id, activity_key);

-- Remove iteration column from activity_queue
ALTER TABLE activity_queue DROP COLUMN iteration;
