#!/bin/sh
#
# StreamFlow Deploy Container Entrypoint
#
# Runs migrations and seeds OAuth client before starting the server.
# Also supports running management commands directly.
#
# Usage:
#   docker compose --profile deploy up              # Start server
#   docker compose run streamflow-deploy migrate    # Run migrations only
#   docker compose run streamflow-deploy sqlx ...   # Run sqlx commands

set -eu

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { printf "${GREEN}[streamflow]${NC} %s\n" "$1"; }
log_warn() { printf "${YELLOW}[streamflow]${NC} %s\n" "$1"; }

# Load RSA keys from file if not provided as environment variables
load_keys() {
    if [ -z "${STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM:-}" ]; then
        if [ -n "${STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE:-}" ] && [ -f "$STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE" ]; then
            log_info "Loading private key from $STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE"
            STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM=$(cat "$STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE")
            export STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM
        elif [ -f "/secrets/private.pem" ]; then
            log_info "Loading private key from /secrets/private.pem"
            STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM=$(cat /secrets/private.pem)
            export STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM
        fi
    fi

    if [ -z "${STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM:-}" ]; then
        if [ -n "${STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE:-}" ] && [ -f "$STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE" ]; then
            log_info "Loading public key from $STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE"
            STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM=$(cat "$STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE")
            export STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM
        elif [ -f "/secrets/public.pem" ]; then
            log_info "Loading public key from /secrets/public.pem"
            STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM=$(cat /secrets/public.pem)
            export STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM
        fi
    fi
}

# Wait for PostgreSQL to be ready
wait_for_postgres() {
    log_info "Waiting for PostgreSQL..."
    until sqlx database create 2>/dev/null || sqlx migrate info >/dev/null 2>&1; do
        sleep 1
    done
    log_info "PostgreSQL is ready"
}

# Run database migrations
run_migrations() {
    log_info "Running database migrations..."
    cd /opt && sqlx migrate run
    log_info "Migrations complete"
}

# Seed OAuth client if not already seeded
seed_oauth_client() {
    if [ -z "${STREAMFLOW_CLIENT_ID:-}" ] || [ -z "${STREAMFLOW_CLIENT_SECRET:-}" ]; then
        log_warn "STREAMFLOW_CLIENT_ID or STREAMFLOW_CLIENT_SECRET not set, skipping OAuth seed"
        return 0
    fi

    log_info "Seeding OAuth client..."
    seed-oauth-client || log_warn "OAuth client may already exist (this is OK)"
}

# Main entrypoint
main() {
    cmd="${1:-serve}"

    # Handle special commands that don't need full initialization
    case "$cmd" in
        sqlx|migrate|--help|-h|version|--version)
            exec "$@"
            ;;
    esac

    # Load configuration
    load_keys

    # Validate required configuration for serve
    if [ "$cmd" = "serve" ]; then
        if [ -z "${DATABASE_URL:-}" ]; then
            echo "Error: DATABASE_URL is required" >&2
            exit 1
        fi

        if [ -z "${STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM:-}" ]; then
            echo "Error: RSA private key not found. Run scripts/init.sh first." >&2
            exit 1
        fi
    fi

    # Initialize database for serve command
    if [ "$cmd" = "serve" ]; then
        wait_for_postgres
        run_migrations
        seed_oauth_client
    fi

    # Execute command
    log_info "Starting: streamflow $*"
    exec streamflow "$@"
}

main "$@"
