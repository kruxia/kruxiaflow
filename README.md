# StreamFlow - Lightweight Workflow Orchestration

StreamFlow is a high-performance workflow orchestration engine designed for edge-to-cloud deployment. Built as a **single Rust binary** with PostgreSQL as the only required dependency.

## Why StreamFlow?

- **🚀 High Performance**: 17-123 workflows/sec (avg 56 wf/sec) - **1.6x faster than Temporal** (avg 35 wf/sec), **44x faster than Airflow**
- **📦 Minimal Footprint**: <15MB binary, <50MB RAM (vs Temporal's multi-GB deployment)
- **⚡ Edge-Ready**: Runs on Raspberry Pi Zero for edge AI and IoT workflows
- **🔧 PostgreSQL-Only**: No Kafka, Cassandra, or Elasticsearch required for MVP
- **🎯 AI-Native**: Built-in LLM cost tracking, budget enforcement, and semantic caching (50-80% cost savings)
- **🔌 Pluggable Architecture**: Swap PostgreSQL for Kafka, Redis, S3, etc. post-MVP

## Status

**Current Version**: 0.3.0 MVP Complete
**Last Updated**: November 27, 2025
**Branch**: `epic-3-mvp-activities-examples`

**Implementation Phase**: ✅ **MVP COMPLETE** - All Examples (1-10) and Core Activities Implemented
- **Epic 1**: ✅ Complete (Event-Driven Orchestration)
- **Epic 1A**: ✅ Complete (API Server - 8/9 stories, including US-1A.9a WebSocket)
- **Epic 1B**: ✅ Complete (Built-in Worker)
- **Epic 1C**: ⏳ Partial (3/7 stories - serve command, graceful shutdown)
- **Epic 2**: ✅ Complete (Performance Benchmarking - 1.6x faster than Temporal)
- **Epic 3**: ✅ Complete (YAML Workflows - All 10 Examples complete)
- **Epic 5**: ✅ Complete (Built-In Activities - All core activities implemented)
- **Epic 7**: ✅ Complete (Token Streaming via WebSocket)

**MVP Status**: Orchestrator and built-in worker with all core activities are feature-complete

📊 **Detailed Status**: See [PROJECT-STATUS.md](docs/PROJECT-STATUS.md) for comprehensive progress tracking

### Completed Features ✅

**Epic 1: Event-Driven Orchestration Architecture** (Complete)
- ✅ **[US-1.1: Activity Queue](docs/implementation/US-1.1-activity-queue.md)** - PostgreSQL-based queue with safe concurrency
- ✅ **[US-1.2: Event-Driven Dynamic Scheduling](docs/implementation/US-1.2-event-driven-scheduling.md)** - Reactive orchestrator with <1ms evaluation

**Epic 1A: API Server** (Complete - 8 of 9 stories, 1 deferred to Post-MVP)
- ✅ **[US-1A.1: Health Check and Service Discovery](docs/implementation/US-1A.1-health-checks.md)** - Liveness/readiness probes with parallel health checks
- ✅ **[US-1A.1.5: API Server CLI Launcher](docs/implementation/US-1A.1.5-api-server-launcher.md)** - `streamflow api` command with configuration management
- ✅ **[US-1A.2: Error Handling and API Contracts](docs/implementation/US-1A.2-error-handling.md)** - Standard error responses, OpenAPI/ReDoc docs, CORS, request tracing
- ✅ **[US-1A.3: Authentication](docs/implementation/US-1A.3-authentication.md)** - OAuth 2.0 JWT authentication with RSA256 signing, refresh token rotation
- ✅ **[US-1A.4: Workflow Definition Management](docs/implementation/US-1A.4-workflow-definition-management.md)** - Deploy, version, and manage workflow definitions
- ✅ **[US-1A.5: Workflow Submission API](docs/implementation/US-1A.5-workflow-submission.md)** - Submit workflows with idempotency support
- ✅ **[US-1A.6: Workflow Status Query](docs/implementation/US-1A.6-workflow-status-query.md)** - Query workflow status and activities
- ✅ **[US-1A.7: Worker Activity APIs](docs/implementation/US-1A.7-worker-activity-apis.md)** - Poll, heartbeat, complete, fail endpoints
- ✅ **[US-1A.9a: WebSocket Infrastructure](docs/implementation/US-1A.9a-websocket-infrastructure.md)** - Token streaming WebSocket support

**Epic 1B: Built-in Worker** (Complete)
- ✅ **[US-1B.1: Worker Polling with Concurrency Safety](docs/implementation/US-1B.1-built-in-worker.md)** - Worker implementation using API endpoints
- ✅ JWT authentication and token management
- ✅ Activity execution and result reporting
- ✅ Concurrent worker polling with FOR UPDATE SKIP LOCKED safety

**Epic 1C: Main Binary and CLI** (Complete)
- ✅ **[US-1C.1: Main Binary and CLI Framework](docs/implementation/US-1C.1-main-binary-cli.md)** - Version command, enhanced help, 4.5MB binary
- ✅ **[US-1C.2: All-in-One Service Launcher](docs/implementation/US-1C.2-all-in-one-launcher.md)** - `streamflow serve` command with graceful shutdown
- ✅ **[US-1C.7: Graceful Shutdown and Signal Handling](docs/implementation/US-1C.7-graceful-shutdown.md)** - SIGTERM/SIGINT handling with CancellationToken

**Epic 2: Performance Benchmarking** (Complete)
- ✅ **[US-2.1: Automated Performance Test Suite](docs/implementation/US-2.1-automated-performance-test-suite.md)** - Continuous benchmarking with regression detection
- ✅ **[US-2.2: Competitor Comparison Benchmarks](docs/implementation/US-2.2-competitor-comparison-benchmarks.md)** - Reproducible benchmarks vs Temporal/Airflow
  - **Results**: StreamFlow 56 wf/sec avg | Temporal 35 wf/sec | Airflow 1.3 wf/sec
  - **Speedup**: 1.6x faster than Temporal, 44x faster than Airflow

**Epic 3: YAML Workflow Definition Language** (Complete - All 10 Examples Done)
- ✅ **[US-3.5: Activity Settings](docs/implementation/US-3.5-activity-settings.md)** - Retry policies, timeout, budget tracking
- ✅ **[US-3.4: Iterative Workflows](docs/implementation/US-3.4-iterative-workflows.md)** - Loop support with counters
- ✅ **[US-3.7: Activity Scheduling](docs/implementation/US-3.7-activity-scheduling.md)** - Delay and scheduled_for with template support
- ✅ **Example 1: Sequential Workflow** - HTTP activity, template expressions, YAML parser
  - Sequential workflows with `depends_on`
  - Template expressions: `{{INPUT.*}}`, `{{activity.output}}`, `{{SECRET.*}}`
  - HTTP activity with custom headers and query parameters
  - Example workflows: `01-weather-report.yaml`, `01b-weather-report-dynamic.yaml`
- ✅ **Example 2: Conditional Branching** - PostgreSQL activity, MiniJinja conditionals
  - Conditional execution with MiniJinja template engine
  - PostgreSQL activity with parameterized queries
  - Flexible condition syntax (single or array)
  - Example workflow: `02-user-validation.yaml`
- ✅ **Example 3: Parallel Execution with File Management** - Fan-out/fan-in, file storage
  - Parallel activity execution (multiple activities ready simultaneously)
  - Fan-in synchronization (wait for all dependencies)
  - PostgreSQL Large Objects for file storage
  - HTTP file download (GET) and upload (POST multipart/form-data)
  - Example workflow: `03-document-processing.yaml`
- ✅ **Example 4: LLM Activity with Cost Tracking and Retry** - AI-native workflow features
  - LLM activity with Anthropic Claude integration
  - Budget enforcement with cost tracking (tokens and USD)
  - Retry logic with exponential backoff
  - Multi-provider support (Anthropic, OpenAI, Google, Ollama)
  - Budget-aware fallback chains
  - Semantic caching for activity results (50-80% cost savings)
  - Example workflow: `04-moderate-content.yaml`
- ✅ **Example 5: Multi-Model LLM Fallback** - Budget-aware provider selection
  - Multi-provider LLM with automatic fallback chains
  - Budget-aware model selection (skip expensive models when constrained)
  - Cost optimization across providers
  - Provider-specific examples: `05-research-assistant.yaml`, `05a-anthropic.yaml`, `05b-openai.yaml`, `05c-google.yaml`
- ✅ **Example 6: Semantic Caching and RAG** - Intelligent caching patterns
  - Semantic caching for LLM responses (`06a-faq-bot-caching.yaml`)
  - RAG index building (`06b-rag-index-builder.yaml`)
  - RAG query patterns (`06c-rag-query.yaml`)
- ✅ **Example 7: Agentic Research / Iterative Workflows** - Loop support
  - Simple iterative loops (`07a-agentic-research-simple.yaml`)
  - Complete iterative workflows (`07b-agentic-research-complete.yaml`)
- ✅ **Example 8: Activity Scheduling and Delays** - Temporal control
  - Rate limiting with delays (`08a-rate-limited-api-calls.yaml`)
  - Absolute scheduling with scheduled_for (`08b-scheduled-daily-report.yaml`)
  - Cascading delays (`08c-delayed-reminders.yaml`)
- ✅ **Example 9: Token Streaming** - Real-time LLM streaming
  - Basic LLM streaming (`09a-streaming-llm.yaml`)
  - Selective streaming in multi-step workflows (`09b-streaming-research.yaml`)
- ✅ **Example 10: Order Processing with Email** - E-commerce workflow
  - HTTP requests with auth headers and timeouts
  - Database transactions with RETURNING clause
  - Email notifications with HTML content
  - Example workflow: `10-order-processing.yaml`

**Epic 7: AI-Native Features** (Token Streaming Complete)
- ✅ **[US-7.1: Token Streaming](docs/implementation/US-7.1-token-streaming.md)** - Real-time LLM token streaming via WebSocket

**Epic 5: Built-In Activity Library** (Complete - All Core Activities)
- ✅ **[US-5.1: Multi-Provider LLM Activities](docs/implementation/US-5.1-multi-provider-llm.md)** - Phases 1-5 complete
  - Anthropic (Claude 3.5 Sonnet, Haiku)
  - OpenAI (GPT-4, GPT-3.5)
  - Google (Gemini Pro, Gemini Flash)
  - Ollama (self-hosted models)
  - Database-backed model catalog with pricing
  - Budget enforcement at workflow and activity level
  - Automatic fallback chains with cost optimization
- ✅ **[US-5.2: AI Cost Tracking and Budget Enforcement](docs/implementation/US-5.1-multi-provider-llm.md)** - Merged into US-5.1
  - Per-activity and per-workflow budget limits
  - Real-time cost tracking in PostgreSQL
  - Token counting and cost calculation
  - Budget exceeded actions (abort/alert)
- ✅ **[US-5.3: Semantic Caching](docs/implementation/US-5.3-semantic-caching.md)** - 100% production ready
  - Redis-based caching with TTL
  - Universal caching for all activity types
  - SHA256-based deterministic cache keys
  - Cache invalidation API
  - 50-80% cost savings for repeated queries
- ✅ **[US-5.4: Object Storage and File Management](docs/implementation/US-5.4-object-storage.md)** - MVP complete
  - PostgreSQL Large Objects backend
  - File production/consumption via {{FILE.activity.filename}}
  - Automatic lifecycle management
  - Cross-cutting capability for all activities
- ✅ **US-5.5**: HTTP/REST Operations - http_request with auth, timeouts, retries
- ✅ **[US-5.6: Database Operations](docs/implementation/US-5.6-database-operations.md)** - postgres_query and postgres_transaction complete
- ✅ **[US-5.7a: Email Send](docs/implementation/US-5.7a-email-send.md)** - SMTP email with HTML/text support

### Current Focus 🎯

**MVP Complete - Post-MVP Planning**
- All 10 example workflows implemented and tested
- All core built-in activities complete (HTTP, PostgreSQL, LLM, Email)
- Token streaming and semantic caching operational
- See [MVP Workflows Implementation Plan](docs/implementation/mvp-workflows-implementation-plan.md) for details
- See [Post-MVP Roadmap](docs/post-mvp.md) for next phase features

### Recent Completions ✅

**Week of Nov 27, 2025** - MVP Complete ✅
- ✅ **US-5.7a**: Email Send Activity
  - SMTP email with HTML and plain text support
  - TLS modes (None, StartTls, ImplicitTls)
  - Template support for dynamic email content
- ✅ **US-5.6**: Database Operations
  - `postgres_query` for single SQL queries with parameterized binding
  - `postgres_transaction` for multi-statement atomic transactions
  - Connection pooling with shared cache
- ✅ **Example 10**: Order Processing with Email
  - Complete e-commerce workflow demonstrating HTTP, database, and email
  - End-to-end test with mock HTTP endpoints and Mailhog SMTP

**Week of Nov 25-26, 2025** - Token Streaming and WebSocket Infrastructure Complete ✅
- ✅ **US-7.1**: Token Streaming for Real-Time UX
  - WebSocket-based token streaming from LLM activities
  - Real-time token-by-token delivery for ChatGPT-style UX
  - Integration with multi-provider LLM activities
- ✅ **US-1A.9a**: WebSocket Infrastructure
  - WebSocket endpoint for activity streaming
  - Authentication via Bearer token
  - Connection management for concurrent streams
- ✅ **Example 9**: Token Streaming Workflows
  - Basic LLM streaming (`09a-streaming-llm.yaml`)
  - Selective streaming in multi-step workflows (`09b-streaming-research.yaml`)
  - **Test Coverage**: 85% (target >90%)

**Week of Nov 20-24, 2025** - Examples 6-8 Complete ✅
- ✅ **Example 6**: Semantic Caching and RAG Patterns
  - FAQ bot with semantic caching (`06a-faq-bot-caching.yaml`)
  - RAG index builder (`06b-rag-index-builder.yaml`)
  - RAG query workflow (`06c-rag-query.yaml`)
- ✅ **Example 7**: Agentic Research / Iterative Workflows (US-3.4)
  - Simple iterative loops (`07a-agentic-research-simple.yaml`)
  - Complete iterative workflows (`07b-agentic-research-complete.yaml`)
  - Loop counter support with max iterations
- ✅ **Example 8**: Activity Scheduling and Delays (US-3.7)
  - Rate limiting with relative delays (`08a-rate-limited-api-calls.yaml`)
  - Absolute scheduling with scheduled_for (`08b-scheduled-daily-report.yaml`)
  - Cascading delays pattern (`08c-delayed-reminders.yaml`)

**Week of Nov 18-19, 2025** - Epic 3 Examples 3-5 + Epic 5 Stories Complete ✅
- ✅ **Example 3**: Parallel Execution with File Management
  - Parallel activity execution (fan-out pattern)
  - Fan-in synchronization (wait for all dependencies before proceeding)
  - PostgreSQL Large Objects for file storage (WorkflowStorage interface)
  - HTTP file download (GET) and upload (POST multipart/form-data)
  - Example workflow: `03-document-processing.yaml` (8-activity pipeline)
  - End-to-end tests with mock HTTP server
- ✅ **Example 4**: LLM Activity with Cost Tracking and Retry
  - Activity settings model (retry, timeout, budget) fully implemented
  - Retry logic with exponential backoff in orchestrator
  - Budget tracking service with pre-execution checks
  - LLM activity with Anthropic Claude integration
  - Example workflow: `04-moderate-content.yaml`
- ✅ **Example 5**: Multi-Model LLM with Automatic Fallback (5 variants)
  - Multi-provider support (Anthropic, OpenAI, Google, Ollama)
  - Budget-aware fallback chains (skip expensive models when budget constrained)
  - Provider-specific variants for testing each provider
  - Example workflows: `05-research-assistant.yaml`, `05a-anthropic.yaml`, `05b-openai.yaml`, `05c-google.yaml`
- ✅ **US-5.1**: Multi-Provider LLM Activities (Phases 1-5)
- ✅ **US-5.3**: Semantic Caching (100% production ready)
- ✅ **US-5.4**: Object Storage and File Management

**Week of Nov 11-16, 2025** - Epic 3 Examples 1-2 Complete ✅
- ✅ **Example 1**: Sequential Workflow
- ✅ **Example 2**: Conditional Branching

**Week of Nov 4-11, 2025** - Epic 1 Complete ✅
- ✅ All Epic 1A, 1B, 1C user stories complete
- ✅ API Server with full workflow and worker APIs
- ✅ Built-in Worker using HTTP client
- ✅ Main Binary with `streamflow serve` command
- ✅ Graceful shutdown with SIGTERM/SIGINT handling

### Deferred Features 📋

**Post-MVP**:
- 📋 **US-1A.8**: Activity Results and Output Retrieval (~8 hours)
- 📋 **US-1A.9b**: WebSocket Streaming for Workflow Events (~10 hours)
- 📋 **US-1C.3**: Individual Service Launchers (~5 hours)
- 📋 **US-1C.4**: Configuration Management (~4 hours)
- 📋 **US-1C.5**: Database Migration CLI (~3 hours)
- 📋 **US-1C.6**: Health Checks and Service Monitoring (~5 hours)

**Epic 2: Performance Benchmarking** (Additional stories post-MVP):
- ✅ **US-2.1**: Automated Performance Test Suite (Complete)
- ✅ **US-2.2**: Competitor Comparison Benchmarks (Complete)
- 📋 **US-2.3**: PostgreSQL Performance Profiling
- 📋 **US-2.4**: Stress Testing and Capacity Planning
- 📋 **US-2.5**: Grafana Performance Dashboard

## Quick Start

### Prerequisites

- Rust 1.90.0+
- Docker (for PostgreSQL 18+)
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
   ./scripts/test.sh
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
- **API Server**: Complete HTTP/REST API for workflow and worker operations
- **Built-in Worker**: Activity execution with HTTP client using API endpoints
- **Main Binary**: Unified CLI with `api` and `version` commands (4.5MB)

### In Progress Components (🔨)
- **All-in-One Launcher**: `streamflow serve` to launch all services together
- **WorkflowStorage**: Handles large artifacts and files (planned for Epic 5)

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
./scripts/test.sh
```

This script will:
1. Ensure PostgreSQL is running
2. Drop and recreate the `streamflow_test` database
3. Run migrations
4. Execute all tests with proper isolation

**Test Coverage:**
```bash
# Run tests with coverage
./scripts/test.sh --coverage

# Generate HTML coverage report
./scripts/test.sh --coverage-html

# Test specific crate
./scripts/test.sh -p streamflow-api --coverage
```

**More options:**
```bash
./scripts/test.sh --help  # See all options
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

# Caching (Optional - Redis required)
STREAMFLOW_CACHE_PROVIDER=redis
STREAMFLOW_REDIS_URL=redis://localhost:6379
STREAMFLOW_REDIS_KEY_PREFIX=streamflow:cache:
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
  - **Epic 1: Event-Driven Orchestration** ✅
    - [US-1.1: Activity Queue](docs/implementation/US-1.1-activity-queue.md)
    - [US-1.2: Event-Driven Scheduling](docs/implementation/US-1.2-event-driven-scheduling.md)
  - **Epic 1A: API Server** ✅ (8 of 9 complete)
    - [US-1A.1: Health Check and Service Discovery](docs/implementation/US-1A.1-health-checks.md)
    - [US-1A.1.5: API Server CLI Launcher](docs/implementation/US-1A.1.5-api-server-launcher.md)
    - [US-1A.2: Error Handling and API Contracts](docs/implementation/US-1A.2-error-handling.md)
    - [US-1A.3: Authentication](docs/implementation/US-1A.3-authentication.md)
    - [US-1A.4: Workflow Definition Management](docs/implementation/US-1A.4-workflow-definition-management.md)
    - [US-1A.5: Workflow Submission API](docs/implementation/US-1A.5-workflow-submission.md)
    - [US-1A.6: Workflow Status Query](docs/implementation/US-1A.6-workflow-status-query.md)
    - [US-1A.7: Worker Activity APIs](docs/implementation/US-1A.7-worker-activity-apis.md)
    - [US-1A.9a: WebSocket Infrastructure](docs/implementation/US-1A.9a-websocket-infrastructure.md)
  - **Epic 7: AI-Native Features** ✅
    - [US-7.1: Token Streaming](docs/implementation/US-7.1-token-streaming.md)
  - **Epic 1B: Built-in Worker** ✅
    - [US-1B.1: Worker Polling with Concurrency Safety](docs/implementation/US-1B.1-built-in-worker.md)
  - **Epic 1C: Main Binary and CLI** (Partial - 3 of 6 complete, Pre-Epic 2 requirements met)
    - [US-1C.1: Main Binary and CLI Framework](docs/implementation/US-1C.1-main-binary-cli.md) ✅
    - [US-1C.2: All-in-One Service Launcher](docs/implementation/US-1C.2-all-in-one-launcher.md) ✅
    - [US-1C.7: Graceful Shutdown and Signal Handling](docs/implementation/US-1C.7-graceful-shutdown.md) ✅
  - **Epic 2: Performance Benchmarking** (Partial - 1 of 5 complete)
    - [US-2.2: Competitor Comparison Benchmarks](docs/implementation/US-2.2-competitor-comparison-benchmarks.md) ✅

### Feature Documentation
- **[Semantic Caching](docs/features/semantic-caching.md)** - Automatic result caching for cost savings and performance

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

### Phase 2C: Pre-Epic 2 Requirements (Weeks 8-9) ✅ **COMPLETE**
**Minimal viable system for performance benchmarking:**
- ✅ Workflow Status and Query API (US-1A.6) - 11 hours
- ✅ Worker Activity APIs (US-1A.7) - 12 hours
- ✅ Main Binary and CLI Framework (US-1C.1) - 6 hours
- ✅ All-in-One Service Launcher (US-1C.2) - 8 hours
- ✅ Graceful Shutdown (US-1C.7) - 4 hours

**Total Effort: ~41 hours (5 days)** - Epic 1 Complete!

### Phase 3: YAML Workflows - Examples 1-2 (Weeks 10-11) ✅ **COMPLETE**
**Epic 3: YAML Workflow Definition Language (Example-Driven)**
- ✅ Example 1: Sequential workflow with HTTP activity
  - YAML parser and workflow definition
  - Template expression engine ({{INPUT.*}}, {{activity.output}}, {{SECRET.*}})
  - HTTP activity executor with custom headers
  - Example workflows: weather-report.yaml, weather-report-dynamic.yaml
- ✅ Example 2: Conditional branching with PostgreSQL
  - MiniJinja conditional evaluation
  - PostgreSQL activity executor
  - depends_on alias and flexible condition syntax
  - Example workflow: user-validation.yaml

### Phase 4: YAML Workflows - Examples 3-10 (Weeks 12-18) ✅ **COMPLETE**
**Epic 3 + Epic 5 (Built-in Activities) + Epic 7 (Token Streaming)**
- ✅ Example 3: Parallel execution with file management
- ✅ Example 4: LLM with cost tracking and retry
- ✅ Example 5: Multi-model LLM fallback
- ✅ Example 6: Semantic caching and RAG
- ✅ Example 7: Iterative workflows/loops
- ✅ Example 8: Scheduled/delayed activities
- ✅ US-1A.9a: WebSocket Infrastructure
- ✅ US-7.1: Token Streaming
- ✅ Example 9: Token streaming workflows
- ✅ Example 10: Order processing with email notification
- ✅ US-5.6: Database operations (postgres_query, postgres_transaction)
- ✅ US-5.7a: Email send activity

See [MVP Workflows Implementation Plan](docs/implementation/mvp-workflows-implementation-plan.md)

### Phase 5: Performance Benchmarking (Post-Epic 3) 📋 **DEFERRED**
**Epic 2: Validate Architecture After YAML Implementation**
- Automated performance test suite (US-2.1)
- ✅ Competitor comparison benchmarks (US-2.2) - Already complete
- PostgreSQL performance profiling (US-2.3)
- Stress testing and capacity planning (US-2.4)
- Performance dashboard and monitoring (US-2.5)
- **Target**: Prove >1,000 workflows/sec

### Phase 6: Complete Epic 1A/1C (Post-Epic 3) 📋 **DEFERRED**
**Features informed by Epic 3 insights:**
- Activity Results and Output Retrieval (US-1A.8) - ~8 hours
- WebSocket Streaming for Real-Time Updates (US-1A.9) - ~15 hours
- Individual Service Launchers (US-1C.3) - ~5 hours
- Configuration Management (US-1C.4) - ~4 hours
- Database Migration CLI (US-1C.5) - ~3 hours
- Health Checks and Service Monitoring (US-1C.6) - ~5 hours

### Phase 7: Programmatic Definition (Post-MVP) 📋 **DEFERRED**
**Epic 4: Python/JavaScript Builder APIs**
- Compilation pipeline (code → YAML)
- 5+ examples per language

### Phase 8: PostgreSQL Optimization (Post-MVP) 📋 **DEFERRED**
**Epic 6: Query optimization based on Epic 2 insights**
- Connection pooling and batching
- Advanced indexing strategy
- Partitioning for time-series data
- Target validation: >1,000 workflows/sec sustained

### Phase 9: Developer Experience (Post-MVP) 📋 **DEFERRED**
**Epic 9: Tools and Migration**
- CLI tools for workflow lifecycle
- VS Code extension
- Migration tools (Temporal, Airflow)
- Production deployment guides

### Beyond MVP
See [Post-MVP Roadmap](docs/post-mvp.md) for external service integrations, multi-tenancy, advanced features, and enterprise operations.

**Key Benefits of Example-Driven Approach**:
- ✅ Epic 3 (YAML) and Epic 5 (Activities) implemented together through realistic workflows
- ✅ Each example is a runnable, testable workflow demonstrating new capabilities
- ✅ Incremental complexity: Sequential → Conditional → Parallel → Loops → LLM
- ✅ End-to-end validation at each step ensures production-ready features
- ✅ Examples serve as documentation and learning resources

## Performance

**Current Benchmarks** (v0.2.0 MVP):
- **Throughput**: 17-123 workflows/sec (scenario-dependent, avg 56 wf/sec)
  - Sequential workflows: 17 wf/sec
  - Parallel workflows: 27 wf/sec
  - High-concurrency: 123 wf/sec
- **Latency**: P50: 350-760ms, P99: 430-1000ms (end-to-end workflow completion)
- **vs Competitors**: 1.6x faster than Temporal, 44x faster than Airflow
- **Footprint**: 4.5MB binary, <50MB base RAM

**Post-MVP Targets** (after PostgreSQL optimization - Epic 6):
- **Throughput**: >1,000 workflows/sec sustained
- **Latency**: <10ms P99 workflow start, <1ms orchestrator evaluation
- **Optimization areas**: Connection pooling, query batching, advanced indexing

## License

MIT
