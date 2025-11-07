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
**Implementation Phase**: Pre-Epic 2 Requirements (Phase 2C)
**Next Milestone**: Epic 2 - Performance Benchmarking

### Completed Features ✅

**Epic 1: Event-Driven Orchestration Architecture** (Complete)
- ✅ **[US-1.1: Activity Queue](docs/implementation/US-1.1-activity-queue.md)** - PostgreSQL-based queue with safe concurrency
- ✅ **[US-1.2: Event-Driven Dynamic Scheduling](docs/implementation/US-1.2-event-driven-scheduling.md)** - Reactive orchestrator with <1ms evaluation

**Epic 1A: API Server** (Partial - 5 of 9 stories complete)
- ✅ **[US-1A.1: Health Check and Service Discovery](docs/implementation/US-1A.1-health-checks.md)** - Liveness/readiness probes with parallel health checks
- ✅ **[US-1A.1.5: API Server CLI Launcher](docs/implementation/US-1A.1.5-api-server-launcher.md)** - `streamflow api` command with configuration management
- ✅ **[US-1A.2: Error Handling and API Contracts](docs/implementation/US-1A.2-error-handling.md)** - Standard error responses, OpenAPI/ReDoc docs, CORS, request tracing
- ✅ **[US-1A.3: Authentication](docs/implementation/US-1A.3-authentication.md)** - OAuth 2.0 JWT authentication with RSA256 signing, refresh token rotation
- ✅ **[US-1A.4: Workflow Definition Management](docs/implementation/US-1A.4-workflow-definition-management.md)** - Deploy, version, and manage workflow definitions
- ✅ **[US-1A.5: Workflow Submission API](docs/implementation/US-1A.5-workflow-submission.md)** - Submit workflows with idempotency support

**Epic 1B: Built-in Worker** (Complete)
- ✅ Worker implementation using API endpoints for consistency
- ✅ JWT authentication and token management
- ✅ Activity execution and result reporting

### Current Focus 🎯

**Pre-Epic 2 Requirements** (~41 hours remaining)
- 🔨 **[US-1A.6: Workflow Status Query](docs/implementation/US-1A.6-workflow-status-query.md)** - Query workflow status and activities (~11 hours)
- 📋 **[US-1A.7: Worker Activity APIs](docs/implementation/US-1A.7-worker-activity-apis.md)** - Poll, heartbeat, complete, fail endpoints (~12 hours)
- 📋 **US-1C.1: Main Binary and CLI Framework** - Unified binary with subcommands (~6 hours)
- 📋 **US-1C.2: All-in-One Service Launcher** - `streamflow serve` command (~8 hours)
- 📋 **US-1C.7: Graceful Shutdown** - Clean SIGTERM/SIGINT handling (~4 hours)

### Deferred to Post-Epic 2 📋

These features will be implemented after Epic 2 performance validation informs their design:
- 📋 **US-1A.8**: Activity Results and Output Retrieval (~8 hours)
- 📋 **US-1A.9**: WebSocket Streaming for Real-Time Updates (~15 hours)
- 📋 **US-1C.3**: Individual Service Launchers (~5 hours)
- 📋 **US-1C.4**: Configuration Management (~4 hours)
- 📋 **US-1C.5**: Database Migration CLI (~3 hours)
- 📋 **US-1C.6**: Health Checks and Service Monitoring (~5 hours)

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

### Completed Components (✅)
- **ActivityQueue**: PostgreSQL-based queue with safe concurrency
- **EventSource**: PostgreSQL polling with guaranteed delivery
- **Orchestrator**: Event-driven workflow evaluation
- **AuthenticationService**: OAuth 2.0 JWT authentication with RSA256

### In Progress Components (🔨)
- **API Server**: HTTP/REST endpoints for workflow and worker operations
- **WorkflowStorage**: Handles large artifacts and files
- **Built-in Worker**: Activity execution using API endpoints

### MVP Implementation Strategy

**All services use PostgreSQL** for MVP simplicity:
- **Database**: PostgreSQL 18+
- **Queue**: PostgreSQL with optimized indexes
- **Event Stream**: PostgreSQL polling (guaranteed delivery)
- **Storage**: PostgreSQL Large Objects (planned)
- **Auth**: Custom JWT provider with PostgreSQL backend ✅

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

# API Server
STREAMFLOW_API_PORT=8080
STREAMFLOW_API_BIND=0.0.0.0

# OAuth 2.0 Authentication (Required)
STREAMFLOW_OAUTH_RSA_PRIVATE_KEY_PEM="$(cat private.pem)"
STREAMFLOW_OAUTH_RSA_PUBLIC_KEY_PEM="$(cat public.pem)"  # Optional
STREAMFLOW_OAUTH_JWT_ISSUER=streamflow
STREAMFLOW_OAUTH_JWT_AUDIENCE=streamflow-api
STREAMFLOW_OAUTH_TOKEN_TTL=86400  # 24 hours

# Logging
STREAMFLOW_LOG_LEVEL=info
STREAMFLOW_LOG_FORMAT=text

