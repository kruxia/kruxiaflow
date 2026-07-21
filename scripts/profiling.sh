#!/bin/bash
#
# Kruxia Flow Internal Profiling Runner
#
# Assumes:
#   - PostgreSQL database is running and migrations are complete
#   - Kruxia Flow server is already running
#   - OAuth client credentials are set in environment (KRUXIAFLOW_CLIENT_ID, KRUXIAFLOW_CLIENT_SECRET)
#
# Usage:
#   ./scripts/profiling.sh [OPTIONS]
#
# Options:
#   --help, -h              Show this help message
#   --port PORT             Kruxia Flow server port (default: 8080)
#   --test TEST_NAME        Run specific benchmark test
#   --nocapture             Don't capture test output
#   --verbose, -v           Verbose output
#   --output-dir DIR        Directory for all output (benchmark results, profiling data)
#                           Default: var/benchmark-TIMESTAMP
#   --compare BASELINE      Compare results with baseline file
#   --level LEVEL           Tracing level (debug, info, warn, error) for timing spans
#                           Default: info (recommended for performance analysis)
#   --duration SECONDS      Duration for sustained load test in seconds (default: 120)
#
# Examples:
#   ./scripts/profiling.sh                                       # Full benchmark suite
#   ./scripts/profiling.sh --test test_sequential_workflow_load  # Single test
#   ./scripts/profiling.sh --output-dir my-results               # Save to custom directory
#   ./scripts/profiling.sh --compare baseline.json               # Compare with baseline

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

PROJECT_DIR="$(dirname "$(dirname "$0")")"

# Default options
PORT=8080
TEST_NAME=""
NOCAPTURE="--nocapture"
VERBOSE=""
OUTPUT_DIR="${PROJECT_DIR}/var/profiling-$(date +%Y%m%d-%H%M%S)"
COMPARE_BASELINE=""
TRACE_LEVEL="trace"
BUILD_MODE="profiling"
SUSTAINED_DURATION=120

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# *//'
            exit 0
            ;;
        --build)
            BUILD_MODE="$2"
            shift 2
            ;;
        --port)
            PORT="$2"
            shift 2
            ;;
        --test)
            TEST_NAME="$2"
            shift 2
            ;;
        --nocapture)
            NOCAPTURE="--nocapture"
            shift
            ;;
        --verbose|-v)
            VERBOSE="--verbose"
            shift
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --compare)
            COMPARE_BASELINE="$2"
            shift 2
            ;;
        --level)
            TRACE_LEVEL="$2"
            shift 2
            ;;
        --duration)
            SUSTAINED_DURATION="$2"
            shift 2
            ;;
        *)
            echo -e "${RED}Error: Unknown option: $1${NC}"
            echo "Run '$0 --help' for usage information"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}Kruxia Flow Internal Profiling Runner${NC}"
echo "========================================"

# Create directory first, then resolve to absolute path
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(realpath "$OUTPUT_DIR")

echo -e "${BLUE}Output directory: $OUTPUT_DIR${NC}"

# Configure tracing level for performance analysis
echo -e "${BLUE}Configuring tracing level: $TRACE_LEVEL${NC}"
export RUST_LOG="info,kruxiaflow=$TRACE_LEVEL,sqlx=info"
export RUST_BACKTRACE=1
export KRUXIAFLOW_LOG_LEVEL="$TRACE_LEVEL"
echo -e "${BLUE}Tracing configured (RUST_LOG=$RUST_LOG)${NC}"

# Check required environment variables
if [ -z "$KRUXIAFLOW_CLIENT_ID" ]; then
    echo -e "${RED}Error: KRUXIAFLOW_CLIENT_ID environment variable not set${NC}"
    echo "Please set OAuth client credentials in your environment"
    exit 1
fi

if [ -z "$KRUXIAFLOW_CLIENT_SECRET" ]; then
    echo -e "${RED}Error: KRUXIAFLOW_CLIENT_SECRET environment variable not set${NC}"
    echo "Please set OAuth client credentials in your environment"
    exit 1
fi

if [ -z "$DATABASE_URL" ]; then
    echo -e "${RED}Error: DATABASE_URL environment variable not set${NC}"
    echo "Please set DATABASE_URL in your environment"
    exit 1
