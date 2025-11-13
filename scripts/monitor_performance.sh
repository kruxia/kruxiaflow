#!/bin/bash
#
# Performance Monitoring Script for StreamFlow
#
# Monitors critical metrics during benchmark execution:
# 1. Memory usage (heap growth)
# 2. Database connection count
# 3. Event consumer positions
# 4. Worker thread count
# 5. Orchestrator backoff state (via log analysis)
#
# Usage:
#   ./scripts/monitor_performance.sh [OPTIONS]
#
# Options:
#   --server-pid PID        PID of StreamFlow server to monitor
#   --db-url URL            PostgreSQL connection URL
#   --duration SECONDS      How long to monitor (default: 60)
#   --interval SECONDS      Sampling interval (default: 2)
#   --output-dir DIR        Directory for output files
#

set -e

# Default options
SERVER_PID=""
DB_URL="${DATABASE_URL:-postgres://streamflow:streamflow_dev@127.0.0.1:5432/streamflow_profiling}"
DURATION=60
INTERVAL=2
OUTPUT_DIR="."

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --server-pid)
            SERVER_PID="$2"
            shift 2
            ;;
        --db-url)
            DB_URL="$2"
            shift 2
            ;;
        --duration)
            DURATION="$2"
            shift 2
            ;;
        --interval)
            INTERVAL="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate required parameters
if [ -z "$SERVER_PID" ]; then
    echo "Error: --server-pid is required"
    exit 1
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Output files
MEMORY_LOG="$OUTPUT_DIR/memory_usage.csv"
DB_CONN_LOG="$OUTPUT_DIR/db_connections.csv"
CONSUMER_POS_LOG="$OUTPUT_DIR/consumer_positions.csv"
THREAD_COUNT_LOG="$OUTPUT_DIR/thread_count.csv"
SYSTEM_STATS_LOG="$OUTPUT_DIR/system_stats.csv"

# Initialize CSV headers
echo "timestamp,rss_mb,vsz_mb,cpu_percent" > "$MEMORY_LOG"
echo "timestamp,total_connections,active,idle,idle_in_transaction,waiting" > "$DB_CONN_LOG"
echo "timestamp,consumer_id,last_event_id,updated_at" > "$CONSUMER_POS_LOG"
echo "timestamp,thread_count" > "$THREAD_COUNT_LOG"
echo "timestamp,db_size_mb,event_count,workflow_count,activity_queue_size" > "$SYSTEM_STATS_LOG"

echo "Starting performance monitoring..."
echo "  Server PID: $SERVER_PID"
echo "  Duration: ${DURATION}s"
echo "  Interval: ${INTERVAL}s"
echo "  Output: $OUTPUT_DIR"
echo ""

# Monitoring loop
END_TIME=$(($(date +%s) + DURATION))

