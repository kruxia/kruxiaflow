#!/bin/bash
#
# StreamFlow Benchmark Runner
#
# Usage:
#   ./scripts/benchmark.sh [OPTIONS]
#
# Options:
#   --help, -h              Show this help message
#   --skip-db-setup         Skip database setup (assumes DB is ready)
#   --skip-server-start     Skip starting StreamFlow server (assumes server is running)
#   --keep-server           Keep server running after benchmarks complete
#   --port PORT             StreamFlow server port (default: 8080)
#   --test TEST_NAME        Run specific benchmark test
#   --release               Run in release mode (default, recommended)
#   --debug                 Run in debug mode (faster compile, slower execution)
#   --nocapture             Don't capture test output
#   --verbose, -v           Verbose output
#   --output-dir DIR        Directory for all output (benchmark results, profiling data)
#                           Default: var/benchmark-TIMESTAMP
#   --compare BASELINE      Compare results with baseline file
#   --profile               Enable CPU profiling with flamegraph
#   --max-activities-per-poll N  Max activities per worker poll (default: 10)
#
# Examples:
#   ./scripts/benchmark.sh                                    # Full benchmark suite
#   ./scripts/benchmark.sh --skip-server-start                # Use existing server
#   ./scripts/benchmark.sh --test test_sequential_workflow_load  # Single test
#   ./scripts/benchmark.sh --output-dir my-results            # Save to custom directory
#   ./scripts/benchmark.sh --compare baseline.json            # Compare with baseline
#   ./scripts/benchmark.sh --profile --test test_sequential_workflow_load  # Profile single test

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default options
SKIP_DB_SETUP=false
SKIP_SERVER_START=false
KEEP_SERVER=false
PORT=8080
TEST_NAME=""
BUILD_MODE="release"
NOCAPTURE="--nocapture"
VERBOSE=""
OUTPUT_DIR=""
COMPARE_BASELINE=""
ENABLE_PROFILING=false
MAX_ACTIVITIES_PER_POLL="${STREAMFLOW_MAX_ACTIVITIES_PER_POLL:-10}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# *//'
            exit 0
            ;;
        --skip-db-setup)
            SKIP_DB_SETUP=true
            shift
            ;;
        --skip-server-start)
            SKIP_SERVER_START=true
            shift
            ;;
        --keep-server)
            KEEP_SERVER=true
            shift
            ;;
        --port)
            PORT="$2"
            shift 2
            ;;
        --test)
            TEST_NAME="$2"
            shift 2
            ;;
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --debug)
            BUILD_MODE="debug"
            shift
            ;;
        --nocapture)
            NOCAPTURE="--nocapture"
            shift
            ;;
        --verbose|-v)
            VERBOSE="--verbose"
            shift
            ;;
        --output-dir|--profile-output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --compare)
            COMPARE_BASELINE="$2"
            shift 2
            ;;
        --profile)
            ENABLE_PROFILING=true
            shift
            ;;
        --max-activities-per-poll)
            MAX_ACTIVITIES_PER_POLL="$2"
            shift 2
            ;;
        *)
            echo -e "${RED}Error: Unknown option: $1${NC}"
            echo "Run '$0 --help' for usage information"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}StreamFlow Benchmark Runner${NC}"
echo "========================================"

# Set output directory
if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="var/benchmark-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUTPUT_DIR"

echo -e "${BLUE}Output directory: $OUTPUT_DIR${NC}"

# Profiling setup
PROFILE_PID=""
if [ "$ENABLE_PROFILING" = true ]; then
    echo -e "${BLUE}Profiling enabled${NC}"

    # Require specific test when profiling
    if [ -z "$TEST_NAME" ]; then
        echo -e "${RED}Error: --profile requires --test TEST_NAME${NC}"
        echo "Profiling the entire test suite produces unclear results"
        exit 1
    fi

    # Check for inferno (optional, for flamegraph generation)
    if ! command -v inferno-collapse-sample &> /dev/null || ! command -v inferno-flamegraph &> /dev/null; then
        echo -e "${YELLOW}Note: inferno not found. Flamegraph generation will be skipped.${NC}"
        echo "Install with: cargo install inferno"
        echo "Raw profiling data will still be collected"
    fi

    # Enable debug logging for profiling
    export RUST_LOG=streamflow=debug,sqlx=warn
    export RUST_BACKTRACE=1
    echo -e "${BLUE}Debug logging enabled (RUST_LOG=$RUST_LOG)${NC}"