elif [[ ! "$DATABASE_URL" =~ "kruxiaflow_profiling" ]]; then
    DATABASE_URL="${DATABASE_URL%/*}/kruxiaflow_profiling"
fi

# Extract database name from DATABASE_URL
DB_NAME="${DATABASE_URL##*/}"

# Verify server is accessible
echo -e "${YELLOW}Verifying server is running...${NC}"
if ! curl -f http://localhost:${PORT}/health > /dev/null 2>&1; then
    echo -e "${RED}Error: Kruxia Flow server not accessible at http://localhost:${PORT}${NC}"
    echo "Please start the server before running benchmarks"
    exit 1
fi
echo -e "${GREEN}Server is accessible${NC}"

# Initialize database and monitoring for benchmark suite
echo ""
echo -e "${YELLOW}Initializing for benchmark suite...${NC}"

# Reset pg_stat_statements for clean measurement
echo -e "${BLUE}Resetting pg_stat_statements...${NC}"
docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c "SELECT pg_stat_statements_reset();" 2>/dev/null || true

# Truncate tables for clean state before first test
# Note: Each test will truncate before running, and database state is preserved after each test
echo -e "${BLUE}Truncating workflow tables for first test...${NC}"
docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c "
    TRUNCATE TABLE workflow_events CASCADE;
    TRUNCATE TABLE activity_queue CASCADE;
    TRUNCATE TABLE workflows CASCADE;
    TRUNCATE TABLE workflow_event_consumers CASCADE;
" > /dev/null 2>&1

echo -e "${GREEN}Initialization complete${NC}"

# Clear memory tracking file for clean measurement
MEMORY_TRACKING_FILE="${PROJECT_DIR}/var/memory/memory_usage.csv"
if [ -f "$MEMORY_TRACKING_FILE" ]; then
    echo -e "${BLUE}Clearing memory tracking file...${NC}"
    echo "timestamp,rss_mb,vsz_mb,cpu_percent" > "$MEMORY_TRACKING_FILE"
    echo -e "${GREEN}Memory tracking reset${NC}"
else
    echo -e "${YELLOW}Note: Memory tracking file not found at $MEMORY_TRACKING_FILE${NC}"
    echo "Server may not be running in profiling mode"
fi

# Set base URL and sustained load duration for benchmarks
export KRUXIAFLOW_BASE_URL="http://localhost:${PORT}"
export SUSTAINED_LOAD_DURATION_SECS="${SUSTAINED_DURATION}"

# Register workflow definitions
echo ""
echo -e "${YELLOW}Registering workflow definitions...${NC}"
if cargo run --package kruxiaflow-profiling --bin register-workflows 2>&1; then
    echo -e "${GREEN}Workflow definitions registered successfully${NC}"
else
    echo -e "${RED}Error: Failed to register workflow definitions${NC}"
    echo "Make sure the server is running and credentials are valid"
    exit 1
fi

# Run benchmarks
echo ""
echo -e "${YELLOW}Running benchmarks...${NC}"
echo "========================================"

# Export output directory for Rust tests to save results directly
export PROFILING_OUTPUT_DIR="$OUTPUT_DIR"
echo -e "${BLUE}PROFILING_OUTPUT_DIR exported: $PROFILING_OUTPUT_DIR${NC}"

# Define tests to run (all tests by default, or specific test if provided)
if [ -n "$TEST_NAME" ]; then
    TESTS_TO_RUN=("$TEST_NAME")
else
    TESTS_TO_RUN=(
        "test_parallel_workflow_load"
        "test_sequential_workflow_load"
        "test_high_concurrency_load"
        "test_sustained_throughput"
    )
fi

# Build base command
BASE_CMD="cargo test --package kruxiaflow-profiling --test load_tests --release"

if [ -n "$VERBOSE" ]; then
    BASE_CMD="$BASE_CMD $VERBOSE"
fi

# Add test arguments: 1 thread, ignored tests (for explicit running)
TEST_ARGS="--test-threads=1 --ignored"
if [ -n "$NOCAPTURE" ]; then
    TEST_ARGS="$TEST_ARGS $NOCAPTURE"
fi

