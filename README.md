# StreamFlow

**AI-Native durable Workflow Orchestration with Built-in Cost Control**

A lightweight, high-performance workflow engine designed for AI applications. Track every token, cache intelligently, and never exceed your LLM budget.

```
7.5MB binary | 63MB image | PostgreSQL-only | 24x faster than Airflow | 20x less memory
```

## Quick Start

Get StreamFlow running in 60 seconds:

```bash
# Clone and start
git clone https://github.com/kruxia/streamflow.git
cd streamflow
./dev up -d
./dev logs -f

# Wait for services to be healthy (~30 seconds)
docker compose ps

# API is ready at http://localhost:8080
curl http://localhost:8080/health

# API docs:
open http://localhost:8080/api/v1/docs
```

That's it. StreamFlow is running with PostgreSQL and Redis, ready to execute workflows.

## Why StreamFlow?

### The Problem

LLM costs spiral out of control. You're running AI workflows with no visibility into token usage. Existing tools don't help:

- **Airflow/Temporal**: Great for orchestration, but no LLM awareness
- **LangChain/LangGraph**: Great for LLM chains, but no durability or cost tracking
- **DIY**: You're building billing infrastructure instead of your product

### The Solution

StreamFlow combines durable execution with AI-native features:

| Feature                  | StreamFlow | Temporal | Airflow | LangChain |
|--------------------------|:----------:|:--------:|:-------:|:---------:|
| Durable execution        | **Yes**    | Yes      | Yes     | No        |
| LLM cost tracking        | **Yes**    | No       | No      | No        |
| Budget enforcement       | **Yes**    | No       | No      | No        |
| Semantic caching         | **Yes**    | No       | No      | Partial   |
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
  analyze:
    type: llm_prompt
    budget:
      max_cost_usd: 0.50
      exceeded_action: abort
    input:
      model: claude-4-5-sonnet
      prompt: "Analyze this document..."
```

Real-time cost visibility per workflow, per activity, per model.

### Budget-Aware Model Fallback

Automatically fall back to cheaper models when budget is constrained:

```yaml
activities:
  generate:
    type: llm_prompt
    input:
      model: claude-4-5-sonnet  # Try first
      fallback_models:
        - gpt-4o-mini           # If budget constrained
        - claude-4-5-haiku      # Last resort
```

### Semantic Caching

Save 50-80% on LLM costs by caching similar queries:

```yaml
activities:
  answer:
    type: llm_prompt
    cache:
      enabled: true
      similarity_threshold: 0.92
      ttl: 24h
```

Repeated or similar questions hit cache instead of the LLM.

### Durable Execution

Workflows survive crashes and restart from where they left off. No lost work, no duplicate charges.

### Multi-Provider LLM Support

Native support for all major providers:

- **Anthropic**: Claude 4.5 Sonnet, Claude 4.5 Haiku
- **OpenAI**: GPT-5.1, GPT-4o, GPT-4o-mini, GPT-3.5 Turbo
- **Google**: Gemini Pro, Gemini Flash
- **Ollama**: Self-hosted open models

## Examples

StreamFlow includes 10 production-ready example workflows:

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

StreamFlow is a single Rust binary with PostgreSQL as the only required dependency:

```
┌─────────────────────────────────────────────────────────────────┐
│                     StreamFlow (7.5MB binary)                   │
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

- **Event-driven**: No polling loops, reactive scheduling
- **PostgreSQL-only**: No Kafka, Cassandra, or Elasticsearch required
- **Pluggable**: Swap in Kafka, Redis, S3 when you need scale [POST-MVP]

## Performance

Benchmarked against industry-standard workflow engines (November 2025):

| Metric              | StreamFlow | Temporal | Airflow   |
|---------------------|------------|----------|-----------|
| Throughput (wf/sec) | **32**     | 27       | 1.3       |
| P99 Latency         | **0.7-2s** | 0.7-3s   | 9-106s    |
| Peak Memory         | **380MB**  | 380MB    | 7.6GB     |
| Binary Size         | **7.5MB**  | ~200MB   | ~500MB+   |
| Docker Image        | **63MB**   | ~500MB   | ~1GB+     |

*StreamFlow: 24x faster than Airflow, 20x less memory*

Benchmark methodology: Identical echo workflows (sequential, parallel, high-concurrency), Docker Compose environment, same hardware. See `benchmarks/` for reproducible tests.

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
./dev up

# View logs
./dev logs -f

# Stop services
./dev down
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
docker run -d --name pg -e POSTGRES_PASSWORD=dev -p 5432:5432 postgres:18

# Run migrations
export DATABASE_URL='postgres://postgres:dev@localhost:5432/streamflow'
sqlx database create
sqlx migrate run

# Build and run
cargo build --release
./target/release/streamflow serve
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
- Python SDK (`pip install streamflow`)
- Web dashboard for cost visualization
- Airflow migration guide
- Kubernetes Helm chart

### Later
- TypeScript SDK
- RBAC and multi-tenancy
- Kafka/Redis backends

See [Post-MVP Roadmap](docs/post-mvp.md) for details.

## Community

- **GitHub Issues**: [Report bugs and request features](https://github.com/kruxia/streamflow/issues)
- **Discussions**: [Ask questions and share ideas](https://github.com/kruxia/streamflow/discussions)
- **Code of Conduct**: [Community guidelines](CODE_OF_CONDUCT.md)
- **Security**: [Report vulnerabilities](SECURITY.md)

## Contributing

Contributions are welcome! Please read our [Contributing Guidelines](CONTRIBUTING.md) before submitting PRs.

```bash
# Fork and clone
git clone https://github.com/YOUR_USERNAME/streamflow.git

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

**StreamFlow** - AI-native durable workflow orchestration with built-in cost control.
