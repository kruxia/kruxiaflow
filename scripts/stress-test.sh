#!/bin/bash
#
# StreamFlow Stress Test Runner
#
# Runs ramping stress tests to identify system breaking points and capacity limits.
#
# Prerequisites:
#   - StreamFlow server running
#   - PostgreSQL database running with migrations applied
#   - OAuth client credentials set (STREAMFLOW_CLIENT_ID, STREAMFLOW_CLIENT_SECRET)
#
# Usage:
#   ./scripts/stress-test.sh [OPTIONS]
#
# Options:
#   --quick                Run quick test (100 → 1,000 concurrent)
#   --standard             Run standard test (100 → 5,000 concurrent)
#   --full                 Run full test (100 → 10,000 concurrent)
#   --peak N               Custom peak concurrent workflows
#   --step-size N          Custom step size (default: 500)
#   --step-duration SECS   Duration per step (default: 30)
#   --workflow NAME        Workflow definition to use
#   --output-dir DIR       Output directory for results
#   --stop-on-failure      Stop when breaking point detected
#   --help, -h             Show this help message
#
# Examples:
#   ./scripts/stress-test.sh --quick
#   ./scripts/stress-test.sh --standard
#   ./scripts/stress-test.sh --peak 2000 --step-size 200
#   ./scripts/stress-test.sh --full --output-dir my-results

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

PROJECT_DIR="$(dirname "$(dirname "$0")")"

# Default options
MODE=""
PEAK=""
STEP_SIZE=""
STEP_DURATION=""
WORKFLOW=""
OUTPUT_DIR=""
STOP_ON_FAILURE=""
EXTRA_ARGS=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# *//'
            exit 0
            ;;
        --quick)
            MODE="--quick"
            shift
            ;;
        --standard)
            MODE="--standard"
            shift
            ;;
        --full)
            MODE="--full"
            shift
            ;;
        --peak)
            PEAK="--peak-concurrent $2"
            shift 2
            ;;
        --step-size)
            STEP_SIZE="--step-size $2"
            shift 2
            ;;
        --step-duration)
            STEP_DURATION="--step-duration $2"
            shift 2
            ;;
        --workflow)
            WORKFLOW="--workflow $2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="--output-dir $2"
            shift 2
            ;;
        --stop-on-failure)
            STOP_ON_FAILURE="--stop-on-failure true"
            shift
            ;;
        *)
            echo -e "${RED}Error: Unknown option: $1${NC}"
            echo "Run '$0 --help' for usage information"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}"
echo "╔══════════════════════════════════════════════════════════════════╗"
echo "║              StreamFlow Stress Test Runner                       ║"
echo "╚══════════════════════════════════════════════════════════════════╝"
echo -e "${NC}"

# Check required environment variables
if [ -z "${STREAMFLOW_CLIENT_ID:-}" ]; then
    echo -e "${RED}Error: STREAMFLOW_CLIENT_ID environment variable not set${NC}"
    echo "Please set OAuth client credentials in your environment"
    exit 1
fi

if [ -z "${STREAMFLOW_CLIENT_SECRET:-}" ]; then
    echo -e "${RED}Error: STREAMFLOW_CLIENT_SECRET environment variable not set${NC}"
    echo "Please set OAuth client credentials in your environment"
    exit 1
fi

# Check server is running
PORT="${STREAMFLOW_PORT:-8080}"
BASE_URL="${STREAMFLOW_BASE_URL:-http://localhost:$PORT}"

echo -e "${BLUE}Checking server at ${BASE_URL}...${NC}"
if ! curl -sf "${BASE_URL}/health" > /dev/null 2>&1; then
    echo -e "${RED}Error: StreamFlow server not accessible at ${BASE_URL}${NC}"
    echo ""
    echo "Please start the server before running stress tests:"
    echo "  streamflow serve --port ${PORT}"
    exit 1
fi
echo -e "${GREEN}Server is accessible${NC}"
echo ""

# Export base URL for stress test binary
export STREAMFLOW_BASE_URL="${BASE_URL}"

# Register workflow definitions if needed
echo -e "${BLUE}Registering workflow definitions...${NC}"
if cargo run --package streamflow-profiling --bin register-workflows --release 2>&1; then
    echo -e "${GREEN}Workflow definitions registered${NC}"
else
    echo -e "${YELLOW}Warning: Could not register workflow definitions${NC}"
    echo "Continuing with existing definitions..."
fi
echo ""

# Build command
CMD="cargo run --package streamflow-profiling --bin stress-test --release --"

# Add options
if [ -n "$MODE" ]; then
    CMD="$CMD $MODE"
fi
if [ -n "$PEAK" ]; then
    CMD="$CMD $PEAK"
fi
if [ -n "$STEP_SIZE" ]; then
    CMD="$CMD $STEP_SIZE"
fi
if [ -n "$STEP_DURATION" ]; then
    CMD="$CMD $STEP_DURATION"
fi
if [ -n "$WORKFLOW" ]; then
    CMD="$CMD $WORKFLOW"
fi
if [ -n "$OUTPUT_DIR" ]; then
    CMD="$CMD $OUTPUT_DIR"
fi
if [ -n "$STOP_ON_FAILURE" ]; then
    CMD="$CMD $STOP_ON_FAILURE"
fi

echo -e "${YELLOW}Running stress test...${NC}"
echo "Command: $CMD"
echo ""

# Run the stress test
eval "$CMD"

EXIT_CODE=$?

echo ""
if [ $EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}╔══════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║              Stress Test Completed Successfully                  ║${NC}"
    echo -e "${GREEN}╚══════════════════════════════════════════════════════════════════╝${NC}"
else
    echo -e "${RED}╔══════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║              Stress Test Failed                                  ║${NC}"
    echo -e "${RED}╚══════════════════════════════════════════════════════════════════╝${NC}"
fi

exit $EXIT_CODE