# Run each test individually to isolate resource usage
PROFILING_SUCCESS=true
echo "" > ${OUTPUT_DIR}/profiling-output.txt  # Initialize output file

for test_name in "${TESTS_TO_RUN[@]}"; do
    echo ""
    echo -e "${YELLOW}========================================${NC}"
    echo -e "${YELLOW}Running: $test_name${NC}"
    echo -e "${YELLOW}========================================${NC}"

    # Clean database BEFORE each test to ensure clean state
    echo -e "${BLUE}Truncating database before test...${NC}"
    docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c "
        TRUNCATE TABLE workflow_events CASCADE;
        TRUNCATE TABLE activity_queue CASCADE;
        TRUNCATE TABLE workflows CASCADE;
        TRUNCATE TABLE workflow_event_consumers CASCADE;
    " > /dev/null 2>&1

    if [ $? -eq 0 ]; then
        echo -e "${GREEN}Database truncated successfully${NC}"
    else
        echo -e "${YELLOW}Warning: Database truncation may have failed${NC}"
    fi

    echo ""
    CMD="$BASE_CMD $test_name -- $TEST_ARGS"
    echo "Command: $CMD"
    echo ""

    # Run test and capture output
    if eval "$CMD" 2>&1 | tee -a ${OUTPUT_DIR}/profiling-output.txt; then
        echo -e "${GREEN}✓ $test_name completed${NC}"
    else
        echo -e "${RED}✗ $test_name failed${NC}"
        PROFILING_SUCCESS=false
    fi

    # Database is NOT cleaned after test - inspect state in database
    echo -e "${BLUE}Test complete. Database state preserved for inspection.${NC}"

    # Brief delay before next test
    sleep 1
done

echo ""
echo -e "${YELLOW}========================================${NC}"
echo -e "${YELLOW}All tests completed${NC}"
echo -e "${YELLOW}========================================${NC}"

# Capture Kruxia Flow server logs
if [ "$BUILD_MODE" == "profiling" ]; then
    CONTAINER_NAME="kruxiaflow-profiling"
else
    CONTAINER_NAME="kruxiaflow"
fi
echo ""
echo -e "${YELLOW}Capturing server logs...${NC}"
if docker ps --format '{{.Names}}' | grep $CONTAINER_NAME; then
    docker compose logs --timestamps --no-color $CONTAINER_NAME > "$OUTPUT_DIR/server-logs.txt" 2>&1
    echo -e "${GREEN}Server logs saved to: $OUTPUT_DIR/server-logs.txt${NC}"

    # Extract relevant logs based on trace level
    if [[ "$TRACE_LEVEL" == "trace" ]] || [[ "$TRACE_LEVEL" == "debug" ]]; then
        # Extract detailed trace-level timing logs (only available with verbose_tracing=true)
        grep -E "Transaction started|Advisory lock acquired|loaded in|evaluated in|scheduled and events|saved in|committed in|Total event processing" \
            "$OUTPUT_DIR/server-logs.txt" > "$OUTPUT_DIR/trace-timings.txt" 2>/dev/null || true

        if [ -s "$OUTPUT_DIR/trace-timings.txt" ]; then
            echo -e "${GREEN}Trace timing logs extracted to: $OUTPUT_DIR/trace-timings.txt${NC}"
        else
            echo -e "${YELLOW}Note: No trace-level logs found. Verify verbose_tracing is enabled in server logs.${NC}"
        fi
    else
        # Extract INFO-level orchestrator activity logs
        grep -E "Scheduling.*activities|Processing event|Found.*ready activities|Activity state distribution" \
            "$OUTPUT_DIR/server-logs.txt" > "$OUTPUT_DIR/orchestrator-activity.txt" 2>/dev/null || true

        if [ -s "$OUTPUT_DIR/orchestrator-activity.txt" ]; then
            echo -e "${GREEN}Orchestrator activity logs extracted to: $OUTPUT_DIR/orchestrator-activity.txt${NC}"
        fi
    fi
else
    echo -e "${YELLOW}Note: $CONTAINER_NAME container not found${NC}"
fi

# Save PostgreSQL query stats
echo ""
echo -e "${YELLOW}Collecting PostgreSQL query statistics...${NC}"

# Ensure pg_stat_statements extension exists
docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c "
    CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
