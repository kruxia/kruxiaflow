-- Commit notifications come from the application, not from triggers
-- (core/src/events/wake.rs, `Announcer`).
--
-- These two triggers sent a NOTIFY from inside every transaction that wrote an
-- event or a claimable activity. PostgreSQL serializes the commits of
-- notifying transactions on one database-wide lock, held through each
-- commit's WAL flush, so under load those commits queued behind one another.
-- Each process now announces its own writes after they commit, wakes its own
-- consumers in memory, and sends the other processes one NOTIFY per batch.
--
-- The notifications keep their channels and payloads, so a listener of
-- either version is woken by a writer of the new one. A writer of an older
-- version wakes nobody once this has run, and its rows are picked up on the
-- consumers' poll timers until it is replaced.
DROP TRIGGER IF EXISTS workflow_events_notify ON workflow_events;
DROP FUNCTION IF EXISTS workflow_events_notify();

DROP TRIGGER IF EXISTS activity_queue_notify ON activity_queue;
DROP FUNCTION IF EXISTS activity_queue_notify();
