use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Profile command - PostgreSQL performance profiling
///
/// Queries pg_stat_statements and PostgreSQL statistics views to analyze
/// database performance. Requires pg_stat_statements extension to be enabled.
#[derive(Args)]
pub struct ProfileCommand {
    /// Number of slow queries to show
    #[arg(
        short = 'n',
        long,
        default_value = "20",
        help = "Number of slow queries to display"
    )]
    pub limit: i64,

    /// Minimum average execution time (ms) to include
    #[arg(
        long,
        default_value = "0.01",
        help = "Minimum average query time in ms"
    )]
    pub min_time_ms: f64,

    /// Output format (text or json)
    #[arg(
        short,
        long,
        default_value = "text",
        help = "Output format (text, json)"
    )]
    pub format: String,

    /// Reset pg_stat_statements before profiling
    #[arg(long, help = "Reset query statistics before profiling")]
    pub reset: bool,

    /// Show execution plans for hot path queries
    #[arg(long, help = "Run EXPLAIN ANALYZE on hot path queries")]
    pub explain: bool,

    /// Verbose output with additional details
    #[arg(short, long, help = "Show additional details")]
    pub verbose: bool,
}

// Query result structs (used for JSON serialization and display)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowQuery {
    pub query_preview: Option<String>,
    pub calls: Option<i64>,
    pub total_time_ms: Option<f64>,
    pub avg_time_ms: Option<f64>,
    pub max_time_ms: Option<f64>,
    pub rows: Option<i64>,
    pub pct_total: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexUsage {
    pub schemaname: Option<String>,
    pub tablename: Option<String>,
    pub indexname: Option<String>,
    pub scans: Option<i64>,
    pub tuples_read: Option<i64>,
    pub tuples_fetched: Option<i64>,
    pub index_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnusedIndex {
    pub schemaname: Option<String>,
    pub tablename: Option<String>,
    pub indexname: Option<String>,
    pub index_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableStats {
    pub schemaname: Option<String>,
    pub tablename: Option<String>,
    pub seq_scan: Option<i64>,
    pub seq_tup_read: Option<i64>,
    pub idx_scan: Option<i64>,
    pub idx_tup_fetch: Option<i64>,
    pub inserts: Option<i64>,
    pub updates: Option<i64>,
    pub deletes: Option<i64>,
    pub live_rows: Option<i64>,
    pub dead_rows: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockContention {
    pub blocked_pid: Option<i32>,
    pub blocked_user: Option<String>,
    pub blocking_pid: Option<i32>,
    pub blocking_user: Option<String>,
    pub blocked_statement: Option<String>,
    pub blocking_statement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStats {
    pub state: Option<String>,
    pub connections: Option<i64>,
    pub applications: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMetrics {
    pub size: u32,
    pub idle: u32,
    pub active: u32,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileReport {
    pub slow_queries: Vec<SlowQuery>,
    pub index_usage: Vec<IndexUsage>,
    pub unused_indexes: Vec<UnusedIndex>,
    pub table_stats: Vec<TableStats>,
    pub lock_contention: Vec<LockContention>,
    pub connection_stats: Vec<ConnectionStats>,
    pub pool_metrics: PoolMetrics,
    pub pg_stat_statements_available: bool,
    pub timestamp: String,
}

/// Execute profile command
pub async fn execute(cmd: ProfileCommand, database_url: String) -> Result<()> {
    // Create database connection pool
    let pool = PgPool::connect(&database_url).await?;

    // Reset stats if requested
    if cmd.reset {
        reset_stats(&pool).await?;
        println!("✅ pg_stat_statements reset");
        return Ok(());
    }

    // Check if pg_stat_statements is available
    let pg_stat_available = check_pg_stat_statements(&pool).await;

    // Gather all profiling data
    let slow_queries = if pg_stat_available {
        get_slow_queries(&pool, cmd.limit, cmd.min_time_ms).await?
    } else {
        vec![]
    };

    let index_usage = get_index_usage(&pool).await?;
    let unused_indexes = get_unused_indexes(&pool).await?;
    let table_stats = get_table_stats(&pool).await?;
    let lock_contention = get_lock_contention(&pool).await?;
    let connection_stats = get_connection_stats(&pool).await?;

    let pool_metrics = PoolMetrics {
        size: pool.size(),
        idle: pool.num_idle() as u32,
        active: pool.size() - pool.num_idle() as u32,
        max_connections: pool.options().get_max_connections(),
    };

    let report = ProfileReport {
        slow_queries,
        index_usage,
        unused_indexes,
        table_stats,
        lock_contention,
        connection_stats,
        pool_metrics,
        pg_stat_statements_available: pg_stat_available,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    // Output results
    match cmd.format.as_str() {
        "json" => print_json_report(&report),
        _ => print_text_report(&report, cmd.verbose),
    }

    // Show EXPLAIN ANALYZE for hot paths if requested
    if cmd.explain {
        println!("\n{:=<80}", "");
        println!("EXPLAIN ANALYZE for Hot Path Queries");
        println!("{:=<80}", "");
        explain_hot_paths(&pool).await?;
    }

    Ok(())
}

/// Check if pg_stat_statements extension is available
async fn check_pg_stat_statements(pool: &PgPool) -> bool {
    sqlx::query!("SELECT 1 as exists FROM pg_extension WHERE extname = 'pg_stat_statements'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Reset pg_stat_statements
async fn reset_stats(pool: &PgPool) -> Result<()> {
    sqlx::query_scalar!("SELECT pg_stat_statements_reset()")
        .fetch_one(pool)
        .await?;
    Ok(())
}

/// Get slow queries from v_slow_queries view
async fn get_slow_queries(pool: &PgPool, limit: i64, min_time_ms: f64) -> Result<Vec<SlowQuery>> {
    // Use the v_slow_queries view created by migration, with additional filtering
    // Cast numeric to float8 for Rust f64 compatibility
    let rows = sqlx::query!(
        r#"
        SELECT
            query_preview,
            calls,
            total_time_ms::float8 as "total_time_ms: f64",
            avg_time_ms::float8 as "avg_time_ms: f64",
            max_time_ms::float8 as "max_time_ms: f64",
            rows,
            pct_total::float8 as "pct_total: f64"
        FROM v_slow_queries
        WHERE avg_time_ms::float8 >= $1 OR $1 = 0
        LIMIT $2
        "#,
        min_time_ms,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SlowQuery {
            query_preview: r.query_preview,
            calls: r.calls,
            total_time_ms: r.total_time_ms,
            avg_time_ms: r.avg_time_ms,
            max_time_ms: r.max_time_ms,
            rows: r.rows,
            pct_total: r.pct_total,
        })
        .collect())
}

/// Get index usage statistics from v_index_usage view
async fn get_index_usage(pool: &PgPool) -> Result<Vec<IndexUsage>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            schemaname::text,
            tablename::text,
            indexname::text,
            scans,
            tuples_read,
            tuples_fetched,
            index_size
        FROM v_index_usage
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| IndexUsage {
            schemaname: r.schemaname,
            tablename: r.tablename,
            indexname: r.indexname,
            scans: r.scans,
            tuples_read: r.tuples_read,
            tuples_fetched: r.tuples_fetched,
            index_size: r.index_size,
        })
        .collect())
}

/// Get unused indexes from v_unused_indexes view
async fn get_unused_indexes(pool: &PgPool) -> Result<Vec<UnusedIndex>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            schemaname::text,
            tablename::text,
            indexname::text,
            index_size
        FROM v_unused_indexes
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| UnusedIndex {
            schemaname: r.schemaname,
            tablename: r.tablename,
            indexname: r.indexname,
            index_size: r.index_size,
        })
        .collect())
}

/// Get table statistics from v_table_stats view
async fn get_table_stats(pool: &PgPool) -> Result<Vec<TableStats>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            schemaname::text,
            tablename::text,
            seq_scan,
            seq_tup_read,
            idx_scan,
            idx_tup_fetch,
            inserts,
            updates,
            deletes,
            live_rows,
            dead_rows
        FROM v_table_stats
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| TableStats {
            schemaname: r.schemaname,
            tablename: r.tablename,
            seq_scan: r.seq_scan,
            seq_tup_read: r.seq_tup_read,
            idx_scan: r.idx_scan,
            idx_tup_fetch: r.idx_tup_fetch,
            inserts: r.inserts,
            updates: r.updates,
            deletes: r.deletes,
            live_rows: r.live_rows,
            dead_rows: r.dead_rows,
        })
        .collect())
}

