-- Drop subscriptions table
DROP TABLE IF EXISTS activity_event_subscriptions;

-- Note: PostgreSQL does not support removing enum values directly.
-- The 'waiting' status and 'ActivityWaiting'/'ActivitySignaled' event types
-- will remain in the enum but won't be used after rollback.
-- A full cleanup would require recreating the enum types.
