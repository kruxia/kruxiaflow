-- Add iteration field to activity_queue to support looping activities
ALTER TABLE activity_queue ADD COLUMN iteration INTEGER DEFAULT NULL;

-- Drop old unique constraint on (workflow_id, activity_key)
ALTER TABLE activity_queue DROP CONSTRAINT activity_queue_workflow_id_activity_key_key;

-- Create new unique constraint that includes iteration.
-- NULLS NOT DISTINCT ensures idempotency for non-looping activities (iteration=NULL)
ALTER TABLE activity_queue ADD CONSTRAINT activity_queue_workflow_id_activity_key_iteration_key
    UNIQUE NULLS NOT DISTINCT (workflow_id, activity_key, iteration);