fi

# Server PID tracking
SERVER_PID=""

# Cleanup function
cleanup() {
    # Stop profiling if running
    if [ -n "$PROFILE_PID" ]; then
        echo ""
        echo -e "${YELLOW}Stopping profiler (PID: $PROFILE_PID)...${NC}"
        kill -SIGINT $PROFILE_PID 2>/dev/null || true
        wait $PROFILE_PID 2>/dev/null || true
    fi

    # Stop server if running
    if [ -n "$SERVER_PID" ] && [ "$KEEP_SERVER" = false ]; then
        echo ""
        echo -e "${YELLOW}Stopping StreamFlow server (PID: $SERVER_PID)...${NC}"
        kill $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true
        echo -e "${GREEN}Server stopped${NC}"
    fi
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Check required environment variables
if [ -z "$STREAMFLOW_CLIENT_ID" ]; then
    echo -e "${RED}Error: STREAMFLOW_CLIENT_ID environment variable not set${NC}"
    echo "Please set OAuth client credentials in your environment"
    exit 1
fi

if [ -z "$STREAMFLOW_CLIENT_SECRET" ]; then
    echo -e "${RED}Error: STREAMFLOW_CLIENT_SECRET environment variable not set${NC}"
    echo "Please set OAuth client credentials in your environment"
    exit 1
fi

# Database setup (unless skipped)
if [ "$SKIP_DB_SETUP" = false ]; then
    # Check if PostgreSQL container is running
    if ! docker ps | grep -q streamflow-postgres; then
        echo -e "${YELLOW}Starting PostgreSQL container...${NC}"
        docker-compose up -d postgres
        echo "Waiting for PostgreSQL to be ready..."
        sleep 5
    fi

    # Wait for PostgreSQL to be ready
    until docker exec streamflow-postgres pg_isready -U streamflow > /dev/null 2>&1; do
        echo "Waiting for PostgreSQL..."
        sleep 1
    done

    echo -e "${GREEN}PostgreSQL is ready${NC}"

    # Database configuration
    DB_USER="streamflow"
    DB_PASSWORD="streamflow_dev"
    DB_HOST="127.0.0.1"
    DB_PORT="5432"
    DB_NAME="streamflow_benchmark"

    # Drop and recreate benchmark database
    echo -e "${YELLOW}Setting up benchmark database...${NC}"

    # Terminate any existing connections
    docker exec streamflow-postgres psql -U ${DB_USER} -c "
    SELECT pg_terminate_backend(pid)
    FROM pg_stat_activity
    WHERE datname = '${DB_NAME}' AND pid <> pg_backend_pid();" 2>/dev/null || true

    # Drop and recreate database
    docker exec streamflow-postgres psql -U ${DB_USER} -c "DROP DATABASE IF EXISTS ${DB_NAME};" 2>/dev/null || true
    docker exec streamflow-postgres psql -U ${DB_USER} -c "CREATE DATABASE ${DB_NAME};"

    echo -e "${GREEN}Benchmark database created${NC}"

    # Set DATABASE_URL for benchmark database
    export DATABASE_URL="postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}"

    # Run migrations
    echo -e "${YELLOW}Running migrations...${NC}"
    sqlx migrate run

    echo -e "${GREEN}Migrations complete${NC}"

    # Seed OAuth client
    echo -e "${YELLOW}Seeding OAuth client...${NC}"
    cargo run --package streamflow-benchmark --bin seed-oauth-client

    echo -e "${GREEN}OAuth client seeded${NC}"

    # Enable and reset pg_stat_statements if profiling enabled
    if [ "$ENABLE_PROFILING" = true ]; then
        echo -e "${YELLOW}Enabling pg_stat_statements extension...${NC}"

        # Create extension if it doesn't exist (requires shared_preload_libraries already set in docker-compose)
        docker exec streamflow-postgres psql -U ${DB_USER} -d ${DB_NAME} -c \
            "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;" 2>&1 | grep -v "already exists" || true

        # Reset statistics for clean measurement
        docker exec streamflow-postgres psql -U ${DB_USER} -d ${DB_NAME} -c \
            "SELECT pg_stat_statements_reset();" 2>&1 | grep -v "does not exist" || true

        echo -e "${GREEN}pg_stat_statements enabled and reset${NC}"
    fi
else
    echo -e "${BLUE}Skipping database setup (--skip-db-setup)${NC}"
    # DATABASE_URL should already be set in environment
    if [ -z "$DATABASE_URL" ]; then
        echo -e "${YELLOW}Warning: DATABASE_URL not set${NC}"
    fi

    # Reset pg_stat_statements if profiling enabled
    if [ "$ENABLE_PROFILING" = true ]; then
        echo -e "${YELLOW}Resetting pg_stat_statements for clean measurement...${NC}"
        # Extract database name from DATABASE_URL if needed
        DB_NAME="${DATABASE_URL##*/}"
        docker exec streamflow-postgres psql -U streamflow -d ${DB_NAME} -c "SELECT pg_stat_statements_reset();" 2>/dev/null || true
    fi
fi

# Set base URL for benchmarks
export STREAMFLOW_BASE_URL="http://localhost:${PORT}"

# Start StreamFlow server (unless skipped)
if [ "$SKIP_SERVER_START" = false ]; then
    echo ""
    echo -e "${YELLOW}Building StreamFlow server...${NC}"

    if [ "$BUILD_MODE" = "release" ]; then
        cargo build --release --bin streamflow
        BINARY="./target/release/streamflow"
    else
        cargo build --bin streamflow
        BINARY="./target/debug/streamflow"
    fi

    echo -e "${GREEN}Build complete${NC}"

    echo -e "${YELLOW}Starting StreamFlow server on port ${PORT}...${NC}"

    # Start server in background with 20 workers for high concurrency
    $BINARY serve --port $PORT --workers 20 --max-activities-per-poll $MAX_ACTIVITIES_PER_POLL > /tmp/streamflow-benchmark.log 2>&1 &
    SERVER_PID=$!

    echo -e "${BLUE}Server PID: $SERVER_PID${NC}"
    echo "Server logs: /tmp/streamflow-benchmark.log"

    # Wait for server to be ready
    echo "Waiting for server to be ready..."
    MAX_WAIT=30
    WAITED=0
    until curl -f http://localhost:${PORT}/health > /dev/null 2>&1; do
        if [ $WAITED -ge $MAX_WAIT ]; then
            echo -e "${RED}Error: Server failed to start within ${MAX_WAIT} seconds${NC}"
            echo "Check logs at: /tmp/streamflow-benchmark.log"
            tail -50 /tmp/streamflow-benchmark.log
            exit 1
        fi
        sleep 1
        WAITED=$((WAITED + 1))
        echo -n "."
    done
    echo ""

    echo -e "${GREEN}StreamFlow server is ready${NC}"

    # Register benchmark workflow definitions
    echo -e "${YELLOW}Registering benchmark workflows...${NC}"
    cargo run --package streamflow-benchmark --bin register-workflows

    echo -e "${GREEN}Workflows registered${NC}"
else
    echo -e "${BLUE}Skipping server start (--skip-server-start)${NC}"

    # Verify server is accessible
    if ! curl -f http://localhost:${PORT}/health > /dev/null 2>&1; then
        echo -e "${RED}Error: Server not accessible at http://localhost:${PORT}${NC}"
        echo "Make sure StreamFlow is running before using --skip-server-start"
        exit 1
    fi
    echo -e "${GREEN}Server is accessible${NC}"
fi

# Run benchmarks
echo ""
echo -e "${YELLOW}Running benchmarks...${NC}"
echo "========================================"

# Export output directory for Rust tests to save results directly
export BENCHMARK_OUTPUT_DIR="$OUTPUT_DIR"

# Build test command
CMD="cargo test --package streamflow-benchmark --test load_tests"

if [ "$BUILD_MODE" = "release" ]; then
    CMD="$CMD --release"
fi

if [ -n "$TEST_NAME" ]; then
    CMD="$CMD $TEST_NAME"
fi

if [ -n "$VERBOSE" ]; then
    CMD="$CMD $VERBOSE"
fi

# Add test arguments
TEST_ARGS="--test-threads=1"
if [ -n "$NOCAPTURE" ]; then
    TEST_ARGS="$TEST_ARGS $NOCAPTURE"
fi
CMD="$CMD -- $TEST_ARGS"

# Capture output for reference
CMD="$CMD | tee /tmp/benchmark-output.txt"

# Start profiling if enabled
if [ "$ENABLE_PROFILING" = true ]; then
    echo -e "${YELLOW}Starting CPU profiler...${NC}"

    # Wait a moment for server to settle
    sleep 2

    # Detect OS and use appropriate profiling tool
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS: Use dtrace or sample command
        echo "Using macOS 'sample' profiler"

        # Run sample in background (samples for 60 seconds by default)
        sample $SERVER_PID 60 -file "$OUTPUT_DIR/sample.txt" > "$OUTPUT_DIR/profiler.log" 2>&1 &
        PROFILE_PID=$!

        echo -e "${BLUE}Profiler started (PID: $PROFILE_PID)${NC}"
        echo "Sample data will be saved to: $OUTPUT_DIR/sample.txt"
        echo -e "${YELLOW}Note: Will convert to flamegraph after sampling completes${NC}"

    elif [[ "$OSTYPE" == "linux"* ]]; then
        # Linux: Use perf record
        echo "Using Linux 'perf record' profiler"

        # Check if perf is available
        if ! command -v perf &> /dev/null; then
            echo -e "${RED}Error: perf not found. Install with: sudo apt-get install linux-tools-common${NC}"
            ENABLE_PROFILING=false
        else
            # Run perf record in background
            sudo perf record -F 99 -p $SERVER_PID -g -o "$OUTPUT_DIR/perf.data" \
                > "$OUTPUT_DIR/profiler.log" 2>&1 &
            PROFILE_PID=$!

            echo -e "${BLUE}Profiler started (PID: $PROFILE_PID)${NC}"
            echo "Perf data will be saved to: $OUTPUT_DIR/perf.data"
        fi
    else
        echo -e "${YELLOW}Warning: Unsupported OS for profiling: $OSTYPE${NC}"
        echo "Skipping CPU profiling, but will still collect query stats"
        ENABLE_PROFILING=false
    fi

    # Give profiler time to start
    if [ "$ENABLE_PROFILING" = true ]; then
        sleep 2
    fi
fi

# Execute benchmarks
echo "Running: $CMD"
echo ""

if eval "$CMD"; then
    BENCHMARK_SUCCESS=true
else
    BENCHMARK_SUCCESS=false
fi

# Stop profiling if enabled
if [ "$ENABLE_PROFILING" = true ] && [ -n "$PROFILE_PID" ]; then
    echo ""
    echo -e "${YELLOW}Stopping profiler...${NC}"

    # Send SIGINT to profiler to stop it gracefully
    kill -SIGINT $PROFILE_PID 2>/dev/null || true

    # Wait for profiler to finish processing
    wait $PROFILE_PID 2>/dev/null || true
    PROFILE_PID=""

    echo -e "${GREEN}Profiling data collected${NC}"

    # Generate flamegraph from collected data
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS: Convert sample output to flamegraph
        if [ -f "$OUTPUT_DIR/sample.txt" ]; then
            echo -e "${YELLOW}Converting sample data to flamegraph...${NC}"

            # Check if inferno is installed for flamegraph generation
            if command -v inferno-collapse-sample &> /dev/null && command -v inferno-flamegraph &> /dev/null; then
                inferno-collapse-sample "$OUTPUT_DIR/sample.txt" | \
                    inferno-flamegraph > "$OUTPUT_DIR/flamegraph.svg" 2>/dev/null && \
                    echo -e "${GREEN}Flamegraph generated: $OUTPUT_DIR/flamegraph.svg${NC}" || \
                    echo -e "${YELLOW}Could not generate flamegraph. Install inferno: cargo install inferno${NC}"
            else
                echo -e "${YELLOW}Flamegraph generation skipped. Install inferno: cargo install inferno${NC}"
                echo "Sample data is available at: $OUTPUT_DIR/sample.txt"
            fi
        fi

    elif [[ "$OSTYPE" == "linux"* ]]; then
        # Linux: Convert perf.data to flamegraph
        if [ -f "$OUTPUT_DIR/perf.data" ]; then
            echo -e "${YELLOW}Converting perf data to flamegraph...${NC}"

            # Check if inferno is installed
            if command -v inferno-collapse-perf &> /dev/null && command -v inferno-flamegraph &> /dev/null; then
                sudo perf script -i "$OUTPUT_DIR/perf.data" | \
                    inferno-collapse-perf | \
                    inferno-flamegraph > "$OUTPUT_DIR/flamegraph.svg" 2>/dev/null && \
                    echo -e "${GREEN}Flamegraph generated: $OUTPUT_DIR/flamegraph.svg${NC}" || \
                    echo -e "${YELLOW}Could not generate flamegraph${NC}"
            else
                echo -e "${YELLOW}Flamegraph generation skipped. Install inferno: cargo install inferno${NC}"
                echo "Perf data is available at: $OUTPUT_DIR/perf.data"
            fi
        fi
    fi

    # Copy server logs to profiling output
    if [ -f "/tmp/streamflow-benchmark.log" ]; then
        cp /tmp/streamflow-benchmark.log "$OUTPUT_DIR/server.log"
        echo "Server logs saved to: $OUTPUT_DIR/server.log"
    fi

    # Save PostgreSQL query stats
    echo ""
    echo -e "${YELLOW}Collecting PostgreSQL query statistics...${NC}"

    # Get slow queries using psql inside the Docker container
    # Note: Using 0.01ms threshold to capture queries (most queries are sub-millisecond now)
    docker exec streamflow-postgres psql -U streamflow -d streamflow_benchmark -c "
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
        echo -e "${YELLOW}Note: pg_stat_statements data not available${NC}"
        echo "Make sure pg_stat_statements extension is enabled (see docker-compose.yml)"
    fi
fi

# Check if benchmark results were saved by Rust tests
if [ -f "$OUTPUT_DIR/results.json" ]; then
    echo ""
    echo -e "${GREEN}Benchmark results saved to: $OUTPUT_DIR/results.json${NC}"
else
    echo ""
    echo -e "${YELLOW}Note: No results.json generated (tests may have failed or been skipped)${NC}"
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
if [ "$BENCHMARK_SUCCESS" = true ]; then
    echo -e "${GREEN}✅ Benchmarks completed successfully!${NC}"
    echo ""
    echo -e "${BLUE}Output Directory: $OUTPUT_DIR${NC}"
    echo ""
    echo "Benchmark Results:"
    echo "  results.json - Benchmark metrics (throughput, latency, etc.)"

    if [ "$ENABLE_PROFILING" = true ]; then
        echo ""
        echo "Profiling Data:"
        echo "  flamegraph.svg - CPU flamegraph visualization"
        echo "  server.log - Debug server logs"
        echo "  queries.txt - PostgreSQL query statistics"
        echo "  sample.txt or perf.data - Raw profiling data"
        echo ""
        echo "To view the flamegraph:"
        echo "  open $OUTPUT_DIR/flamegraph.svg"
    fi

    if [ -n "$COMPARE_BASELINE" ] && [ -f "$OUTPUT_DIR/comparison.json" ]; then
        echo ""
        echo "Comparison:"
        echo "  comparison.json - Performance comparison vs baseline"
    fi

    exit 0
else
    echo -e "${RED}❌ Benchmarks failed${NC}"
    echo ""
    echo "Tips for debugging:"
    echo "  - Check server logs: tail -f /tmp/streamflow-benchmark.log"
    echo "  - Verify OAuth credentials are set: echo \$STREAMFLOW_CLIENT_ID"
    echo "  - Check workflow status via API"

    # Show output directory even if benchmark failed
    echo ""
    echo -e "${BLUE}Output Directory: $OUTPUT_DIR${NC}"
    if [ "$ENABLE_PROFILING" = true ]; then
        echo "Profiling data (despite failure):"
        if [ -f "$OUTPUT_DIR/flamegraph.svg" ]; then
            echo "  flamegraph.svg"
        fi
        if [ -f "$OUTPUT_DIR/server.log" ]; then
            echo "  server.log"
        fi
        if [ -f "$OUTPUT_DIR/queries.txt" ]; then
            echo "  queries.txt"
        fi
    fi

    exit 1
fi
