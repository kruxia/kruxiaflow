#!/bin/bash
#
# Docker-based Memory Profiling Script
#
# This script runs memory profiling inside a Linux Docker container where
# jemalloc's jeprof works correctly with full symbol resolution.
#
# Usage:
#   ./scripts/profile_memory_docker.sh [OPTIONS]
#
# Options:
#   --build             Rebuild the profiling Docker image
#   --bash              Drop into bash shell instead of running profiling
#   --help, -h          Show this help message
#
# The profiling results will be saved to var/memory-profile-TIMESTAMP/
# on your host machine.
#

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Options
BUILD=false
RUN_BASH=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --build)
            BUILD=true
            shift
            ;;
        --bash)
            RUN_BASH=true
            shift
            ;;
        --help|-h)
            grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# *//'
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run '$0 --help' for usage"
            exit 1
            ;;
    esac
done

echo -e "${YELLOW}StreamFlow Docker Memory Profiling${NC}"
echo "=========================================="
echo ""

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not running${NC}"
    echo "Please start Docker and try again"
    exit 1
fi

# Build profiling image if requested or if it doesn't exist
if [ "$BUILD" = true ] || ! docker images | grep -q streamflow-profiling; then
    echo -e "${YELLOW}Building profiling Docker image...${NC}"
    docker-compose build profiling
    echo -e "${GREEN}Image built successfully${NC}"
    echo ""
fi

# Start PostgreSQL if not already running
if ! docker ps | grep -q streamflow-postgres; then
    echo -e "${YELLOW}Starting PostgreSQL...${NC}"
    docker-compose up -d postgres

    # Wait for postgres to be healthy
    echo "Waiting for PostgreSQL to be ready..."
    timeout 30 sh -c 'until docker exec streamflow-postgres pg_isready -U streamflow > /dev/null 2>&1; do sleep 1; done' || {
        echo -e "${RED}Error: PostgreSQL failed to start${NC}"
        exit 1
    }
    echo -e "${GREEN}PostgreSQL ready${NC}"
    echo ""
fi

# Set up database for profiling
echo -e "${YELLOW}Setting up profiling database...${NC}"

# Create benchmark database
docker exec streamflow-postgres psql -U streamflow -c "DROP DATABASE IF EXISTS streamflow_profiling;" 2>/dev/null || true
docker exec streamflow-postgres psql -U streamflow -c "CREATE DATABASE streamflow_profiling;" 2>/dev/null || true

# Run migrations in Docker
docker-compose run --rm profiling sh -c "cd /opt && sqlx migrate run --source migrations --database-url postgres://streamflow:streamflow_dev@postgres:5432/streamflow_profiling"

# Seed OAuth client
docker-compose run --rm profiling sh -c "cd /opt && cargo run --package streamflow-profiling --bin seed-oauth-client"

echo -e "${GREEN}Database ready${NC}"
echo ""

if [ "$RUN_BASH" = true ]; then
    echo -e "${BLUE}Starting interactive bash shell in profiling container...${NC}"
    echo "Run './scripts/profile_memory.sh' to start profiling"
    echo ""
    docker-compose run --rm profiling bash
else
    echo -e "${YELLOW}Running memory profiling in Docker container...${NC}"
    echo "This will:"
    echo "  1. Build StreamFlow with jemalloc profiling (inside Linux container)"
    echo "  2. Run sustained throughput benchmark"
    echo "  3. Analyze heap dumps with full symbol resolution"
    echo ""

    # Run profiling
    docker-compose run --rm profiling ./scripts/profile_memory.sh

    echo ""
    echo -e "${GREEN}Profiling complete!${NC}"
    echo ""

    # Find the latest profiling directory
    LATEST_PROFILE=$(ls -td var/memory-profile-* 2>/dev/null | head -1)

    if [ -n "$LATEST_PROFILE" ]; then
        echo "Results available at:"
        echo "  ${LATEST_PROFILE}/allocation_report.txt"
        echo "  ${LATEST_PROFILE}/callgraph.svg"
        echo ""
        echo "View allocation report:"
        echo "  cat ${LATEST_PROFILE}/allocation_report.txt | head -50"
        echo ""
        echo "View call graph:"
        echo "  open ${LATEST_PROFILE}/callgraph.svg"
    fi
fi

echo ""
echo -e "${BLUE}Next steps:${NC}"
echo "  - Review allocation report for function names and call stacks"
echo "  - Compare multiple runs to track memory growth"
echo "  - Investigate top memory consumers"