" > /dev/null 2>&1

# Check if extension was created successfully
if docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c "
    SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements';
" 2>/dev/null | grep -q "1"; then
    # Get slow queries using psql inside the Docker container
    # Note: Using 0.01ms threshold to capture queries (most queries are sub-millisecond now)
    docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c "
        SELECT
            query,
            calls,
            ROUND(mean_exec_time::numeric, 3) as mean_ms,
            ROUND(stddev_exec_time::numeric, 3) as stddev_ms,
            ROUND(total_exec_time::numeric, 2) as total_ms
        FROM pg_stat_statements
        WHERE mean_exec_time > 0.01
        ORDER BY mean_exec_time DESC
        LIMIT 20;
    " > "$OUTPUT_DIR/queries.txt" 2>&1

    # Check if query stats were collected successfully
    if grep -q "mean_ms" "$OUTPUT_DIR/queries.txt" 2>/dev/null; then
        echo -e "${GREEN}Query statistics saved to: $OUTPUT_DIR/queries.txt${NC}"
    else
        echo -e "${YELLOW}Note: No slow queries found (all queries < 0.01ms)${NC}"
    fi
else
    echo -e "${YELLOW}Note: pg_stat_statements extension not available${NC}"
    echo "To enable, add 'shared_preload_libraries = pg_stat_statements' to postgresql.conf"
    echo "No query statistics available" > "$OUTPUT_DIR/queries.txt"
fi

# Copy and analyze memory tracking data
echo ""
echo -e "${YELLOW}Analyzing memory usage...${NC}"

if [ -f "$MEMORY_TRACKING_FILE" ]; then
    # Copy memory CSV to output directory
    cp "$MEMORY_TRACKING_FILE" "$OUTPUT_DIR/memory_usage.csv"

    # Analyze memory usage using awk
    MEMORY_ANALYSIS="$OUTPUT_DIR/memory_analysis.txt"

    # Skip header and analyze data
    if [ $(wc -l < "$MEMORY_TRACKING_FILE") -gt 1 ]; then
        awk -F',' '
        BEGIN {
            min_rss = 999999999
            max_rss = 0
            sum_rss = 0
            count = 0
            min_vsz = 999999999
            max_vsz = 0
            sum_vsz = 0
            first_ts = 0
            last_ts = 0
        }
        NR > 1 {
            # Skip header
            rss = $2
            vsz = $3
            ts = $1

            if (rss != "" && rss != "rss_mb") {
                if (first_ts == 0) first_ts = ts
                last_ts = ts

                if (rss < min_rss) min_rss = rss
                if (rss > max_rss) max_rss = rss
                sum_rss += rss

                if (vsz < min_vsz) min_vsz = vsz
                if (vsz > max_vsz) max_vsz = vsz
                sum_vsz += vsz

                count++
            }
        }
        END {
            if (count > 0) {
                avg_rss = sum_rss / count
                avg_vsz = sum_vsz / count
                duration = last_ts - first_ts
                growth_rate = (max_rss - min_rss) / duration

                print "Memory Usage Analysis"
                print "===================="
                print ""
                print "RSS (Resident Set Size):"
                printf "  Min:     %10.2f MB\n", min_rss
                printf "  Max:     %10.2f MB\n", max_rss
                printf "  Average: %10.2f MB\n", avg_rss
                printf "  Growth:  %10.2f MB\n", max_rss - min_rss
                print ""
                print "VSZ (Virtual Size):"
                printf "  Min:     %10.2f MB\n", min_vsz
                printf "  Max:     %10.2f MB\n", max_vsz
                printf "  Average: %10.2f MB\n", avg_vsz
                print ""
                print "Duration:"
                printf "  %d seconds (%d samples)\n", duration, count
                print ""

                # Memory leak detection (simple heuristic)
                if (growth_rate > 0.1) {
                    print "⚠️  WARNING: Potential memory leak detected"
                    printf "   Growth rate: %.3f MB/second\n", growth_rate
                } else if (growth_rate > 0.01) {
                    print "⚠️  CAUTION: Memory growth observed"
                    printf "   Growth rate: %.3f MB/second\n", growth_rate
                } else {
                    print "✓ Memory usage appears stable"
                    printf "   Growth rate: %.3f MB/second\n", growth_rate
                }
            } else {
                print "No memory data collected"
            }
        }
        ' "$MEMORY_TRACKING_FILE" > "$MEMORY_ANALYSIS"

        # Display analysis
        cat "$MEMORY_ANALYSIS"
        echo ""
        echo -e "${GREEN}Memory analysis saved to: $OUTPUT_DIR/memory_analysis.txt${NC}"
        echo -e "${GREEN}Memory CSV saved to: $OUTPUT_DIR/memory_usage.csv${NC}"
    else
        echo -e "${YELLOW}Note: No memory data collected (file is empty)${NC}"
    fi
