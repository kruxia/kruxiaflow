# StreamFlow - Lightweight Workflow Orchestration

StreamFlow is a high-performance workflow orchestration engine designed for edge-to-cloud deployment. Built as a **single Rust binary** with PostgreSQL as the only required dependency.

## Why StreamFlow?

- **🚀 10x Performance**: Event-driven architecture targets >1,000 workflows/sec (vs Temporal's 35-100/sec)
- **📦 Minimal Footprint**: <15MB binary, <50MB RAM (vs Temporal's multi-GB deployment)
- **⚡ Edge-Ready**: Runs on Raspberry Pi Zero for edge AI and IoT workflows
- **🔧 PostgreSQL-Only**: No Kafka, Cassandra, or Elasticsearch required for MVP
- **🎯 AI-Native**: Built-in LLM cost tracking, budget enforcement, and result caching
- **🔌 Pluggable Architecture**: Swap PostgreSQL for Kafka, Redis, S3, etc. post-MVP

## Status

**Current Version**: 0.2.0 MVP (In Development)
**Implementation Phase**: Epic 1 - Core Orchestration

### Completed Features

- ✅ **Epic 1: Event-Driven Orchestration Architecture**
  - ✅ **[US-1.1: Activity Queue](docs/implementation/US-1.1-activity-queue.md)** - PostgreSQL-based queue with safe concurrency
  - ✅ **[US-1.2: Event-Driven Dynamic Scheduling](docs/implementation/US-1.2-event-driven-scheduling.md)** - Reactive orchestrator
- ✅ **Epic 1A: API Server** (Partial)
  - ✅ **[US-1A.1: Health Check and Service Discovery](docs/implementation/US-1A.1-health-checks.md)** - Liveness/readiness probes with parallel health checks
  - ✅ **[US-1A.1.5: API Server CLI Launcher](docs/implementation/US-1A.1.5-api-server-launcher.md)** - `streamflow api` command with configuration management
  - ✅ **[US-1A.2: Error Handling and API Contracts](docs/implementation/US-1A.2-error-handling.md)** - Standard error responses, OpenAPI/ReDoc docs, CORS, request tracing

### In Progress

- 🔨 **Epic 1A: API Server** - HTTP/REST endpoints for workflow and worker operations
- 🔨 **Epic 1B: Built-in Worker** - Worker implementation using API endpoints

## Quick Start

### Prerequisites

- Rust 1.90.0+
- Docker (for PostgreSQL 18+)
- sqlx-cli: `cargo install sqlx-cli --no-default-features --features postgres`

### Setup

**Quick Start** (automated):
```bash
# Set up development database
./scr/setup-dev-db.sh

# Run tests
./scr/test.sh

# Build
cargo build --release

# Run API server
export DATABASE_URL='postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow'
./target/release/streamflow api
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
   ./scr/test.sh
   ```

5. **Run API server**:
   ```bash
   export DATABASE_URL='postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow'
   ./target/release/streamflow api
   # Or with custom port:
   ./target/release/streamflow api --port 9090 --bind 127.0.0.1
   ```

## Project Structure

```
streamflow/
├── streamflow/     # Main binary and CLI (streamflow api, future: serve, orchestrator, worker)
├── core/           # Core orchestration engine (activity queue, event sourcing)
├── api/            # API server library (HTTP/WebSocket endpoints) - In Progress
├── activity/       # Built-in activities and worker - TODO
├── dashboard/      # Web UI for monitoring - TODO
├── migrations/     # Database migrations
└── docs/           # Architecture and implementation docs
```

## Architecture

StreamFlow uses an event-driven, service-oriented architecture with pluggable interfaces:

### Core Components (✅ Implemented)
- **ActivityQueue**: Schedules and manages activity execution
  - PostgreSQL-based with FOR UPDATE SKIP LOCKED for safe concurrency
  - Idempotent scheduling and automatic retry
- **EventSource**: Publishes and consumes workflow events
  - PostgreSQL polling with adaptive backoff (10ms-5s)
  - Guaranteed event delivery (no LISTEN/NOTIFY to avoid message loss)
- **Orchestrator**: Reactive workflow evaluation engine
  - Event-driven scheduling (no polling loops)
  - Materialized workflow state for <1ms evaluation
  - Dependency graph resolution

### In Progress Components
- **API Server**: HTTP/REST endpoints for workflow and worker operations
- **WorkflowStorage**: Handles large artifacts and files
- **AuthenticationService**: JWT-based authentication
- **Built-in Worker**: Activity execution using API endpoints

### MVP Implementation Strategy

**All services use PostgreSQL** for MVP simplicity:
- **Database**: PostgreSQL 18+
- **Queue**: PostgreSQL with optimized indexes
- **Event Stream**: PostgreSQL polling (guaranteed delivery)
- **Storage**: PostgreSQL Large Objects (planned)
- **Auth**: Custom JWT provider with PostgreSQL backend (planned)

**Architectural Decision: Built-in Worker Uses API Server**

The built-in worker authenticates via JWT and uses the same HTTP API endpoints as external workers. This ensures:
- Code path consistency (no behavior divergence)
- Automatic API testing through built-in worker usage
- Future flexibility (easy to separate into standalone service)

See [Architecture Documentation](docs/architecture.md) for detailed design rationale and tradeoff analysis.

### Post-MVP: External Service Integrations

After MVP validation, service interfaces can be swapped for:
- **EventSource**: Kafka/Redpanda (>100k events/sec), NATS JetStream (<1ms latency)
- **ActivityQueue**: AWS SQS, RabbitMQ, Redis (for managed services)
- **WorkflowStorage**: S3-compatible storage
- **Auth**: Auth0, Okta (for SSO integration)

See [Post-MVP Roadmap](docs/post-mvp.md) for details.

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
./scr/test.sh
```

This script will:
1. Ensure PostgreSQL is running
2. Drop and recreate the `streamflow_test` database
3. Run migrations
4. Execute all tests with proper isolation

**Test Coverage:**
```bash
# Run tests with coverage
./scr/test.sh --coverage

# Generate HTML coverage report
./scr/test.sh --coverage-html

# Test specific crate
./scr/test.sh -p streamflow-api --coverage
```

**More options:**
```bash
./scr/test.sh --help  # See all options
```

See [docs/testing.md](docs/testing.md) for comprehensive testing guide.

**Manual testing:**
```bash
export DATABASE_URL='postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow_test'
cargo test --all -- --test-threads=1
```

**Note**: Use `--test-threads=1` to avoid race conditions between tests.

### Running the API Server

```bash
# Using environment variables
export DATABASE_URL='postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow'
./target/release/streamflow api

# Using CLI flags
./target/release/streamflow api \
  --database-url postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow \
  --port 8080 \
  --bind 0.0.0.0 \
  --log-level info \
  --log-format text

# View help
./target/release/streamflow --help
./target/release/streamflow api --help
```

Health endpoints will be available at:
- http://localhost:8080/health - Liveness probe
- http://localhost:8080/health/ready - Readiness probe
- http://localhost:8080/api/v1/info - Service info

### Environment Variables

Copy `.env.example` to `.env` and adjust as needed:

```bash
# Database
DATABASE_URL=postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow
DATABASE_URL=postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow

# API Server
STREAMFLOW_API_PORT=8080
STREAMFLOW_API_BIND=0.0.0.0

# Logging
STREAMFLOW_LOG_LEVEL=info
STREAMFLOW_LOG_FORMAT=text

# Queue Configuration
STREAMFLOW_QUEUE_POLL_INTERVAL=100ms
STREAMFLOW_QUEUE_DEFAULT_TIMEOUT=60s
STREAMFLOW_QUEUE_DEFAULT_MAX_RETRIES=3
```

## Documentation

### Core Documentation
- **[MVP Requirements](docs/mvp-requirements.md)** - Complete product requirements document
  - Epic definitions and user stories
  - Implementation roadmap and phases
  - Performance targets and success criteria
  - Architecture decisions and tradeoffs
- **[Architecture](docs/architecture.md)** - System design and component overview
  - Event-driven orchestration design
  - Service interface patterns
  - Database schema and optimization strategies
  - Built-in worker architectural decisions
- **[Implementation Plans](docs/implementation/)** - Detailed user story implementations
  - [US-1.1: Activity Queue](docs/implementation/US-1.1-activity-queue.md)
  - [US-1.2: Event-Driven Scheduling](docs/implementation/US-1.2-event-driven-scheduling.md)
  - [US-1A.1: Health Check and Service Discovery](docs/implementation/US-1A.1-health-checks.md)
  - [US-1A.1.5: API Server CLI Launcher](docs/implementation/US-1A.1.5-api-server-launcher.md)

### Additional Documentation
- **[Post-MVP Roadmap](docs/post-mvp.md)** - Features deferred beyond MVP

## Roadmap

### Epic 1: Core Orchestration ✅ Complete
- ✅ Activity Queue with Ordering Guarantees
- ✅ Event-Driven Dynamic Scheduling

### Epic 1A: API Server 🔨 In Progress
- ✅ Health Check and Service Discovery
- ✅ API Server CLI Launcher (`streamflow api` command)
- 📋 HTTP/REST endpoints for workflow submission and management
- 📋 Worker activity APIs (poll, heartbeat, complete, fail)
- 📋 JWT authentication and authorization
- 📋 WebSocket streaming for real-time updates

### Epic 1B: Built-in Worker 🔨 In Progress
- Worker implementation using API endpoints
- JWT authentication and token management
- Activity execution and result reporting
- Same code path as external workers (consistency)

### Epic 1C: StreamFlow Binary and CLI 📋 Planned
- Main binary with subcommands (serve, orchestrator, api, worker, migrate)
- All-in-one mode (`streamflow serve`) for single-node deployment
- Individual service launchers for distributed deployment
- Configuration management (CLI flags > env vars > defaults)
- Database migration CLI and graceful shutdown

### Epic 2: Performance Benchmarking 📋 Planned
- Automated performance test suite
- Competitor comparison benchmarks (vs Temporal, Airflow, Conductor)
- PostgreSQL profiling and optimization
- Target: >1,000 workflows/sec validation

### Epic 3: YAML Workflow Definition Language 📋 Planned
- Declarative sequential, parallel, and conditional workflows
- Template expressions and activity settings
- YAML validation and tooling

### Beyond MVP
See [Post-MVP Roadmap](docs/post-mvp.md) for external service integrations, multi-tenancy, advanced features, and enterprise operations.

## Performance Targets

- **Throughput**: >1,000 workflows/sec (>10,000 activities/sec)
- **Latency**: <10ms P99 workflow start, <1ms orchestrator evaluation
- **Footprint**: <50MB base RAM, 10-15MB binary size

## License

MIT