while [ $(date +%s) -lt $END_TIME ]; do
    TIMESTAMP=$(date +"%Y-%m-%d %H:%M:%S")

    # 1. Memory usage (using ps)
    if ps -p $SERVER_PID > /dev/null 2>&1; then
        # macOS ps command
        MEM_INFO=$(ps -p $SERVER_PID -o rss=,vsz=,%cpu= 2>/dev/null || echo "0 0 0.0")
        RSS_KB=$(echo $MEM_INFO | awk '{print $1}')
        VSZ_KB=$(echo $MEM_INFO | awk '{print $2}')
        CPU_PCT=$(echo $MEM_INFO | awk '{print $3}')

        RSS_MB=$(echo "scale=2; $RSS_KB / 1024" | bc)
        VSZ_MB=$(echo "scale=2; $VSZ_KB / 1024" | bc)

        echo "$TIMESTAMP,$RSS_MB,$VSZ_MB,$CPU_PCT" >> "$MEMORY_LOG"
    else
        echo "Warning: Server process $SERVER_PID not found"
        break
    fi

    # 2. Database connections
    DB_CONN_STATS=$(psql "$DB_URL" -t -c "
        SELECT
            COUNT(*) as total,
            COUNT(*) FILTER (WHERE state = 'active') as active,
            COUNT(*) FILTER (WHERE state = 'idle') as idle,
            COUNT(*) FILTER (WHERE state = 'idle in transaction') as idle_in_tx,
            COUNT(*) FILTER (WHERE wait_event IS NOT NULL) as waiting
        FROM pg_stat_activity
        WHERE datname = 'streamflow_profiling'
            AND pid <> pg_backend_pid();
    " 2>/dev/null || echo "0|0|0|0|0")

    echo "$TIMESTAMP,$DB_CONN_STATS" | tr '|' ',' >> "$DB_CONN_LOG"

    # 3. Event consumer positions
    CONSUMER_POSITIONS=$(psql "$DB_URL" -t -c "
        SELECT
            consumer_id,
            last_event_id,
            updated_at
        FROM consumer_positions
        ORDER BY consumer_id;
    " 2>/dev/null | while read -r line; do
        if [ ! -z "$line" ]; then
            echo "$TIMESTAMP,$(echo $line | tr '|' ',')"
        fi
    done)

    if [ ! -z "$CONSUMER_POSITIONS" ]; then
        echo "$CONSUMER_POSITIONS" >> "$CONSUMER_POS_LOG"
    fi

    # 4. Thread count (macOS specific)
    THREAD_COUNT=$(ps -M -p $SERVER_PID 2>/dev/null | wc -l || echo "0")
    # Subtract 1 for header line
    THREAD_COUNT=$((THREAD_COUNT - 1))
    echo "$TIMESTAMP,$THREAD_COUNT" >> "$THREAD_COUNT_LOG"

    # 5. System stats (database size, event count, etc.)
    SYSTEM_STATS=$(psql "$DB_URL" -t -c "
        SELECT
            ROUND(pg_database_size('streamflow_profiling') / 1024.0 / 1024.0, 2) as db_size_mb,
            (SELECT COUNT(*) FROM workflow_events) as event_count,
            (SELECT COUNT(*) FROM workflows) as workflow_count,
            (SELECT COUNT(*) FROM activity_queue WHERE status = 'pending') as queue_size;
    " 2>/dev/null || echo "0|0|0|0")

    echo "$TIMESTAMP,$SYSTEM_STATS" | tr '|' ',' >> "$SYSTEM_STATS_LOG"

    # Progress indicator
    ELAPSED=$(($(date +%s) - (END_TIME - DURATION)))
    printf "\rMonitoring progress: %d/%d seconds" $ELAPSED $DURATION

    sleep $INTERVAL
done

echo ""
echo ""
echo "Monitoring complete!"
echo ""
echo "Results saved to:"
echo "  Memory usage:        $MEMORY_LOG"
echo "  DB connections:      $DB_CONN_LOG"
echo "  Consumer positions:  $CONSUMER_POS_LOG"
echo "  Thread count:        $THREAD_COUNT_LOG"
echo "  System stats:        $SYSTEM_STATS_LOG"
echo ""

# Generate summary report
echo "=== SUMMARY REPORT ===" > "$OUTPUT_DIR/monitoring_summary.txt"
echo "" >> "$OUTPUT_DIR/monitoring_summary.txt"

# Memory growth analysis
echo "Memory Usage:" >> "$OUTPUT_DIR/monitoring_summary.txt"
FIRST_RSS=$(head -2 "$MEMORY_LOG" | tail -1 | cut -d',' -f2)
LAST_RSS=$(tail -1 "$MEMORY_LOG" | cut -d',' -f2)
MAX_RSS=$(tail -n +2 "$MEMORY_LOG" | cut -d',' -f2 | sort -n | tail -1)
MEMORY_GROWTH=$(echo "scale=2; $LAST_RSS - $FIRST_RSS" | bc)
echo "  Initial RSS: ${FIRST_RSS} MB" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Final RSS: ${LAST_RSS} MB" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Peak RSS: ${MAX_RSS} MB" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Growth: ${MEMORY_GROWTH} MB" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "" >> "$OUTPUT_DIR/monitoring_summary.txt"

# Connection analysis
echo "Database Connections:" >> "$OUTPUT_DIR/monitoring_summary.txt"
FIRST_CONNS=$(head -2 "$DB_CONN_LOG" | tail -1 | cut -d',' -f2)
LAST_CONNS=$(tail -1 "$DB_CONN_LOG" | cut -d',' -f2)
MAX_CONNS=$(tail -n +2 "$DB_CONN_LOG" | cut -d',' -f2 | sort -n | tail -1)
MAX_ACTIVE=$(tail -n +2 "$DB_CONN_LOG" | cut -d',' -f3 | sort -n | tail -1)
echo "  Initial connections: $FIRST_CONNS" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Final connections: $LAST_CONNS" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Peak connections: $MAX_CONNS" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Peak active: $MAX_ACTIVE" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "" >> "$OUTPUT_DIR/monitoring_summary.txt"

# Consumer position analysis
echo "Event Consumer Positions (final state):" >> "$OUTPUT_DIR/monitoring_summary.txt"
tail -20 "$CONSUMER_POS_LOG" | awk -F',' '{print "  " $2 ": event_id=" $3}' | sort | uniq >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "" >> "$OUTPUT_DIR/monitoring_summary.txt"

# Thread count analysis
echo "Thread Count:" >> "$OUTPUT_DIR/monitoring_summary.txt"
FIRST_THREADS=$(head -2 "$THREAD_COUNT_LOG" | tail -1 | cut -d',' -f2)
LAST_THREADS=$(tail -1 "$THREAD_COUNT_LOG" | cut -d',' -f2)
MAX_THREADS=$(tail -n +2 "$THREAD_COUNT_LOG" | cut -d',' -f2 | sort -n | tail -1)
echo "  Initial threads: $FIRST_THREADS" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Final threads: $LAST_THREADS" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Peak threads: $MAX_THREADS" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "" >> "$OUTPUT_DIR/monitoring_summary.txt"

# System stats
echo "System Statistics (final state):" >> "$OUTPUT_DIR/monitoring_summary.txt"
FINAL_STATS=$(tail -1 "$SYSTEM_STATS_LOG")
DB_SIZE=$(echo $FINAL_STATS | cut -d',' -f2)
EVENT_COUNT=$(echo $FINAL_STATS | cut -d',' -f3)
WORKFLOW_COUNT=$(echo $FINAL_STATS | cut -d',' -f4)
QUEUE_SIZE=$(echo $FINAL_STATS | cut -d',' -f5)
echo "  Database size: ${DB_SIZE} MB" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Total events: $EVENT_COUNT" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Total workflows: $WORKFLOW_COUNT" >> "$OUTPUT_DIR/monitoring_summary.txt"
echo "  Pending activities: $QUEUE_SIZE" >> "$OUTPUT_DIR/monitoring_summary.txt"

cat "$OUTPUT_DIR/monitoring_summary.txt"