else
    echo -e "${YELLOW}Note: Memory tracking file not found${NC}"
    echo "Server may not be running in profiling mode"
fi

# Check if benchmark results were saved by Rust tests
if [ -f "$OUTPUT_DIR/results.json" ]; then
    echo ""
    echo -e "${GREEN}Benchmark results saved to: $OUTPUT_DIR/results.json${NC}"
else
    echo ""
    echo -e "${YELLOW}Note: No results.json generated (tests may have failed or been skipped)${NC}"
fi

# Run kruxiaflow profile command for comprehensive database profiling
echo ""
echo -e "${YELLOW}Running database performance profiling...${NC}"

# Build the kruxiaflow binary if needed (use release for accuracy)
if cargo build --package kruxiaflow --release 2>/dev/null; then
    # Resolve the cargo target dir (honors CARGO_TARGET_DIR / build.target-dir)
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$(cd "$PROJECT_DIR" && cargo metadata --format-version=1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}"
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${PROJECT_DIR}/target}"
    KRUXIAFLOW_BIN="${CARGO_TARGET_DIR}/release/kruxiaflow"

    # Generate JSON profile with EXPLAIN ANALYZE
    # Note: stderr goes to log file, only stdout (JSON) goes to output file
    echo -e "${BLUE}Generating JSON profile with execution plans...${NC}"
    if $KRUXIAFLOW_BIN profile --explain --format json > "$OUTPUT_DIR/db_profile.json" 2>"$OUTPUT_DIR/db_profile_errors.log"; then
        # Validate JSON output (remove any non-JSON lines that might have leaked)
        if python3 -c "import json; json.load(open('$OUTPUT_DIR/db_profile.json'))" 2>/dev/null; then
            echo -e "${GREEN}Database profile (JSON) saved to: $OUTPUT_DIR/db_profile.json${NC}"
            rm -f "$OUTPUT_DIR/db_profile_errors.log"
        else
            echo -e "${YELLOW}Note: JSON output contains invalid data, attempting to fix...${NC}"
            # Extract just the JSON object (skip any log lines before the opening brace)
            sed -n '/^{/,$p' "$OUTPUT_DIR/db_profile.json" > "$OUTPUT_DIR/db_profile_fixed.json"
            if python3 -c "import json; json.load(open('$OUTPUT_DIR/db_profile_fixed.json'))" 2>/dev/null; then
                mv "$OUTPUT_DIR/db_profile_fixed.json" "$OUTPUT_DIR/db_profile.json"
                echo -e "${GREEN}Database profile (JSON) fixed and saved to: $OUTPUT_DIR/db_profile.json${NC}"
            else
                echo -e "${YELLOW}Note: Could not produce valid JSON output${NC}"
                rm -f "$OUTPUT_DIR/db_profile_fixed.json"
            fi
        fi
    else
        echo -e "${YELLOW}Note: Database profiling failed (views may not exist yet)${NC}"
        echo "Run 'sqlx migrate run' to create profiling views"
        [ -f "$OUTPUT_DIR/db_profile_errors.log" ] && cat "$OUTPUT_DIR/db_profile_errors.log"
    fi

    # Generate text profile with verbose output and EXPLAIN ANALYZE
    echo -e "${BLUE}Generating text profile with execution plans...${NC}"
    if $KRUXIAFLOW_BIN profile --explain -v > "$OUTPUT_DIR/db_profile.txt" 2>&1; then
        echo -e "${GREEN}Database profile (text) saved to: $OUTPUT_DIR/db_profile.txt${NC}"

        # Display summary of slow queries
        echo ""
        echo -e "${YELLOW}Slow Query Summary:${NC}"
        head -40 "$OUTPUT_DIR/db_profile.txt" | grep -A 20 "Slow Queries" || true
    else
        echo -e "${YELLOW}Note: Database profiling failed${NC}"
    fi
