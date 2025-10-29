# StreamFlow - Lightweight Workflow Orchestration

StreamFlow is a high-performance workflow orchestration platform designed for edge-to-cloud deployment. Built as a single binary with PostgreSQL as the only required dependency.

## Status

**Current Version**: 0.2.0 MVP
**Implementation Status**: US-1.1 Activity Queue - ✅ Complete

### Completed Features

- ✅ **Activity Queue** (US-1.1)
  - PostgreSQL-based queue with FOR UPDATE SKIP LOCKED for safe concurrency
  - Idempotent scheduling via UNIQUE constraints
  - Stale activity detection and automatic retry
  - Heartbeat support for long-running activities
  - Background cleanup for failed activities
  - Comprehensive test suite (9 tests, all passing)

## Quick Start

### Prerequisites

- Rust 1.90.0+
- Docker (for PostgreSQL)
- sqlx-cli: `cargo install sqlx-cli --no-default-features --features postgres`

### Setup

**Quick Start** (automated):
```bash
# Set up development database
./scripts/setup-dev-db.sh

# Run tests
./scripts/test.sh

# Build
cargo build --release
```

**Manual Setup**:

1. **Start PostgreSQL**:
   ```bash
   docker-compose up -d postgres
   ```

2. **Run migrations**:
   ```bash
   export DATABASE_URL='postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow'
   sqlx migrate run
   ```

3. **Build**:
   ```bash
   cargo build --release
   ```

4. **Run tests**:
   ```bash
   ./scripts/test.sh
   ```

## Project Structure

```
streamflow/
├── core/           # Core orchestration engine (activity queue, event sourcing)
├── api/            # API server (HTTP/WebSocket endpoints) - TODO
├── activity/       # Built-in activities and worker - TODO
├── dashboard/      # Web UI for monitoring - TODO
├── migrations/     # Database migrations
└── docs/           # Architecture and implementation docs
```

## Architecture

StreamFlow uses a service-oriented architecture with pluggable interfaces:

- **ActivityQueue**: Schedules and manages activity execution
- **EventSource**: Publishes and consumes workflow events (TODO)
- **WorkflowStorage**: Handles large artifacts and files (TODO)
- **AuthenticationService**: JWT-based authentication (TODO)

### MVP Implementation

- **Database**: PostgreSQL 18+ (all services)
- **Queue**: PostgreSQL with optimized indexes
- **Event Stream**: PostgreSQL polling with adaptive backoff (TODO)
- **Storage**: PostgreSQL Large Objects (TODO)
- **Auth**: Custom JWT provider with PostgreSQL backend (TODO)

## Development

### Database Management

```bash
# Create a new migration
sqlx migrate add migration_name

# Run migrations
sqlx migrate run

# Revert last migration
sqlx migrate revert

# Prepare query cache for offline compilation (commit the .sqlx/ directory)
cargo sqlx prepare --workspace
```

### Testing

Tests require a running PostgreSQL instance. The recommended way to run tests is using the test script, which creates a clean test database:

```bash
./scripts/test.sh
```

This script will:
1. Ensure PostgreSQL is running
2. Drop and recreate the `streamflow_test` database
3. Run migrations
4. Execute all tests with proper isolation

**Manual testing:**
```bash
export DATABASE_URL='postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow_test'
cargo test --all -- --test-threads=1
```

**Note**: Use `--test-threads=1` to avoid race conditions between tests.

### Environment Variables

Copy `.env.example` to `.env` and adjust as needed:

```bash
DATABASE_URL=postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow
STREAMFLOW_QUEUE_POLL_INTERVAL=100ms
STREAMFLOW_QUEUE_DEFAULT_TIMEOUT=60s
STREAMFLOW_QUEUE_DEFAULT_MAX_RETRIES=3
```

## Documentation

- [Architecture](docs/architecture.md) - System design and component overview
- [Implementation Plans](docs/implementation/) - User story implementation details

## Roadmap

### Current Sprint
- ✅ US-1.1: Activity Queue with Ordering Guarantees

### Next Up
- US-1.2: Event-Driven Dynamic Scheduling (Orchestrator)
- US-1.3: Worker Polling with Concurrency Safety
- API Server with authentication

## Performance Targets

- **Throughput**: >1,000 workflows/sec (>10,000 activities/sec)
- **Latency**: <10ms P99 workflow start, <1ms orchestrator evaluation
- **Footprint**: <50MB base RAM, 10-15MB binary size

## License

MIT
