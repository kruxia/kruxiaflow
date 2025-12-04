#!/bin/bash
# Docker entrypoint script for StreamFlow profiling container

# Load auth environment variables
if [ -z "$STREAMFLOW_CLIENT_ID" ] || [ -z "$STREAMFLOW_CLIENT_SECRET" ]; then
    echo "Error: STREAMFLOW_CLIENT_ID and STREAMFLOW_CLIENT_SECRET must be set in the environment."
    exit 1
fi

# Load private and public keys if they haven't been provided (this is a development
# and testing container!)
if [ -z "$STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM" ] || [ -z "$STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM" ]; then
    echo "Loading development OAuth RSA key pair from ./dev-keys for development/testing..."
    export STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM=$(cat dev-keys/private.pem | tr -d '\n')
    export STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM=$(cat dev-keys/public.pem | tr -d '\n')
fi

# Set up profiling environment variables
export PROFILE_DIR="/opt/var/memory"
mkdir -p "$PROFILE_DIR"
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:$PROFILE_DIR/jeprof.out,lg_prof_interval:30"

# Wait for PostgreSQL to be ready
until psql ${DATABASE_ADMIN_URL} -c "select 1" > /dev/null 2>&1; do
    echo "Waiting for PostgreSQL to be ready..."
    sleep 1
done

# Create benchmark database if it doesn't exist
psql ${DATABASE_ADMIN_URL} -c "CREATE DATABASE streamflow_profiling" >/dev/null 2>&1

# Run database migrations
sqlx migrate run

# Build the profiling binary
echo "Building StreamFlow profiling binary..."
cargo build --profile profiling --features profiling

# Seed the oauth client for profiling (idempotent - skips if exists)
target/profiling/streamflow seed-client

# Start memory monitoring in background
MONITOR_OUTPUT="$PROFILE_DIR/memory_usage.csv"
echo "timestamp,rss_mb,vsz_mb,cpu_percent" > "$MONITOR_OUTPUT"

# Memory monitoring function
monitor_memory() {
    local server_pid=$1
    while kill -0 $server_pid 2>/dev/null; do
        # Get memory and CPU stats from /proc
        if [ -f "/proc/$server_pid/status" ]; then
            local timestamp=$(date +%s)
            local rss=$(grep VmRSS /proc/$server_pid/status | awk '{print $2}')
            local vsz=$(grep VmSize /proc/$server_pid/status | awk '{print $2}')
            local rss_mb=$((rss / 1024))
            local vsz_mb=$((vsz / 1024))

            # Get CPU usage (simplified)
            local cpu_percent=$(ps -p $server_pid -o %cpu= | tr -d ' ')

            echo "$timestamp,$rss_mb,$vsz_mb,$cpu_percent" >> "$MONITOR_OUTPUT"
        fi
        sleep 2
    done
}

echo "Starting StreamFlow server with memory monitoring..."
echo "Memory usage will be logged to: $MONITOR_OUTPUT"

# Start server in background so we can monitor it
target/profiling/streamflow serve "$@" &
SERVER_PID=$!

echo "StreamFlow server started (PID: $SERVER_PID)"

# Start monitoring in background
monitor_memory $SERVER_PID &
MONITOR_PID=$!

# Wait for server to exit
wait $SERVER_PID
SERVER_EXIT_CODE=$?

# Stop monitoring
kill $MONITOR_PID 2>/dev/null || true

echo "StreamFlow server exited with code: $SERVER_EXIT_CODE"
exit $SERVER_EXIT_CODE