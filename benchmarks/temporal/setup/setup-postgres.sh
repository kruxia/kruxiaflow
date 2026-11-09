#!/bin/sh
# Creates the Temporal and visibility databases and brings their schemas up to
# date. Runs once from the temporalio/admin-tools image before the server
# starts; follows compose/scripts/setup-postgres.sh in
# https://github.com/temporalio/temporal-server-operations.
set -eu

: "${POSTGRES_SEEDS:?POSTGRES_SEEDS is required}"
: "${POSTGRES_USER:?POSTGRES_USER is required}"
PORT=${DB_PORT:-5432}

echo 'Waiting for PostgreSQL...'
until nc -z -w 10 "$POSTGRES_SEEDS" "$PORT"; do sleep 1; done

for db in temporal temporal_visibility; do
  case $db in
    temporal) schema=/etc/temporal/schema/postgresql/v12/temporal/versioned ;;
    *) schema=/etc/temporal/schema/postgresql/v12/visibility/versioned ;;
  esac
  temporal-sql-tool --plugin postgres12 --ep "$POSTGRES_SEEDS" -u "$POSTGRES_USER" -p "$PORT" --db "$db" create
  temporal-sql-tool --plugin postgres12 --ep "$POSTGRES_SEEDS" -u "$POSTGRES_USER" -p "$PORT" --db "$db" setup-schema -v 0.0
  temporal-sql-tool --plugin postgres12 --ep "$POSTGRES_SEEDS" -u "$POSTGRES_USER" -p "$PORT" --db "$db" update-schema -d "$schema"
done

echo 'PostgreSQL schema setup complete'
