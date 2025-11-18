-- Remove output_definitions column from activity_queue
ALTER TABLE activity_queue
DROP COLUMN IF EXISTS output_definitions;