else
    echo -e "${YELLOW}Note: Could not build kruxiaflow binary for profiling${NC}"
fi

# Compare with baseline if requested
if [ -n "$COMPARE_BASELINE" ]; then
    echo ""
    echo -e "${YELLOW}Comparing with baseline...${NC}"

    if [ ! -f "$COMPARE_BASELINE" ]; then
        echo -e "${RED}Error: Baseline file not found: $COMPARE_BASELINE${NC}"
    else
        CURRENT_RESULTS="$OUTPUT_DIR/results.json"
        if [ ! -f "$CURRENT_RESULTS" ]; then
            echo -e "${RED}Error: Current results not found at: $CURRENT_RESULTS${NC}"
        else
            python3 scripts/compare_benchmarks.py "$COMPARE_BASELINE" "$CURRENT_RESULTS" > "$OUTPUT_DIR/comparison.json"

            echo -e "${GREEN}Comparison complete${NC}"
            echo ""
            cat "$OUTPUT_DIR/comparison.json" | python3 -m json.tool

            # Check for regression
            if grep -q '"regression": true' "$OUTPUT_DIR/comparison.json"; then
                echo ""
                echo -e "${RED}⚠️  Performance regression detected!${NC}"
                python3 scripts/check_regression.py "$OUTPUT_DIR/comparison.json" || true
            else
                echo ""
                echo -e "${GREEN}✅ No performance regression detected${NC}"
            fi
        fi
    fi
fi

# Summary
echo ""
echo "========================================"
if [ "$PROFILING_SUCCESS" = true ]; then
    echo -e "${GREEN}✅ Profiling completed successfully!${NC}"
    echo ""
    echo -e "${BLUE}Output Directory: $OUTPUT_DIR${NC}"
    echo ""
    echo "Profiling Results:"
    echo "  results.json - Performance metrics (throughput, latency, etc.)"
    echo "  queries.txt - PostgreSQL query statistics (pg_stat_statements)"
    echo "  profiling-output.txt - Full test output"

    if [ -f "$OUTPUT_DIR/db_profile.json" ]; then
        echo ""
        echo "Database Profiling:"
        echo "  db_profile.json - Comprehensive DB profile (JSON)"
        echo "  db_profile.txt - Comprehensive DB profile with EXPLAIN plans"
    fi

    if [ -f "$OUTPUT_DIR/memory_usage.csv" ]; then
        echo ""
        echo "Memory Profiling:"
        echo "  memory_usage.csv - Memory usage over time (RSS, VSZ, CPU)"
        echo "  memory_analysis.txt - Memory analysis summary"
    fi

    if [ -n "$COMPARE_BASELINE" ] && [ -f "$OUTPUT_DIR/comparison.json" ]; then
        echo ""
        echo "Comparison:"
        echo "  comparison.json - Performance comparison vs baseline"
    fi

    echo ""
    echo "Database State:"
    echo "  The database has NOT been cleaned after the last test."
    echo "  To inspect accumulated data:"
    echo "    docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c 'SELECT COUNT(*) FROM workflows;'"
    echo "    docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c 'SELECT COUNT(*) FROM workflow_events;'"
    echo "    docker exec kruxiaflow-postgres psql -U kruxiaflow -d ${DB_NAME} -c 'SELECT COUNT(*) FROM activity_queue;'"

    exit 0
else
    echo -e "${RED}❌ Profiling failed${NC}"
    echo ""
    echo "Tips for debugging:"
    echo "  - Check profiling output: cat ${OUTPUT_DIR}/profiling-output.txt"
    echo "  - Verify OAuth credentials are set: echo \$KRUXIAFLOW_CLIENT_ID"
    echo "  - Verify server is running: curl http://localhost:${PORT}/health"
    echo "  - Check workflow status via API"

    echo ""
    echo -e "${BLUE}Output Directory: $OUTPUT_DIR${NC}"

    exit 1
fi