# Queue Configuration
STREAMFLOW_QUEUE_POLL_INTERVAL=100ms
STREAMFLOW_QUEUE_DEFAULT_TIMEOUT=60s
STREAMFLOW_QUEUE_DEFAULT_MAX_RETRIES=3
```

**Generate RSA keys for authentication:**
```bash
openssl genrsa -out private.pem 2048
openssl rsa -in private.pem -pubout -out public.pem
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
  - **Completed**:
    - [US-1.1: Activity Queue](docs/implementation/US-1.1-activity-queue.md)
    - [US-1.2: Event-Driven Scheduling](docs/implementation/US-1.2-event-driven-scheduling.md)
    - [US-1A.1: Health Check and Service Discovery](docs/implementation/US-1A.1-health-checks.md)
    - [US-1A.1.5: API Server CLI Launcher](docs/implementation/US-1A.1.5-api-server-launcher.md)
    - [US-1A.2: Error Handling and API Contracts](docs/implementation/US-1A.2-error-handling.md)
    - [US-1A.3: Authentication](docs/implementation/US-1A.3-authentication.md)
    - [US-1A.4: Workflow Definition Management](docs/implementation/US-1A.4-workflow-definition-management.md)
    - [US-1A.5: Workflow Submission API](docs/implementation/US-1A.5-workflow-submission.md)
  - **Current Focus**:
    - [US-1A.6: Workflow Status Query](docs/implementation/US-1A.6-workflow-status-query.md)
    - [US-1A.7: Worker Activity APIs](docs/implementation/US-1A.7-worker-activity-apis.md)

### Additional Documentation
- **[Post-MVP Roadmap](docs/post-mvp.md)** - Features deferred beyond MVP

## Roadmap

### Phase 1: Foundation (Weeks 1-4) ✅ Complete
**Epic 1: Event-Driven Orchestration Architecture**
- ✅ Activity Queue with Ordering Guarantees (US-1.1)
- ✅ Event-Driven Dynamic Scheduling (US-1.2)

### Phase 2A: API Server Foundation (Weeks 5-6) ✅ Complete
**Epic 1A: API Server** (Partial)
- ✅ Health Check and Service Discovery (US-1A.1)
- ✅ API Server CLI Launcher - `streamflow api` command (US-1A.1.5)
- ✅ Error Handling and API Contracts - OpenAPI/ReDoc docs (US-1A.2)
- ✅ JWT Authentication and Authorization - OAuth 2.0 with RSA256 (US-1A.3)
- ✅ Workflow Definition Management - Deploy, version, query (US-1A.4)
- ✅ Workflow Submission API - Submit workflows with idempotency (US-1A.5)

### Phase 2B: Built-in Worker (Week 7) ✅ Complete
**Epic 1B: Built-in Worker**
- ✅ Worker implementation using API endpoints
- ✅ JWT authentication and token management
- ✅ Activity execution and result reporting
- ✅ Same code path as external workers (consistency)

### Phase 2C: Pre-Epic 2 Requirements (Weeks 8-9) 🎯 Current Focus
**Minimal viable system for performance benchmarking:**
- 🔨 Workflow Status and Query API (US-1A.6) - ~11 hours
- 📋 Worker Activity APIs (US-1A.7) - ~12 hours
- 📋 Main Binary and CLI Framework (US-1C.1) - ~6 hours
- 📋 All-in-One Service Launcher (US-1C.2) - ~8 hours
- 📋 Graceful Shutdown (US-1C.7) - ~4 hours

**Total: ~41 hours (5-6 days)**

### Phase 3: Performance Benchmarking (Weeks 10-11) 📋 Next
**Epic 2: Validate Architecture Before Additional Features**
- Automated performance test suite (US-2.1)
- Competitor comparison benchmarks - vs Temporal, Airflow, Conductor (US-2.2)
- PostgreSQL performance profiling (US-2.3)
- Stress testing and capacity planning (US-2.4)
- Performance dashboard and monitoring (US-2.5)
- **Target**: Prove >1,000 workflows/sec vs competitors' 35-100/sec

### Phase 4: Complete Epic 1A/1C (Week 12) 📋 Post-Epic 2
**Features informed by Epic 2 performance insights:**
- Activity Results and Output Retrieval (US-1A.8) - ~8 hours
- WebSocket Streaming for Real-Time Updates (US-1A.9) - ~15 hours
- Individual Service Launchers (US-1C.3) - ~5 hours
- Configuration Management (US-1C.4) - ~4 hours
- Database Migration CLI (US-1C.5) - ~3 hours
- Health Checks and Service Monitoring (US-1C.6) - ~5 hours

**Total: ~40 hours (5 days)**

### Phase 5: YAML DSL + Programmatic Definition (Weeks 13-16) 📋 Planned
**Epic 3: YAML Workflow Definition Language**
- Declarative sequential, parallel, and conditional workflows
- Template expressions and activity settings
- YAML validation and tooling

**Epic 4: Python/JavaScript Builder APIs**
- Compilation pipeline (code → YAML)
- 5+ examples per language

### Phase 6: PostgreSQL Optimization (Weeks 17-20) 📋 Planned
**Epic 6: Query optimization based on Epic 2 insights**
- Connection pooling and batching
- Advanced indexing strategy
- Partitioning for time-series data
- Target validation: >1,000 workflows/sec sustained

### Phase 7: Developer Experience (Weeks 21-24) 📋 Planned
**Epic 9: Tools and Migration**
- CLI tools for workflow lifecycle
- VS Code extension
- Migration tools (Temporal, Airflow)
- Production deployment guides

### Beyond MVP
See [Post-MVP Roadmap](docs/post-mvp.md) for external service integrations, multi-tenancy, advanced features, and enterprise operations.

**Key Benefits of Revised Sequencing**:
- ✅ Performance validation 4-5 days earlier
- ✅ Epic 2 insights inform remaining Epic 1A/1C implementation decisions
- ✅ Reduced risk: Validate architecture before investing in advanced features
- ✅ Total MVP timeline unchanged, just reordered for better outcomes

## Performance Targets

- **Throughput**: >1,000 workflows/sec (>10,000 activities/sec)
- **Latency**: <10ms P99 workflow start, <1ms orchestrator evaluation
- **Footprint**: <50MB base RAM, 10-15MB binary size

## License

MIT
