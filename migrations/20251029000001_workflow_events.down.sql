-- Drop tables
DROP TABLE workflow_event_consumers CASCADE;
DROP TABLE workflow_events CASCADE;
DROP TABLE workflows CASCADE;
DROP TABLE workflow_definitions CASCADE;

-- Drop enums
DROP TYPE workflow_event_type;
DROP TYPE workflow_status;
