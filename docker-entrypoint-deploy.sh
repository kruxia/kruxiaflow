#!/bin/bash
# Docker entrypoint script for StreamFlow deployment container

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

# Wait for PostgreSQL to be ready
until psql $DATABASE_URL -c "select 1" > /dev/null 2>&1; do
    echo "Waiting for PostgreSQL to be ready..."
    sleep 1
done

# Run database migrations
sqlx migrate run

# Seed the oauth client for profiling
/opt/target/release/seed-oauth-client

# Start StreamFlow server
/opt/target/release/streamflow "$@"
