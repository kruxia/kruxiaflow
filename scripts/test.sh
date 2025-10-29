#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}StreamFlow Test Runner${NC}"
echo "========================================"

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
DB_PORT="5433"
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

# Run tests
echo ""
echo -e "${YELLOW}Running tests...${NC}"
echo "========================================"

if cargo test --all -- --test-threads=1; then
    echo ""
    echo -e "${GREEN}✅ All tests passed!${NC}"
    exit 0
else
    echo ""
    echo -e "${RED}❌ Tests failed${NC}"
    exit 1
fi
