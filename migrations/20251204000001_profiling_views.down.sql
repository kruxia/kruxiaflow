-- US-2.3: PostgreSQL Performance Profiling - Rollback

DROP VIEW IF EXISTS v_long_running_queries;
DROP VIEW IF EXISTS v_connection_stats;
DROP VIEW IF EXISTS v_lock_contention;
DROP VIEW IF EXISTS v_table_stats;
DROP VIEW IF EXISTS v_unused_indexes;
DROP VIEW IF EXISTS v_index_usage;
DROP VIEW IF EXISTS v_slow_queries;

-- Note: Not dropping pg_stat_statements extension as it may be used by other tools
-- DROP EXTENSION IF EXISTS pg_stat_statements;
