#!/bin/bash
#
# Kruxia Flow sqlx Offline Cache Regenerator
#
# Regenerates the checked-in .sqlx offline query cache for the WHOLE workspace,
# including test targets. CI runs clippy/check with SQLX_OFFLINE=true and
# --all-targets, so every sqlx macro invocation — including those in tests —
# must have a cache entry. Plain `cargo sqlx prepare` silently skips test
# targets; this script always passes the required flags.
#
# Runs against a dedicated throwaway PostgreSQL container on a non-default
# port. It deliberately ignores any ambient DATABASE_URL: port 5432 may be
# owned by another project's postgres, and preparing against a drifted or
# foreign schema produces a wrong cache.
#
# Usage:
#   ./scripts/sqlx-prepare.sh [OPTIONS]
#
# Options:
#   --help, -h     Show this help message
#   --port PORT    Host port for the throwaway postgres (default: 5439)
#   --keep         Leave the throwaway container running (faster re-runs)
#   --verify       After preparing, run the offline workspace check CI runs
#
# After a successful run, commit the changed files under .sqlx/ — CI fails
# without them.

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

CONTAINER=kf-sqlx-prepare
PORT=5439
KEEP=false
VERIFY=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# *//'
            exit 0
            ;;
        --port)
            PORT="$2"
            shift 2
            ;;
        --keep)
            KEEP=true
            shift
            ;;
        --verify)
            VERIFY=true
            shift
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}" >&2
            exit 1
            ;;
    esac
done

cd "$(dirname "$0")/.."

if ! command -v sqlx > /dev/null 2>&1 && ! cargo sqlx --version > /dev/null 2>&1; then
    echo -e "${RED}sqlx-cli is not installed.${NC}"
    echo "Install it with: cargo install sqlx-cli --no-default-features --features postgres,rustls"
    exit 1
fi

# The prepare must not inherit ambient sqlx settings: SQLX_OFFLINE would fight
# the recording mode, and an ambient DATABASE_URL may point at the wrong server.
unset SQLX_OFFLINE
export DATABASE_URL="postgres://kruxiaflow:kruxiaflow_prepare@127.0.0.1:${PORT}/kruxiaflow"

if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER}$"; then
    echo -e "${YELLOW}Reusing running ${CONTAINER} container${NC}"
else
    docker rm -f "$CONTAINER" > /dev/null 2>&1 || true
    echo "Starting throwaway postgres:17 on port ${PORT}..."
    docker run -d --name "$CONTAINER" \
        -e POSTGRES_USER=kruxiaflow \
        -e POSTGRES_PASSWORD=kruxiaflow_prepare \
        -e POSTGRES_DB=kruxiaflow \
        -p "${PORT}:5432" \
        postgres:17 > /dev/null
fi

echo -n "Waiting for postgres to accept connections"
for _ in $(seq 1 30); do
    if docker exec "$CONTAINER" pg_isready -U kruxiaflow > /dev/null 2>&1; then
        break
    fi
    echo -n "."
    sleep 1
done
echo

if ! docker exec "$CONTAINER" pg_isready -U kruxiaflow > /dev/null 2>&1; then
    echo -e "${RED}Postgres did not become ready within 30s${NC}"
    exit 1
fi

echo "Running migrations..."
sqlx migrate run

echo "Regenerating offline cache (workspace, all targets)..."
cargo sqlx prepare --workspace -- --all-targets

if [ "$VERIFY" = true ]; then
    echo "Verifying with the offline check CI runs..."
    env -u DATABASE_URL SQLX_OFFLINE=true cargo check --workspace --all-targets
fi

if [ "$KEEP" = true ]; then
    echo -e "${YELLOW}Leaving ${CONTAINER} running (port ${PORT}); remove with: docker rm -f ${CONTAINER}${NC}"
else
    docker rm -f "$CONTAINER" > /dev/null
fi

CHANGED=$(git status --short .sqlx | wc -l | tr -d ' ')
if [ "$CHANGED" -gt 0 ]; then
    echo -e "${GREEN}Done. ${CHANGED} changed file(s) under .sqlx/ — commit them, CI fails without them:${NC}"
    git status --short .sqlx | head -10
    [ "$CHANGED" -gt 10 ] && echo "  ... and $((CHANGED - 10)) more"
else
    echo -e "${GREEN}Done. Cache already up to date — nothing to commit.${NC}"
fi
