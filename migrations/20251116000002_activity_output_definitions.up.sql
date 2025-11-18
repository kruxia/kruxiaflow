-- Add output_definitions column to activity_queue to support file outputs
ALTER TABLE activity_queue
ADD COLUMN output_definitions JSONB;
