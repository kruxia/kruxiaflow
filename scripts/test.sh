#!/bin/bash
#
# StreamFlow Test Runner
#
# Usage:
#   ./scripts/test.sh [OPTIONS]
#
# Options:
#   --help, -h              Show this help message
#   --coverage              Run tests with coverage tracking
#   --coverage-html         Generate HTML coverage report and open in browser
#   --coverage-ci           Generate lcov coverage report for CI/CD
#   --unit                  Run only unit tests
#   --integration           Run only integration tests
#   --doc                   Run only documentation tests
#   --package, -p CRATE     Run tests for specific crate
#   --verbose, -v           Verbose output
#   --nocapture             Don't capture test output
#   --install-coverage      Install cargo-llvm-cov tool
#   --skip-db-setup         Skip database setup (assumes DB is ready)
#
# Examples:
#   ./scripts/test.sh                           # Run all tests
#   ./scripts/test.sh --coverage                # Run with coverage
#   ./scripts/test.sh --coverage-html           # Generate HTML report
#   ./scripts/test.sh -p streamflow-api         # Test only API crate
#   ./scripts/test.sh --unit                    # Unit tests only
#   ./scripts/test.sh --skip-db-setup           # Skip DB setup
#
# Coverage Exclusions:
#   The following files are excluded from coverage reports (dev/tooling):
#   - profiling/src/bin/*           Profiling binaries
#   - profiling/src/client.rs       Profiling HTTP client
#   - profiling/src/metrics.rs      Profiling metrics utilities
#   - streamflow/src/bin/seed-*     Database seeding scripts
#   - streamflow/src/commands/seed_llm.rs  LLM seeding command
#   - streamflow/src/llm_catalog.rs LLM catalog for seeding

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default options
COVERAGE=false
COVERAGE_HTML=false
COVERAGE_CI=false
TEST_TYPE="all"
PACKAGE=""
VERBOSE=""
NOCAPTURE=""
INSTALL_COVERAGE=false
SKIP_DB_SETUP=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# *//'
            exit 0
            ;;
        --coverage)
            COVERAGE=true
            shift
            ;;
        --coverage-html)
            COVERAGE=true
            COVERAGE_HTML=true
            shift
            ;;
        --coverage-ci)
            COVERAGE=true
            COVERAGE_CI=true
            shift
            ;;
        --unit)
            TEST_TYPE="unit"
            shift
            ;;
        --integration)
            TEST_TYPE="integration"
            shift
            ;;
        --doc)
            TEST_TYPE="doc"
            shift
            ;;
        --package|-p)
            PACKAGE="$2"
            shift 2
            ;;
        --verbose|-v)
            VERBOSE="--verbose"
            shift
            ;;
        --nocapture)
            NOCAPTURE="--nocapture"
            shift
            ;;
        --install-coverage)
            INSTALL_COVERAGE=true
            shift
            ;;
        --skip-db-setup)
            SKIP_DB_SETUP=true
            shift
            ;;
        *)
            echo -e "${RED}Error: Unknown option: $1${NC}"
            echo "Run '$0 --help' for usage information"
            exit 1
            ;;
    esac
done

# Function to check if cargo-llvm-cov is installed
check_coverage_tool() {
    if ! command -v cargo-llvm-cov &> /dev/null; then
        echo -e "${YELLOW}Warning: cargo-llvm-cov is not installed${NC}"
        echo "Install it with: cargo install cargo-llvm-cov"
        echo "Or run: $0 --install-coverage"
        exit 1
    fi
}

# Install cargo-llvm-cov if requested
if [ "$INSTALL_COVERAGE" = true ]; then
    echo -e "${BLUE}==>${NC} ${GREEN}Installing cargo-llvm-cov${NC}"
    cargo install cargo-llvm-cov
    echo -e "${GREEN}✓ cargo-llvm-cov installed successfully${NC}"
    exit 0
fi

echo -e "${YELLOW}StreamFlow Test Runner${NC}"
echo "========================================"

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
    DB_NAME="streamflow_test"

    # Drop and recreate test database
    echo -e "${YELLOW}Setting up test database...${NC}"

    # Terminate any existing connections to the test database
    docker exec streamflow-postgres psql -U ${DB_USER} -c "
    SELECT pg_terminate_backend(pid)
    FROM pg_stat_activity
    WHERE datname = '${DB_NAME}' AND pid <> pg_backend_pid();" 2>/dev/null || true

    # Drop and recreate database
    docker exec streamflow-postgres psql -U ${DB_USER} -c "DROP DATABASE IF EXISTS ${DB_NAME};" 2>/dev/null || true
    docker exec streamflow-postgres psql -U ${DB_USER} -c "CREATE DATABASE ${DB_NAME};"

    echo -e "${GREEN}Test database created${NC}"

    # Set DATABASE_URL for test database
    export DATABASE_URL="postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}"

    # Run migrations
    echo -e "${YELLOW}Running migrations...${NC}"
    sqlx migrate run

    echo -e "${GREEN}Migrations complete${NC}"
