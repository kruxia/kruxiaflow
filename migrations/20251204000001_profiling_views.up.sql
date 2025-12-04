-- US-2.3: PostgreSQL Performance Profiling
-- Creates views for query analysis, index usage, and lock contention detection
--
-- Note: pg_stat_statements extension must be enabled in postgresql.conf:
--   shared_preload_libraries = 'pg_stat_statements'
--   pg_stat_statements.track = all
--   pg_stat_statements.max = 10000

-- Enable pg_stat_statements extension (requires superuser or rds_superuser)
-- This may fail in some environments - that's OK, the views will still work
-- if the extension is pre-configured at the server level
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Top slow queries by total execution time
-- Use: SELECT * FROM v_slow_queries;
CREATE VIEW v_slow_queries AS
SELECT
    substring(query, 1, 100) as query_preview,
    calls,
    round(total_exec_time::numeric, 2) as total_time_ms,
    round(mean_exec_time::numeric, 2) as avg_time_ms,
    round(max_exec_time::numeric, 2) as max_time_ms,
    rows,
    round((100 * total_exec_time / NULLIF(sum(total_exec_time) OVER(), 0))::numeric, 2) as pct_total
FROM pg_stat_statements
ORDER BY total_exec_time DESC
LIMIT 20;

-- Index usage analysis
-- Use: SELECT * FROM v_index_usage;
CREATE VIEW v_index_usage AS
SELECT
    schemaname,
    relname as tablename,
    indexrelname as indexname,
    idx_scan as scans,
    idx_tup_read as tuples_read,
    idx_tup_fetch as tuples_fetched,
    pg_size_pretty(pg_relation_size(indexrelid)) as index_size
FROM pg_stat_user_indexes
ORDER BY idx_scan DESC;

-- Unused indexes (candidates for removal)
-- Use: SELECT * FROM v_unused_indexes;
CREATE VIEW v_unused_indexes AS
SELECT
    schemaname,
    relname as tablename,
    indexrelname as indexname,
    pg_size_pretty(pg_relation_size(indexrelid)) as index_size
FROM pg_stat_user_indexes
WHERE idx_scan = 0
AND indexrelname NOT LIKE '%_pkey'
ORDER BY pg_relation_size(indexrelid) DESC;

-- Table statistics (scans, updates, live/dead rows)
-- Use: SELECT * FROM v_table_stats;
CREATE VIEW v_table_stats AS
SELECT
    schemaname,
    relname as tablename,
    seq_scan,
    seq_tup_read,
    idx_scan,
    idx_tup_fetch,
    n_tup_ins as inserts,
    n_tup_upd as updates,
    n_tup_del as deletes,
    n_live_tup as live_rows,
    n_dead_tup as dead_rows
FROM pg_stat_user_tables
ORDER BY seq_scan + idx_scan DESC;

-- Lock contention detection
-- Use: SELECT * FROM v_lock_contention; (during load test)
CREATE VIEW v_lock_contention AS
SELECT
    blocked_locks.pid AS blocked_pid,
    blocked_activity.usename AS blocked_user,
    blocking_locks.pid AS blocking_pid,
    blocking_activity.usename AS blocking_user,
    blocked_activity.query AS blocked_statement,
    blocking_activity.query AS blocking_statement
FROM pg_catalog.pg_locks blocked_locks
JOIN pg_catalog.pg_stat_activity blocked_activity
    ON blocked_activity.pid = blocked_locks.pid
JOIN pg_catalog.pg_locks blocking_locks
    ON blocking_locks.locktype = blocked_locks.locktype
    AND blocking_locks.database IS NOT DISTINCT FROM blocked_locks.database
    AND blocking_locks.relation IS NOT DISTINCT FROM blocked_locks.relation
    AND blocking_locks.page IS NOT DISTINCT FROM blocked_locks.page
    AND blocking_locks.tuple IS NOT DISTINCT FROM blocked_locks.tuple
    AND blocking_locks.virtualxid IS NOT DISTINCT FROM blocked_locks.virtualxid
    AND blocking_locks.transactionid IS NOT DISTINCT FROM blocked_locks.transactionid
    AND blocking_locks.classid IS NOT DISTINCT FROM blocked_locks.classid
    AND blocking_locks.objid IS NOT DISTINCT FROM blocked_locks.objid
    AND blocking_locks.objsubid IS NOT DISTINCT FROM blocked_locks.objsubid
    AND blocking_locks.pid != blocked_locks.pid
JOIN pg_catalog.pg_stat_activity blocking_activity
    ON blocking_activity.pid = blocking_locks.pid
WHERE NOT blocked_locks.granted;

-- Active connections by state
-- Use: SELECT * FROM v_connection_stats;
CREATE VIEW v_connection_stats AS
SELECT
    state,
    count(*) as connections,
    string_agg(DISTINCT application_name, ', ') as applications
FROM pg_stat_activity
WHERE datname = current_database()
GROUP BY state;

-- Long-running queries (potential pool exhaustion)
-- Use: SELECT * FROM v_long_running_queries;
CREATE VIEW v_long_running_queries AS
SELECT
    pid,
    now() - pg_stat_activity.query_start AS duration,
    query,
    state
FROM pg_stat_activity
WHERE (now() - pg_stat_activity.query_start) > interval '5 seconds'
AND state != 'idle';