/// Get current lock contention from v_lock_contention view
async fn get_lock_contention(pool: &PgPool) -> Result<Vec<LockContention>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            blocked_pid,
            blocked_user::text,
            blocking_pid,
            blocking_user::text,
            blocked_statement,
            blocking_statement
        FROM v_lock_contention
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| LockContention {
            blocked_pid: r.blocked_pid,
            blocked_user: r.blocked_user,
            blocking_pid: r.blocking_pid,
            blocking_user: r.blocking_user,
            blocked_statement: r.blocked_statement,
            blocking_statement: r.blocking_statement,
        })
        .collect())
}

/// Get connection statistics from v_connection_stats view
async fn get_connection_stats(pool: &PgPool) -> Result<Vec<ConnectionStats>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            state,
            connections,
            applications
        FROM v_connection_stats
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ConnectionStats {
            state: r.state,
            connections: r.connections,
            applications: r.applications,
        })
        .collect())
}

/// Run EXPLAIN ANALYZE on hot path queries
async fn explain_hot_paths(pool: &PgPool) -> Result<()> {
    // Event poll query explanation - scalar subquery pattern
    // The scalar subquery is evaluated ONCE and used as a constant in the Index Condition
    println!("\n📊 Event Poll Query (orchestrator hot path):");
    println!("{:-<80}", "");

    let explain_result = sqlx::query_scalar!(
        r#"
        EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
        SELECT id, workflow_id, event_type, activity_key, payload, timestamp, iteration
        FROM workflow_events
        WHERE id > COALESCE(
            (SELECT last_event_id FROM workflow_event_consumers WHERE consumer_id = 'explain_test'),
            '00000000-0000-0000-0000-000000000000'::uuid
        )
        ORDER BY id ASC
        LIMIT 100
        "#
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for l in explain_result.into_iter().flatten() {
        println!("  {}", l);
    }

    // Activity claim_next query explanation
    println!("\n📊 Activity Claim Query (worker hot path):");
    println!("{:-<80}", "");

    // Note: We can't run the actual UPDATE with EXPLAIN without modifying data,
    // so we explain the SELECT portion which is the expensive part
    let explain_result = sqlx::query_scalar!(
        r#"
        EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
        SELECT id FROM activity_queue
        WHERE worker = 'std'
          AND name = 'echo'
          AND (
              (status = 'pending'::activity_status AND scheduled_for <= NOW())
              OR
              (status = 'running'::activity_status
               AND NOW() > claimed_at + timeout_duration
               AND retry_count < max_retries)
          )
        ORDER BY scheduled_for ASC
        LIMIT 1
        FOR UPDATE SKIP LOCKED
        "#
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for l in explain_result.into_iter().flatten() {
        println!("  {}", l);
    }

    // Load workflow state query explanation
    println!("\n📊 Load Workflow State Query:");
    println!("{:-<80}", "");

    let explain_result = sqlx::query_scalar!(
        r#"
        EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
        SELECT id, definition_name, status, activities, state_data, input
        FROM workflows
        LIMIT 1
        "#
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for l in explain_result.into_iter().flatten() {
        println!("  {}", l);
    }

    Ok(())
}

/// Print text report
fn print_text_report(report: &ProfileReport, verbose: bool) {
    println!("PostgreSQL Performance Profile");
    println!("{:=<80}", "");
    println!("Timestamp: {}", report.timestamp);

    // pg_stat_statements availability
    if !report.pg_stat_statements_available {
        println!("\n⚠️  WARNING: pg_stat_statements extension not available");
        println!("   Query statistics are not collected.");
        println!("   To enable, add to postgresql.conf:");
        println!("     shared_preload_libraries = 'pg_stat_statements'");
        println!("     pg_stat_statements.track = all");
    }

    // Slow Queries Section
    println!("\n📊 Slow Queries (by total time)");
    println!("{:-<80}", "");

    if report.slow_queries.is_empty() {
        println!("  No queries recorded (run some workload first)");
    } else {
        println!(
            "{:<6} {:>10} {:>10} {:>10} {:>8} {:<30}",
            "Calls", "Total(ms)", "Avg(ms)", "Max(ms)", "% Total", "Query"
        );
        println!("{:-<80}", "");

        for q in &report.slow_queries {
            let preview = q.query_preview.as_deref().unwrap_or("").replace('\n', " ");
            let preview = if preview.len() > 28 {
                format!("{}...", &preview[..28])
            } else {
                preview
            };

            println!(
                "{:<6} {:>10.2} {:>10.2} {:>10.2} {:>7.1}% {:<30}",
                q.calls.unwrap_or(0),
                q.total_time_ms.unwrap_or(0.0),
                q.avg_time_ms.unwrap_or(0.0),
                q.max_time_ms.unwrap_or(0.0),
                q.pct_total.unwrap_or(0.0),
                preview
            );
        }

        // Identify slow queries (>10ms average)
        let slow_count = report
            .slow_queries
            .iter()
            .filter(|q| q.avg_time_ms.unwrap_or(0.0) > 10.0)
            .count();
        if slow_count > 0 {
            println!(
                "\n⚠️  {} queries have avg execution time > 10ms",
                slow_count
            );
        }
    }

    // Index Usage Section
    println!("\n📊 Index Usage");
    println!("{:-<80}", "");
    println!(
        "{:<30} {:>10} {:>12} {:>10}",
        "Index", "Scans", "Tuples Read", "Size"
    );
    println!("{:-<80}", "");

    for idx in &report.index_usage {
        let name = idx.indexname.as_deref().unwrap_or("");
        let display_name = if name.len() > 28 {
            format!("{}...", &name[..28])
        } else {
            name.to_string()
        };

        let symbol = if idx.scans.unwrap_or(0) > 0 {
            "✅"
        } else {
            "⚠️"
        };

        println!(
            "{} {:<28} {:>10} {:>12} {:>10}",
            symbol,
            display_name,
            idx.scans.unwrap_or(0),
            idx.tuples_read.unwrap_or(0),
            idx.index_size.as_deref().unwrap_or("-")
        );
    }

    // Unused Indexes Warning
    if !report.unused_indexes.is_empty() {
        println!("\n⚠️  Unused Indexes (candidates for removal):");
        for idx in &report.unused_indexes {
            println!(
                "   - {}.{} ({})",
                idx.tablename.as_deref().unwrap_or("?"),
                idx.indexname.as_deref().unwrap_or("?"),
                idx.index_size.as_deref().unwrap_or("?")
            );
        }
    }

    // Table Statistics Section
    if verbose {
        println!("\n📊 Table Statistics");
        println!("{:-<80}", "");
        println!(
            "{:<30} {:>8} {:>8} {:>10} {:>10} {:>10}",
            "Table", "SeqScan", "IdxScan", "Inserts", "Updates", "Live Rows"
        );
        println!("{:-<80}", "");

        for stat in &report.table_stats {
            let seq_scan = stat.seq_scan.unwrap_or(0);
            let idx_scan = stat.idx_scan.unwrap_or(0);

            // Warn if sequential scans are high relative to index scans
            let symbol = if seq_scan > 0 && idx_scan == 0 {
                "⚠️"
            } else {
                "  "
            };

            println!(
                "{} {:<18} {:>8} {:>8} {:>10} {:>10} {:>10}",
                symbol,
                stat.tablename.as_deref().unwrap_or("?"),
                seq_scan,
                idx_scan,
                stat.inserts.unwrap_or(0),
                stat.updates.unwrap_or(0),
                stat.live_rows.unwrap_or(0)
            );
        }
    }

    // Lock Contention Section
    println!("\n📊 Lock Contention");
    println!("{:-<80}", "");

    if report.lock_contention.is_empty() {
        println!("✅ No blocking locks detected");
    } else {
        println!(
            "❌ {} blocking locks detected:",
            report.lock_contention.len()
        );
        for lock in &report.lock_contention {
            println!(
                "   PID {} blocked by PID {}",
                lock.blocked_pid.unwrap_or(0),
                lock.blocking_pid.unwrap_or(0)
            );
            if verbose {
                println!(
                    "      Blocked: {}",
                    lock.blocked_statement
                        .as_deref()
                        .unwrap_or("?")
                        .chars()
                        .take(60)
                        .collect::<String>()
                );
                println!(
                    "      Blocking: {}",
                    lock.blocking_statement
                        .as_deref()
                        .unwrap_or("?")
                        .chars()
                        .take(60)
                        .collect::<String>()
                );
            }
        }
    }

    // Connection Pool Section
    println!("\n📊 Connection Pool");
    println!("{:-<80}", "");
    println!(
        "Size: {} | Active: {} | Idle: {} | Max: {}",
        report.pool_metrics.size,
        report.pool_metrics.active,
        report.pool_metrics.idle,
        report.pool_metrics.max_connections
    );

    let utilization = if report.pool_metrics.max_connections > 0 {
        (report.pool_metrics.active as f64 / report.pool_metrics.max_connections as f64) * 100.0
    } else {
        0.0
    };

    if utilization > 80.0 {
        println!(
            "⚠️  Pool utilization high: {:.1}% - consider increasing max_connections",
            utilization
        );
    } else {
        println!("✅ Pool utilization healthy: {:.1}%", utilization);
    }

    // Database Connection Stats
    if verbose && !report.connection_stats.is_empty() {
        println!("\nDatabase Connections by State:");
        for stat in &report.connection_stats {
            println!(
                "   {}: {} connections ({})",
                stat.state.as_deref().unwrap_or("unknown"),
                stat.connections.unwrap_or(0),
                stat.applications.as_deref().unwrap_or("-")
            );
        }
    }

    println!("\n{:=<80}", "");
}

/// Print JSON report
fn print_json_report(report: &ProfileReport) {
    println!("{}", serde_json::to_string_pretty(report).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_command_defaults() {
        let cmd = ProfileCommand {
            limit: 20,
            min_time_ms: 0.01,
            format: "text".to_string(),
            reset: false,
            explain: false,
            verbose: false,
        };

        assert_eq!(cmd.limit, 20);
        assert_eq!(cmd.format, "text");
        assert!(!cmd.reset);
    }

    #[test]
    fn test_pool_metrics_serialization() {
        let metrics = PoolMetrics {
            size: 10,
            idle: 5,
            active: 5,
            max_connections: 20,
        };

        let json = serde_json::to_string(&metrics).unwrap();
        assert!(json.contains("\"size\":10"));
        assert!(json.contains("\"max_connections\":20"));
    }
}