else
    echo -e "${BLUE}Skipping database setup (--skip-db-setup)${NC}"
    # DATABASE_URL should already be set in environment
    if [ -z "$DATABASE_URL" ]; then
        echo -e "${YELLOW}Warning: DATABASE_URL not set${NC}"
    fi
fi

# Run tests
echo ""
echo -e "${YELLOW}Running tests...${NC}"
echo "========================================"

# Build command based on options
if [ "$COVERAGE" = true ]; then
    check_coverage_tool

    # Base coverage command
    CMD="cargo llvm-cov"

    # Add test type
    case $TEST_TYPE in
        unit)
            CMD="$CMD --lib"
            ;;
        integration)
            CMD="$CMD --tests"
            ;;
        doc)
            CMD="$CMD --doc"
            ;;
        all)
            # Use --lib --bins --tests instead of --all-targets to exclude benchmarks
            # Benchmarks don't accept --test-threads argument
            CMD="$CMD --lib --bins --tests"
            ;;
    esac

    # Add workspace flag
    CMD="$CMD --workspace"

    # Exclude profiling and seed scripts from coverage reports
    # These are development/tooling files that don't need test coverage
    # Combined into single regex: profiling tools OR seed scripts
    CMD="$CMD --ignore-filename-regex '(profiling/src/(bin/|client\\.rs|metrics\\.rs)|streamflow/src/(bin/seed|commands/seed_llm\\.rs|llm_catalog\\.rs))'"

    # Add package filter if specified
    if [ -n "$PACKAGE" ]; then
        CMD="$CMD --package $PACKAGE"
    fi

    # Add verbose flag
    if [ -n "$VERBOSE" ]; then
        CMD="$CMD --verbose"
    fi

    # Handle different coverage output formats (must be before -- separator)
    if [ "$COVERAGE_HTML" = true ]; then
        echo -e "${BLUE}Generating HTML coverage report${NC}"
        CMD="$CMD --html"
    elif [ "$COVERAGE_CI" = true ]; then
        echo -e "${BLUE}Generating lcov coverage report for CI${NC}"
        mkdir -p coverage
        CMD="$CMD --lcov --output-path coverage/lcov.info"
    fi

    # Test arguments (including test-threads) - must come after all cargo-llvm-cov flags
    TEST_ARGS="--test-threads=1"
    if [ -n "$NOCAPTURE" ]; then
        TEST_ARGS="$TEST_ARGS --nocapture"
    fi
    CMD="$CMD -- $TEST_ARGS"

    # Execute the command
    eval "$CMD"

    # Post-execution handling
    if [ "$COVERAGE_HTML" = true ]; then
        # Try to open the HTML report
        HTML_REPORT="target/llvm-cov/html/index.html"
        if [ -f "$HTML_REPORT" ]; then
            echo ""
            echo -e "${GREEN}✓ Coverage report generated${NC}"
            echo -e "Report location: ${BLUE}$HTML_REPORT${NC}"

            # Open in browser (cross-platform)
            if command -v open &> /dev/null; then
                open "$HTML_REPORT"
            elif command -v xdg-open &> /dev/null; then
                xdg-open "$HTML_REPORT"
            else
                echo -e "${YELLOW}Could not automatically open browser${NC}"
            fi
        fi
    elif [ "$COVERAGE_CI" = true ]; then
        echo ""
        echo -e "${GREEN}✓ Coverage report generated${NC}"
        echo -e "Report location: ${BLUE}coverage/lcov.info${NC}"
    fi

    echo ""
    echo -e "${GREEN}✅ All tests passed with coverage tracking!${NC}"
    exit 0

else
    # Regular test execution without coverage
    CMD="cargo test"

    # Add test type
    case $TEST_TYPE in
        unit)
            CMD="$CMD --lib"
            ;;
        integration)
            CMD="$CMD --tests"
            ;;
        doc)
            CMD="$CMD --doc"
            ;;
        all)
            # Use explicit flags to exclude benchmarks (they don't accept --test-threads)
            CMD="$CMD --workspace --lib --bins --tests --doc"
            ;;
    esac

    # Add package filter if specified
    if [ -n "$PACKAGE" ]; then
        CMD="$CMD --package $PACKAGE"
    fi

    # Add verbose flag
    if [ -n "$VERBOSE" ]; then
        CMD="$CMD --verbose"
    fi

    # Test arguments
    TEST_ARGS="--test-threads=1"
    if [ -n "$NOCAPTURE" ]; then
        TEST_ARGS="$TEST_ARGS --nocapture"
    fi
    CMD="$CMD -- $TEST_ARGS"

    if eval "$CMD"; then
        echo ""
        echo -e "${GREEN}✅ All tests passed!${NC}"
        exit 0
    else
        echo ""
        echo -e "${RED}❌ Tests failed${NC}"
        exit 1
    fi
fi
