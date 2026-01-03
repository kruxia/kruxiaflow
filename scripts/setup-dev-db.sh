#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}Kruxia Flow Development Database Setup${NC}"
echo "========================================"

# Start PostgreSQL via docker-compose
echo -e "${YELLOW}Starting PostgreSQL 18...${NC}"
docker-compose up -d postgres

# Wait for PostgreSQL to be ready
echo "Waiting for PostgreSQL to be ready..."
until docker exec kruxiaflow-postgres pg_isready -U kruxiaflow > /dev/null 2>&1; do
  sleep 1
done

echo -e "${GREEN}PostgreSQL is ready!${NC}"

# Database configuration
DB_USER="kruxiaflow"
DB_PASSWORD="kruxiaflow_dev"
DB_HOST="127.0.0.1"
DB_PORT="5433"
DB_NAME="kruxiaflow"

# Set DATABASE_URL for migrations
export DATABASE_URL="postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}"

# Run migrations
echo -e "${YELLOW}Running database migrations...${NC}"
cd "$(dirname "$0")/.."
sqlx migrate run

echo -e "${GREEN}✅ Database setup complete!${NC}"
echo ""
echo "Development database: ${DB_NAME}"
echo "Connection: ${DATABASE_URL}"
echo ""
echo "Next steps:"
echo "  - Run tests: ./scripts/test.sh"
echo "  - Prepare sqlx queries: cargo sqlx prepare --workspace"
echo "  - Build: cargo build --release"
