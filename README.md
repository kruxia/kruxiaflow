# Kruxia Flow

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org/)
[![Docker](https://img.shields.io/docker/v/kruxia/kruxiaflow?label=docker&color=2496ED)](https://hub.docker.com/r/kruxia/kruxiaflow)
[![Docker Image Size](https://img.shields.io/docker/image-size/kruxia/kruxiaflow/latest?label=image%20size)](https://hub.docker.com/r/kruxia/kruxiaflow)

**AI-native durable workflows that run everywhere**

A lightweight, high-performance workflow engine designed for AI applications. Track every token, cache intelligently, and never exceed your LLM budget.

```
Single Binary Deployment | 40% Lower Memory | AI Cost Tracking Built-in | Runs Anywhere
```

## Quick Start

### 1. Start Kruxia Flow

```bash
git clone https://github.com/kruxia/kruxia-flow.git
cd kruxia-flow
./docker up -d
./docker logs -f

# Wait for "listening on 0.0.0.0:8080" then verify:
./docker exec kruxiaflow-server /kruxiaflow health
```

That's it. Kruxia Flow is running with PostgreSQL and Redis, ready to execute workflows.

### 2. Get an Access Token

Kruxia Flow always runs with OAuth2 security, so you'll need client authentication to
run workflows. The simplest approach for local running is to use the generated client
credentials to get an access token:

```bash
# Read the generated client secret from .env
CLIENT_SECRET=$(grep KRUXIAFLOW_CLIENT_SECRET .env | cut -d= -f2)

TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/oauth/token \
  -d "grant_type=client_credentials" \
  -d "client_id=kruxiaflow-docker-client" \
  -d "client_secret=$CLIENT_SECRET" | jq -r '.access_token')
```

### 3. Run a Workflow

Deploy the weather report example and run it:

```bash
# Deploy the workflow definition
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/yaml" \
  --data-binary @examples/01-weather-report.yaml | jq .

# Submit a workflow instance
curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "weather_report",
    "input": {"webhook_url": "https://httpbin.org/post"}
  }' | jq .
```

This fetches a weather forecast from the National Weather Service API and POSTs
the result to a webhook. Copy the `workflow_id` from the response to check status:

```bash
curl -s http://localhost:8080/api/v1/workflows/WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN" | jq .
```

### 4. Run an LLM Workflow

For AI workflows, set your provider API key in your shell environment and restart:

```bash
# Set your Anthropic API key (add to ~/.bashrc or ~/.zshrc to persist)
export ANTHROPIC_API_KEY=your-key-here

# Restart the server to pick up the new key
./docker down && ./docker up -d
```

Then deploy and run the content moderation example:

```bash
# Deploy the moderation workflow
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/yaml" \
  --data-binary @examples/04-moderate-content.yaml | jq .

# Submit a moderation request
curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "moderate_content",
    "input": {
      "user_content": "Check out this amazing product!",
      "content_id": "test-001"
    }
  }' | jq .
```

Check workflow result and cost tracking with the `workflow_id` from the response:

```bash
curl -s http://localhost:8080/api/v1/workflows/WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN" | jq .

# View cost breakdown
curl -s http://localhost:8080/api/v1/workflows/WORKFLOW_ID/cost \
  -H "Authorization: Bearer $TOKEN" | jq .
```

See `examples/` for 15+ workflows covering parallel execution, model fallback, caching,
loops, scheduling, and RAG patterns. API docs at http://localhost:8080/api/v1/docs.

## Why Kruxia Flow?

Kruxia Flow is a **durable execution engine**: workflows survive crashes, retries are automatic, and state is persistent. This puts us in the same category as Temporal and Inngest, not batch schedulers like Airflow.

### vs. Temporal

Temporal is the industry standard for durable execution, and we respect what they've built. Choose Kruxia Flow when you want:

| | Temporal | Kruxia Flow |
|---|----------|-------------|
| **Deployment** | 4+ services (Frontend, History, Matching, Worker) | Single binary |
| **Memory footprint** | 300-400 MB baseline | ~200 MB baseline |
| **Operational complexity** | Requires expertise | Minimal configuration |
| **AI workflow features** | — | Built-in cost tracking, budgets, model fallback, streaming |

**When to choose Kruxia Flow:** You want durable execution without the operational overhead, need AI-native features, or are a small team shipping fast.

### vs. Inngest

Inngest offers a great developer experience for durable functions. Kruxia Flow differentiates with:

- **Self-hosted first:** Run anywhere, including edge and air-gapped environments
- **AI-native:** Built-in cost tracking, budget enforcement, model fallback, token streaming
- **Resource efficiency:** Lower memory footprint for cost-sensitive deployments

### vs. Airflow

Airflow is a batch scheduler. It’s great for data pipelines on a schedule, but fundamentally different from durable execution:

| | Airflow | Kruxia Flow |
|---|---------|-------------|
| **Model** | DAG scheduling | Durable execution |
| **Failure handling** | Task retry | Workflow survives crashes |
| **State** | External (database) | Built-in persistence |
| **Real-time** | Not designed for it | Sub-second capable |

**Migrating from Airflow?** If you need durability guarantees, exactly-once semantics, or real-time workflows, Kruxia Flow is a natural next step. [**TODO**: See migration guide →]

### The Problem

LLM costs spiral out of control. You're running AI workflows with no visibility into token usage. Existing tools don't help:

- **Airflow/Temporal**: Great for orchestration, but no LLM awareness
- **LangChain/LangGraph**: Great for LLM chains, but no durability or cost tracking
- **DIY**: You're building infrastructure instead of your product

### The Solution

Kruxia Flow combines durable execution with AI-native features:

| Feature                  | Kruxia Flow | Temporal | Airflow | LangChain |
|--------------------------|:----------:|:--------:|:-------:|:---------:|
| Durable execution        | **Yes**    | Yes      | Yes     | No        |
| LLM cost tracking        | **Yes**    | No       | No      | No        |
| Budget enforcement       | **Yes**    | No       | No      | No        |
| Semantic caching         | **Planned**| No       | No      | Partial   |
| Multi-provider LLM       | **Yes**    | No       | No      | Yes       |
| Token streaming          | **Yes**    | No       | No      | Yes       |
| Single binary            | **7.5MB**  | ~200MB   | ~500MB+ | N/A       |
| Docker image             | **63MB**   | ~500MB   | ~1GB+   | N/A       |
| Peak memory              | **380MB**  | ~380MB   | ~7.6GB  | N/A       |
| Throughput (wf/sec)      | **32**     | 27       | 1.3     | N/A       |

## Key Features

### Built-in LLM Cost Tracking

Every token is tracked. Every dollar is accounted for.

```yaml
activities:
  - key: analyze
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: "Analyze this document..."
      max_tokens: 500
    settings:
      budget:
        limit_usd: 0.50
        action: abort
```

Real-time cost visibility per workflow, per activity, per model.

### Budget-Aware Model Fallback

Automatically fall back to cheaper models when budget is constrained:

```yaml
activities:
  - key: generate
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-sonnet-4-5-20250929  # Try first
        - openai/gpt-4o-mini                    # If budget constrained
        - anthropic/claude-haiku-4-20250415      # Last resort
      prompt: "Generate a summary..."
      max_tokens: 500
    settings:
      budget:
        limit_usd: 0.10
        action: abort
```

### Result Caching

Save on LLM costs by caching repeated queries:

```yaml
activities:
  - key: answer
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-haiku-4-20250415
      prompt: "{{INPUT.question}}"
      max_tokens: 200
    settings:
      cache:
        enabled: true
        ttl_seconds: 3600
        key:
          - llm_prompt
          - "{{parameters.model}}"
          - "{{parameters.prompt}}"
```

Identical queries hit cache instead of the LLM. (NOTE: Semantic caching is planned.)

### Durable Execution

Workflows survive crashes and restart from where they left off. No lost work, no duplicate charges.

### Multi-Provider LLM Support

Native support for all major providers:

- **Anthropic**: Claude 4.5 Sonnet, Claude 4.5 Haiku
- **OpenAI**: GPT-5.1, GPT-4o, GPT-4o-mini, GPT-3.5 Turbo
- **Google**: Gemini Pro, Gemini Flash
- **Ollama**: Self-hosted open models

## Examples

Kruxia Flow includes 10 production-ready example workflows:

| #  | Example                     | Concepts Demonstrated                              |
|----|-----------------------------|----------------------------------------------------|
| 1  | [Weather Report][ex1]       | Sequential workflow, HTTP requests, templates      |
| 2  | [User Validation][ex2]      | Conditional branching, PostgreSQL queries          |
| 3  | [Document Processing][ex3]  | Parallel execution, fan-out/fan-in, file storage   |
| 4  | [Content Moderation][ex4]   | LLM with cost tracking, retry with backoff         |
| 5  | [Research Assistant][ex5]   | Multi-model fallback, budget-aware selection       |
| 6  | [FAQ Bot / RAG][ex6]        | Semantic caching, vector search, embeddings        |
| 7  | [Agentic Research][ex7]     | Iterative loops, agent patterns                    |
| 8  | [Scheduled Tasks][ex8]      | Delays, rate limiting, scheduled execution         |
| 9  | [Token Streaming][ex9]      | Real-time LLM streaming via WebSocket              |
| 10 | [Order Processing][ex10]    | HTTP, database transactions, email notifications   |

[ex1]: examples/01-weather-report.yaml
[ex2]: examples/02-user-validation.yaml
[ex3]: examples/03-document-processing.yaml
[ex4]: examples/04-moderate-content.yaml
[ex5]: examples/05-research-assistant.yaml
[ex6]: examples/06a-faq-bot-caching.yaml
[ex7]: examples/07a-agentic-research-simple.yaml
[ex8]: examples/08a-rate-limited-api-calls.yaml
[ex9]: examples/09a-streaming-llm.yaml
[ex10]: examples/10-order-processing.yaml

## Architecture

Kruxia Flow is a single Rust binary with PostgreSQL as the only required dependency:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Kruxia Flow (7.5MB binary)                   │
├─────────────────────────────────────────────────────────────────┤
│  API Server  │  Orchestrator  │  Worker Pool  │  Cost Tracker   │
└──────────────┴────────────────┴───────────────┴─────────────────┘
                              │
                              ▼
                    ┌───────────────────┐
                    │    PostgreSQL     │
                    │  (events, state,  │
                    │   costs, files)   │
                    └───────────────────┘
```

- **Event-driven**: Publish-subscribe architecture with exactly-once guarantees
- **PostgreSQL-only**: No Kafka, Cassandra, or Elasticsearch required
- **Pluggable**: Swap in Kafka, Redis, S3 when you need scale [POST-MVP]

## Performance

Benchmarked against industry-standard workflow engines (November 2025):

| Metric              | Kruxia Flow | Temporal | Airflow   |
|---------------------|------------|----------|-----------|
| Throughput (wf/sec) | **32**     | 27       | 1.3       |
| P99 Latency         | **0.7-2s** | 0.7-3s   | 9-106s    |
| Peak Memory         | **380MB**  | 380MB    | 7.6GB     |
| Binary Size         | **7.5MB**  | ~200MB   | ~500MB+   |
| Docker Image        | **63MB**   | ~500MB   | ~1GB+     |

*Kruxia Flow: 24x faster than Airflow, 20x less memory*

Benchmark methodology: Identical echo workflows (sequential, parallel, high-concurrency), Docker Compose environment, same hardware. See [`benchmarks/`](benchmarks/) for reproducible tests.

## Documentation

- **[Architecture](docs/architecture.md)** - System design and component overview
- **[MVP Requirements](docs/mvp-requirements.md)** - Product requirements and roadmap
- **[Implementation Plans](docs/implementation/)** - Detailed technical specifications
- **[Post-MVP Roadmap](docs/post-mvp.md)** - Future features and integrations

## Development

### Prerequisites

- Docker and Docker Compose
- (Optional) Rust 1.90+ for local development

### Local Development

```bash
# Start development environment (hot reload)
./docker up --develop

# View logs
./docker logs -f

# Stop services
./docker down
```

### Running Tests

```bash
# Set up test database and run tests
./scripts/setup-dev-db.sh
./scripts/test.sh

# With coverage
./scripts/test.sh --coverage
```

### Manual Setup (without Docker)

```bash
# Install Rust and sqlx-cli
cargo install sqlx-cli --no-default-features --features postgres

# Start PostgreSQL manually
docker run -d --name pg -e POSTGRES_PASSWORD=dev -p 5432:5432 postgres:17

# Run migrations
export DATABASE_URL='postgres://postgres:dev@localhost:5432/kruxia-flow'
sqlx database create
sqlx migrate run

# Build and run
cargo build --release
./target/release/kruxia-flow serve
```

## Roadmap

### Now (MVP Complete)
- Durable workflow execution
- 10 example workflows
- LLM cost tracking and budgets
- Semantic caching
- Multi-provider LLM support
- Token streaming

### Next
- Python SDK (`pip install kruxia-flow`)
- Web dashboard for cost visualization
- Airflow migration guide
- Kubernetes Helm chart

### Later
- TypeScript SDK
- RBAC and multi-tenancy
- Kafka/Redis backends

See [Post-MVP Roadmap](docs/post-mvp.md) for details.

## Community

- **GitHub Issues**: [Report bugs and request features](https://github.com/kruxia/kruxia-flow/issues)
- **Discussions**: [Ask questions and share ideas](https://github.com/kruxia/kruxia-flow/discussions)
- **Code of Conduct**: [Community guidelines](CODE_OF_CONDUCT.md)
- **Security**: [Report vulnerabilities](SECURITY.md)

## Contributing

Contributions are welcome! Please read our [Contributing Guidelines](CONTRIBUTING.md) before submitting PRs.

```bash
# Fork and clone
git clone https://github.com/YOUR_USERNAME/kruxia-flow.git

# Create a branch
git checkout -b feature/your-feature

# Make changes and test
./scripts/test.sh

# Submit a PR
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed development setup and guidelines.

## License

MIT License - See [LICENSE](LICENSE) for details.

---

**Kruxia Flow** - AI-native durable workflow orchestration with built-in cost control.
